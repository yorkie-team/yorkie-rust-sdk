//! Connect protocol transport for Yorkie RPCs: unary calls and the
//! server-streaming `Watch` call, for documents and channels.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (Connect client setup,
//! `x-api-key`, `x-shard-key` and `x-yorkie-user-agent` headers). Streaming
//! follows the Connect protocol's enveloped messages: a flag byte, a 4-byte
//! big-endian length, and the payload; flag `0x02` marks the end-stream
//! message, a JSON object with an optional `error`.

use crate::error::{ClientError, ClientResult};
use crate::options::ClientOptions;
use crate::protocol as conv;
use crate::transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    BroadcastRequest, BroadcastResponse, ChannelWatchStream, ClientTransport,
    DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
    DetachDocumentResponse, PeekChannelRequest, PeekChannelResponse, PushPullChangesRequest,
    PushPullChangesResponse, RefreshChannelRequest, RefreshChannelResponse, RemoveDocumentRequest,
    RemoveDocumentResponse, WatchChannelRequest, WatchDocumentRequest, WatchStream,
};
use async_trait::async_trait;
use futures_util::{stream, Stream, StreamExt};
use prost::Message;
use yorkie_protocol::yorkie::v1 as api;

const SERVICE_PATH: &str = "yorkie.v1.YorkieService";

/// Envelope flag of a compressed message. This client never asks for
/// compression, so a compressed envelope is a protocol error.
const FLAG_COMPRESSED: u8 = 0x01;
/// Envelope flag of the end-stream message.
const FLAG_END_STREAM: u8 = 0x02;
const ENVELOPE_HEADER_LEN: usize = 5;

/// Unary RPC transport speaking the Connect protocol over HTTP.
#[derive(Debug, Clone)]
pub struct ConnectTransport {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    user_agent: String,
}

impl ConnectTransport {
    /// Builds a transport for the server at `options.rpc_addr`.
    pub fn new(options: &ClientOptions) -> ClientResult<Self> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|err| ClientError::Transport(err.to_string()))?;
        let user_agent = options
            .user_agent
            .clone()
            .unwrap_or_else(|| format!("yorkie-rust-sdk/{}", env!("CARGO_PKG_VERSION")));
        Ok(Self {
            http,
            base_url: options.rpc_addr.trim_end_matches('/').to_owned(),
            api_key: options.api_key.clone(),
            user_agent,
        })
    }

    fn post(&self, method: &str, shard_key: &str, content_type: &str) -> reqwest::RequestBuilder {
        let builder = self
            .http
            .post(format!("{}/{SERVICE_PATH}/{method}", self.base_url))
            .header("content-type", content_type)
            .header("connect-protocol-version", "1")
            .header("x-shard-key", shard_key)
            .header("x-yorkie-user-agent", &self.user_agent);
        if self.api_key.is_empty() {
            builder
        } else {
            builder.header("x-api-key", &self.api_key)
        }
    }

    async fn unary<Req, Res>(
        &self,
        method: &str,
        shard_key: &str,
        request: &Req,
    ) -> ClientResult<Res>
    where
        Req: Message,
        Res: Message + Default,
    {
        let response = self
            .post(method, shard_key, "application/proto")
            .body(request.encode_to_vec())
            .send()
            .await
            .map_err(|err| ClientError::Transport(err.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|err| ClientError::Transport(err.to_string()))?;
        if status != 200 {
            return Err(decode_connect_error(status, &body));
        }
        Res::decode(body.as_ref()).map_err(|err| ClientError::Transport(err.to_string()))
    }

    /// Calls a server-streaming RPC. The request is sent as one enveloped
    /// message; the returned stream yields each response message and ends
    /// at the end-stream message, or yields its error.
    async fn server_stream<Req, Res>(
        &self,
        method: &str,
        shard_key: &str,
        request: &Req,
    ) -> ClientResult<impl Stream<Item = ClientResult<Res>> + Send + 'static>
    where
        Req: Message,
        Res: Message + Default + 'static,
    {
        let response = self
            .post(method, shard_key, "application/connect+proto")
            .body(encode_envelope(0, &request.encode_to_vec()))
            .send()
            .await
            .map_err(|err| ClientError::Transport(err.to_string()))?;
        let status = response.status().as_u16();
        if status != 200 {
            let body = response
                .bytes()
                .await
                .map_err(|err| ClientError::Transport(err.to_string()))?;
            return Err(decode_connect_error(status, &body));
        }

        let body = response.bytes_stream();
        Ok(stream::unfold(
            Some((body, EnvelopeDecoder::default())),
            |state| async move {
                let (mut body, mut decoder) = state?;
                loop {
                    match decoder.next_envelope() {
                        Ok(Some(Envelope::Message(payload))) => {
                            let message = Res::decode(payload.as_slice())
                                .map_err(|err| ClientError::Transport(err.to_string()));
                            return Some((message, Some((body, decoder))));
                        }
                        Ok(Some(Envelope::EndStream(payload))) => {
                            return decode_end_stream(&payload)
                                .err()
                                .map(|err| (Err(err), None));
                        }
                        Ok(None) => {}
                        Err(err) => return Some((Err(err), None)),
                    }
                    match body.next().await {
                        Some(Ok(chunk)) => decoder.push(&chunk),
                        Some(Err(err)) => {
                            return Some((Err(ClientError::Transport(err.to_string())), None));
                        }
                        None => {
                            let err = ClientError::Transport(
                                "stream ended without an end-stream message".to_owned(),
                            );
                            return Some((Err(err), None));
                        }
                    }
                }
            },
        ))
    }
}

