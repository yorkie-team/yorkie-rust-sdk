//! Connect protocol transport for unary Yorkie RPCs.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (Connect client setup,
//! `x-api-key`, `x-shard-key` and `x-yorkie-user-agent` headers).

use crate::error::{ClientError, ClientResult};
use crate::options::ClientOptions;
use crate::protocol as conv;
use crate::transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    ClientTransport, DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
    DetachDocumentResponse, PushPullChangesRequest, PushPullChangesResponse, RemoveDocumentRequest,
    RemoveDocumentResponse,
};
use async_trait::async_trait;
use prost::Message;
use yorkie_protocol::yorkie::v1 as api;

const SERVICE_PATH: &str = "yorkie.v1.YorkieService";

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
        let mut builder = self
            .http
            .post(format!("{}/{SERVICE_PATH}/{method}", self.base_url))
            .header("content-type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("x-shard-key", shard_key)
            .header("x-yorkie-user-agent", &self.user_agent);
        if !self.api_key.is_empty() {
            builder = builder.header("x-api-key", &self.api_key);
        }

        let response = builder
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
            })
        })
        .unwrap_or_default();
    match parsed.code {
        Some(code) => ClientError::Rpc {
            code,
            message: parsed.message,
        },
        None => ClientError::Rpc {
            code: http_status_to_code(status).to_owned(),
            message: String::from_utf8_lossy(body).into_owned(),
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
            },
            err
        );
    }

    #[test]
    fn non_json_error_body_maps_http_status() {
        assert_eq!(
            ClientError::Rpc {
                code: "unauthenticated".to_owned(),
                message: "nope".to_owned(),
            },
            decode_connect_error(401, b"nope")
        );
        assert!(matches!(
            decode_connect_error(404, b"<html>"),
            ClientError::Rpc { code, .. } if code == "unimplemented"
        ));
    }
}