/// Wraps a message in a Connect envelope.
pub(crate) fn encode_envelope(flags: u8, payload: &[u8]) -> Vec<u8> {
    let len = u32::try_from(payload.len()).expect("Connect messages are below 4 GiB");
    let mut envelope = Vec::with_capacity(ENVELOPE_HEADER_LEN + payload.len());
    envelope.push(flags);
    envelope.extend_from_slice(&len.to_be_bytes());
    envelope.extend_from_slice(payload);
    envelope
}

/// A decoded Connect envelope.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Envelope {
    Message(Vec<u8>),
    EndStream(Vec<u8>),
}

/// Splits a byte stream into Connect envelopes, buffering partial frames.
#[derive(Debug, Default)]
pub(crate) struct EnvelopeDecoder {
    buffer: Vec<u8>,
}

impl EnvelopeDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Returns the next complete envelope, or `None` until more bytes arrive.
    pub(crate) fn next_envelope(&mut self) -> ClientResult<Option<Envelope>> {
        let Some(header) = self.buffer.get(..ENVELOPE_HEADER_LEN) else {
            return Ok(None);
        };
        let flags = header[0];
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if self.buffer.len() < ENVELOPE_HEADER_LEN + len {
            return Ok(None);
        }
        if flags & FLAG_COMPRESSED != 0 {
            return Err(ClientError::Transport(
                "received a compressed Connect message without asking for one".to_owned(),
            ));
        }

        let payload = self.buffer[ENVELOPE_HEADER_LEN..ENVELOPE_HEADER_LEN + len].to_vec();
        self.buffer.drain(..ENVELOPE_HEADER_LEN + len);
        Ok(Some(if flags & FLAG_END_STREAM != 0 {
            Envelope::EndStream(payload)
        } else {
            Envelope::Message(payload)
        }))
    }
}

/// Decodes the JSON end-stream message: `Ok` when it carries no error, else
/// its error as [`ClientError::Rpc`].
pub(crate) fn decode_end_stream(payload: &[u8]) -> ClientResult<()> {
    if payload.is_empty() {
        return Ok(());
    }
    let value = serde_json::from_slice::<serde_json::Value>(payload)
        .map_err(|err| ClientError::Transport(format!("invalid end-stream message: {err}")))?;
    let Some(error) = value.get("error").filter(|error| !error.is_null()) else {
        return Ok(());
    };
    let text = |field: &str| error.get(field).and_then(|value| value.as_str());
    Err(ClientError::Rpc {
        code: text("code").unwrap_or("unknown").to_owned(),
        message: text("message").unwrap_or_default().to_owned(),
        yorkie_code: yorkie_code_of(error),
    })
}

/// `google.rpc.ErrorInfo`, the error detail Yorkie puts its error code in.
#[derive(Clone, PartialEq, prost::Message)]
struct ErrorInfo {
    #[prost(string, tag = "1")]
    reason: String,
    #[prost(string, tag = "2")]
    domain: String,
    #[prost(map = "string, string", tag = "3")]
    metadata: std::collections::HashMap<String, String>,
}

/// Returns the Yorkie error code from the first `ErrorInfo` detail of a
/// Connect JSON error (JS `errorCodeOf`, which reads only that detail), if it
/// carries one. Connect sends each detail's protobuf bytes base64-encoded,
/// with or without padding.
fn yorkie_code_of(error: &serde_json::Value) -> Option<String> {
    use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
    use base64::engine::DecodePaddingMode;
    use base64::Engine;

    const BASE64: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );

    error
        .get("details")?
        .as_array()?
        .iter()
        .filter(|detail| {
            detail.get("type").and_then(|value| value.as_str()) == Some("google.rpc.ErrorInfo")
        })
        .filter_map(|detail| BASE64.decode(detail.get("value")?.as_str()?).ok())
        .find_map(|bytes| ErrorInfo::decode(bytes.as_slice()).ok())?
        .metadata
        .remove("code")
}

fn protocol_error(err: impl ToString) -> ClientError {
    ClientError::Transport(err.to_string())
}

/// Decodes a non-200 Connect unary reply into [`ClientError::Rpc`].
///
/// Bodies that are not Connect JSON errors fall back to a code derived from
/// the HTTP status, with the body text as the message.
pub(crate) fn decode_connect_error(status: u16, body: &[u8]) -> ClientError {
    #[derive(Default)]
    struct Parsed {
        code: Option<String>,
        message: String,
        yorkie_code: Option<String>,
    }
    let parsed = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let object = value.as_object()?;
            Some(Parsed {
                code: object.get("code")?.as_str().map(str::to_owned),
                message: object
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                yorkie_code: yorkie_code_of(&value),
            })
        })
        .unwrap_or_default();
    match parsed.code {
        Some(code) => ClientError::Rpc {
            code,
            message: parsed.message,
            yorkie_code: parsed.yorkie_code,
        },
        None => ClientError::Rpc {
            code: http_status_to_code(status).to_owned(),
            message: String::from_utf8_lossy(body).into_owned(),
            yorkie_code: None,
        },
    }
}

fn http_status_to_code(status: u16) -> &'static str {
    match status {
        400 => "internal",
        401 => "unauthenticated",
        403 => "permission_denied",
        404 => "unimplemented",
        429 | 502 | 503 | 504 => "unavailable",
        _ => "unknown",
    }
}

#[async_trait]
impl ClientTransport for ConnectTransport {
    async fn activate_client(
        &self,
        request: ActivateClientRequest,
    ) -> ClientResult<ActivateClientResponse> {
        let response: api::ActivateClientResponse = self
            .unary(
                "ActivateClient",
                &request.shard_key,
                &conv::to_activate_client_request(&request),
            )
            .await?;
        Ok(conv::from_activate_client_response(&response))
    }

    async fn deactivate_client(
        &self,
        request: DeactivateClientRequest,
    ) -> ClientResult<DeactivateClientResponse> {
        let response: api::DeactivateClientResponse = self
            .unary(
                "DeactivateClient",
                &request.shard_key,
                &conv::to_deactivate_client_request(&request),
            )
            .await?;
        Ok(conv::from_deactivate_client_response(&response))
    }

    async fn attach_document(
        &self,
        request: AttachDocumentRequest,
    ) -> ClientResult<AttachDocumentResponse> {
        let proto = conv::to_attach_document_request(&request).map_err(protocol_error)?;
        let response: api::AttachDocumentResponse = self
            .unary("AttachDocument", &request.shard_key, &proto)
            .await?;
        conv::from_attach_document_response(&response).map_err(protocol_error)
    }

    async fn detach_document(
        &self,
        request: DetachDocumentRequest,
    ) -> ClientResult<DetachDocumentResponse> {
        let proto = conv::to_detach_document_request(&request).map_err(protocol_error)?;
        let response: api::DetachDocumentResponse = self
            .unary("DetachDocument", &request.shard_key, &proto)
            .await?;
        conv::from_detach_document_response(&response).map_err(protocol_error)
    }

    async fn remove_document(
        &self,
        request: RemoveDocumentRequest,
    ) -> ClientResult<RemoveDocumentResponse> {
        let proto = conv::to_remove_document_request(&request).map_err(protocol_error)?;
        let response: api::RemoveDocumentResponse = self
            .unary("RemoveDocument", &request.shard_key, &proto)
            .await?;
        conv::from_remove_document_response(&response).map_err(protocol_error)
    }

    async fn push_pull_changes(
        &self,
        request: PushPullChangesRequest,
    ) -> ClientResult<PushPullChangesResponse> {
        let proto = conv::to_push_pull_changes_request(&request).map_err(protocol_error)?;
        let response: api::PushPullChangesResponse = self
            .unary("PushPullChanges", &request.shard_key, &proto)
            .await?;
        conv::from_push_pull_changes_response(&response).map_err(protocol_error)
    }

    async fn watch_document(&self, request: WatchDocumentRequest) -> ClientResult<WatchStream> {
        let responses = self
            .server_stream::<_, api::WatchResponse>(
                "Watch",
                &request.shard_key,
                &conv::to_watch_document_request(&request),
            )
            .await?;
        Ok(Box::pin(responses.filter_map(|response| async move {
            match response {
                Ok(response) => conv::from_watch_response(&response).map(Ok),
                Err(err) => Some(Err(err)),
            }
        })))
    }

    async fn refresh_channel(
        &self,
        request: RefreshChannelRequest,
    ) -> ClientResult<RefreshChannelResponse> {
        let response: api::RefreshChannelResponse = self
            .unary(
                "RefreshChannel",
                &request.shard_key,
                &conv::to_refresh_channel_request(&request),
            )
            .await?;
        Ok(conv::from_refresh_channel_response(&response))
    }

    async fn peek_channel(&self, request: PeekChannelRequest) -> ClientResult<PeekChannelResponse> {
        let response: api::PeekChannelResponse = self
            .unary(
                "PeekChannel",
                &request.shard_key,
                &conv::to_peek_channel_request(&request),
            )
            .await?;
        Ok(conv::from_peek_channel_response(&response))
    }

    async fn watch_channel(
        &self,
        request: WatchChannelRequest,
    ) -> ClientResult<ChannelWatchStream> {
        let responses = self
            .server_stream::<_, api::WatchResponse>(
                "Watch",
                &request.shard_key,
                &conv::to_watch_channel_request(&request),
            )
            .await?;
        Ok(Box::pin(responses.filter_map(|response| async move {
            match response {
                Ok(response) => conv::from_watch_channel_response(&response).map(Ok),
                Err(err) => Some(Err(err)),
            }
        })))
    }

    async fn broadcast(&self, request: BroadcastRequest) -> ClientResult<BroadcastResponse> {
        let response: api::BroadcastResponse = self
            .unary(
                "Broadcast",
                &request.shard_key,
                &conv::to_broadcast_request(&request),
            )
            .await?;
        Ok(conv::from_broadcast_response(&response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_error_body_maps_to_rpc_error() {
        let err = decode_connect_error(
            404,
            br#"{"code":"not_found","message":"document not found"}"#,
        );
        assert_eq!(
            ClientError::Rpc {
                code: "not_found".to_owned(),
                message: "document not found".to_owned(),
                yorkie_code: None,
            },
            err
        );
    }

    /// JS `errorCodeOf`: the Yorkie code rides in an `ErrorInfo` detail.
    /// The body is what the server answers a refresh of an unknown session.
    #[test]
    fn connect_error_details_carry_the_yorkie_code() {
        let body = br#"{"code":"not_found","message":"refresh 000000000000000000000000: session not found","details":[{"type":"google.rpc.ErrorInfo","value":"Cglub3RfZm91bmQaGgoEY29kZRISRXJyU2Vzc2lvbk5vdEZvdW5k","debug":{"reason":"not_found","metadata":{"code":"ErrSessionNotFound"}}}]}"#;
        let err = decode_connect_error(404, body);
        assert!(matches!(
            &err,
            ClientError::Rpc { code, yorkie_code: Some(yorkie_code), .. }
                if code == "not_found" && yorkie_code == "ErrSessionNotFound"
        ));
        assert!(err.has_yorkie_code("ErrSessionNotFound"));
        assert!(!err.has_yorkie_code("ErrClientNotFound"));

        // Only the first ErrorInfo counts, even without a code (JS
        // `errorMetadataOf`). "CgFy" is an ErrorInfo with reason "r" only.
        let first_without_code = br#"{"code":"not_found","message":"m","details":[{"type":"google.rpc.ErrorInfo","value":"CgFy"},{"type":"google.rpc.ErrorInfo","value":"Cglub3RfZm91bmQaGgoEY29kZRISRXJyU2Vzc2lvbk5vdEZvdW5k"}]}"#;
        assert!(matches!(
            decode_connect_error(404, first_without_code),
            ClientError::Rpc {
                yorkie_code: None,
                ..
            }
        ));

        let end_stream = br#"{"error":{"code":"not_found","message":"m","details":[{"type":"google.rpc.ErrorInfo","value":"Cglub3RfZm91bmQaGgoEY29kZRISRXJyU2Vzc2lvbk5vdEZvdW5k"}]}}"#;
        assert!(decode_end_stream(end_stream)
            .unwrap_err()
            .has_yorkie_code("ErrSessionNotFound"));
    }

    #[test]
    fn non_json_error_body_maps_http_status() {
        assert_eq!(
            ClientError::Rpc {
                code: "unauthenticated".to_owned(),
                message: "nope".to_owned(),
                yorkie_code: None,
            },
            decode_connect_error(401, b"nope")
        );
        assert!(matches!(
            decode_connect_error(404, b"<html>"),
            ClientError::Rpc { code, .. } if code == "unimplemented"
        ));
    }

    /// Connect envelopes: messages split across chunks are buffered, and an
    /// end-stream error becomes `ClientError::Rpc`.
    #[test]
    fn connect_envelope_roundtrip() {
        let first = api::WatchResponse {
            body: Some(api::watch_response::Body::Heartbeat(api::WatchHeartbeat {})),
        };
        let mut bytes = encode_envelope(0, &first.encode_to_vec());
        assert_eq!(0, bytes[0]);
        assert_eq!(
            (bytes.len() - ENVELOPE_HEADER_LEN) as u32,
            u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]])
        );
        bytes.extend(encode_envelope(
            FLAG_END_STREAM,
            br#"{"error":{"code":"unavailable","message":"server stopped"},"metadata":{}}"#,
        ));

        let mut decoder = EnvelopeDecoder::default();
        let mut envelopes = Vec::new();
        for chunk in bytes.chunks(3) {
            decoder.push(chunk);
            while let Some(envelope) = decoder.next_envelope().unwrap() {
                envelopes.push(envelope);
            }
        }

        assert_eq!(2, envelopes.len());
        let Envelope::Message(payload) = &envelopes[0] else {
            panic!("expected a message, got {:?}", envelopes[0]);
        };
        assert_eq!(
            first,
            api::WatchResponse::decode(payload.as_slice()).unwrap()
        );
        let Envelope::EndStream(payload) = &envelopes[1] else {
            panic!("expected an end-stream message, got {:?}", envelopes[1]);
        };
        assert_eq!(
            Err(ClientError::Rpc {
                code: "unavailable".to_owned(),
                message: "server stopped".to_owned(),
                yorkie_code: None,
            }),
            decode_end_stream(payload)
        );
    }

    #[test]
    fn end_stream_without_error_ends_cleanly() {
        assert_eq!(Ok(()), decode_end_stream(b""));
        assert_eq!(Ok(()), decode_end_stream(br#"{"metadata":{"k":["v"]}}"#));
        assert_eq!(Ok(()), decode_end_stream(br#"{"error":null}"#));
        assert!(matches!(
            decode_end_stream(b"not json"),
            Err(ClientError::Transport(_))
        ));

        let mut decoder = EnvelopeDecoder::default();
        decoder.push(&encode_envelope(FLAG_COMPRESSED, b"x"));
        assert!(matches!(
            decoder.next_envelope(),
            Err(ClientError::Transport(_))
        ));
    }
}
