//! The watch loops that keep the streams of realtime documents and channels
//! open.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`runWatchLoop`,
//! `createDocumentWatchStream`, `handleWatchDocumentResponse`,
//! `createChannelWatchStream`, `handleWatchChannelResponse`),
//! `client/attachment.ts` (`runWatchLoop`), and `client/watch.ts`
//! (`runWatchStream`, `watchIdleTimeout`).
//!
//! A loop opens the stream, applies what it reports to the document or
//! channel, and when the stream fails or ends opens it again after
//! `reconnect_stream_delay` (longer while the auth token refresh backs off,
//! see `Client::loop_retry_delay`); a document also publishes
//! `ConnectionChanged(Disconnected)`, and `AuthError` when the server refused
//! the stream with `ErrUnauthenticated`. Errors are handled as JS
//! `handleConnectError` does: one that is not retryable stops the loop. Once
//! the server reports a heartbeat interval, a stream silent for three times
//! that long is treated as half-open and reconnected.

use crate::attachment::SharedDocument;
use crate::channel::{first_key_path, Channel, ChannelEvent, SharedChannel};
use crate::client::{
    lock_channel, lock_document, Client, ClientCondition, ClientInner, TaskHandle,
};
use crate::error::{ClientError, ClientResult};
use crate::transport::{
    ClientTransport, WatchChannelEvent, WatchChannelEventType, WatchChannelRequest,
    WatchChannelResponse, WatchDocEvent, WatchDocEventType, WatchDocumentRequest,
    WatchDocumentResponse,
};
use futures_util::{Stream, StreamExt};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::oneshot;
use yorkie_core::{DocEvent, Document, ErrorCode, StreamConnectionStatus};

/// JS `WatchIdleTimeoutFactor`.
const WATCH_IDLE_TIMEOUT_FACTOR: u32 = 3;
/// JS `WatchIdleTimeoutMin`.
const WATCH_IDLE_TIMEOUT_MIN: Duration = Duration::from_secs(1);
/// JS `WatchIdleTimeoutMax`.
const WATCH_IDLE_TIMEOUT_MAX: Duration = Duration::from_secs(24 * 60 * 60);

/// Starts the watch loop of an attached document. The receiver completes
/// when the first stream is initialized or its first attempt ends, which is
/// when JS `runWatchLoop` resolves.
pub(crate) fn spawn(
    inner: &Arc<ClientInner>,
    key: String,
    doc: SharedDocument,
) -> (TaskHandle, oneshot::Receiver<()>) {
    let (ready, ready_rx) = oneshot::channel();
    let task = inner.spawn_task(run(Arc::downgrade(inner), key, doc, ready));
    (task, ready_rx)
}

/// Publishes what JS publishes when a document's watch stream closes: the
/// online clients are cleared, `Initialized` lists what is left, and the
/// connection is reported as disconnected.
pub(crate) fn publish_disconnected(doc: &mut Document) {
    doc.reset_online_clients();
    let presences = doc.presences();
    doc.publish(DocEvent::Initialized(presences));
    doc.publish(DocEvent::ConnectionChanged(
        StreamConnectionStatus::Disconnected,
    ));
}

/// How long a stream may stay silent before it is treated as half-open, given
/// the heartbeat interval the server reported (JS `watchIdleTimeout`). `None`
/// when the server sends no heartbeats.
pub(crate) fn watch_idle_timeout(heartbeat_interval_ms: u64) -> Option<Duration> {
    if heartbeat_interval_ms == 0 {
        return None;
    }
    let interval = Duration::from_millis(heartbeat_interval_ms);
    Some(
        interval
            .saturating_mul(WATCH_IDLE_TIMEOUT_FACTOR)
            .clamp(WATCH_IDLE_TIMEOUT_MIN, WATCH_IDLE_TIMEOUT_MAX),
    )
}

/// What the loop does after a stream ends.
enum StreamEnd {
    /// Open a new stream after the given wait.
    Reconnect(Duration),
    Stop,
}

/// What the loop does after a stream message.
enum Handled {
    Continue,
    Initialized { heartbeat_interval_ms: u64 },
    Stop,
}

async fn run(
    inner: Weak<ClientInner>,
    key: String,
    doc: SharedDocument,
    ready: oneshot::Sender<()>,
) {
    let mut ready = Some(ready);
    loop {
        let Some((transport, request)) = open_request(&inner, &key, &doc) else {
            return;
        };
        let end = watch_stream(&inner, &key, &doc, transport, request, &mut ready).await;
        if let Some(ready) = ready.take() {
            let _ = ready.send(());
        }
        match end {
            StreamEnd::Stop => return,
            StreamEnd::Reconnect(delay) => tokio::time::sleep(delay).await,
        }
    }
}

/// Builds the request for the next stream and announces it, or returns
/// `None` when the client is gone, inactive, or no longer holds the document.
fn open_request(
    inner: &Weak<ClientInner>,
    key: &str,
    doc: &SharedDocument,
) -> Option<(Arc<dyn ClientTransport>, WatchDocumentRequest)> {
    let client = Client::upgrade(inner)?;
    let Ok(session) = client.session() else {
        client.set_condition(ClientCondition::WatchLoop, false);
        return None;
    };
    let heartbeat_interval_ms = ceil_millis(client.options().watch_heartbeat_interval);
    let document_id = client.with_attachment(key, doc, |attachment| {
        // JS: pull once per stream, in case a change landed while it was
        // closed.
        attachment.change_event_received = true;
        attachment.resource_id.clone()
    })?;
    let request = WatchDocumentRequest {
        client_id: session.client_id,
        actor_id: session.actor_id,
        document_id,
        heartbeat_interval_ms,
        shard_key: client.shard_key(key),
    };
    lock_document(doc).publish(DocEvent::ConnectionChanged(
        StreamConnectionStatus::Connected,
    ));
    Some((client.inner.transport.clone(), request))
}

/// Opens one stream and handles it until it ends.
async fn watch_stream(
    inner: &Weak<ClientInner>,
    key: &str,
    doc: &SharedDocument,
    transport: Arc<dyn ClientTransport>,
    request: WatchDocumentRequest,
    ready: &mut Option<oneshot::Sender<()>>,
) -> StreamEnd {
    let mut stream = match transport.watch_document(request).await {
        Ok(stream) => stream,
        Err(err) => return stream_failed(inner, key, doc, Some(&err)).await,
    };

    let mut idle_timeout = None;
    loop {
        let response = match next_message(&mut stream, idle_timeout).await {
            Some(Ok(response)) => response,
            Some(Err(err)) => return stream_failed(inner, key, doc, Some(&err)).await,
            None => return stream_failed(inner, key, doc, None).await,
        };

        let Some(client) = Client::upgrade(inner) else {
            return StreamEnd::Stop;
        };
        match handle_response(&client, key, doc, response) {
            Handled::Continue => {}
            Handled::Initialized {
                heartbeat_interval_ms,
            } => {
                idle_timeout = watch_idle_timeout(heartbeat_interval_ms);
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
            Handled::Stop => return StreamEnd::Stop,
        }
    }
}

/// Waits for the next stream message. A stream silent for `idle_timeout`
/// yields an error, which reconnects it (JS `runWatchStream`).
async fn next_message<S, T>(
    stream: &mut S,
    idle_timeout: Option<Duration>,
) -> Option<ClientResult<T>>
where
    S: Stream<Item = ClientResult<T>> + Unpin,
{
    let Some(timeout) = idle_timeout else {
        return stream.next().await;
    };
    match tokio::time::timeout(timeout, stream.next()).await {
        Ok(next) => next,
        Err(_) => Some(Err(ClientError::Transport(format!(
            "watch stream is idle for {}ms",
            timeout.as_millis()
        )))),
    }
}

/// Handles a stream that failed with `err`, or ended normally when `err` is
/// `None` (JS `onError`/`onStreamEnd` and `handleConnectError`).
async fn stream_failed(
    inner: &Weak<ClientInner>,
    key: &str,
    doc: &SharedDocument,
    err: Option<&ClientError>,
) -> StreamEnd {
    let Some(client) = Client::upgrade(inner) else {
        return StreamEnd::Stop;
    };
    if client.with_attachment(key, doc, |_| ()).is_none() {
        return StreamEnd::Stop;
    }
    {
        let mut doc = lock_document(doc);
        publish_disconnected(&mut doc);
        if let Some(err) = err.filter(|err| err.is_error_code(ErrorCode::ErrUnauthenticated)) {
            doc.publish(DocEvent::AuthError {
                reason: err.error_reason().map(str::to_owned),
                method: "Watch".to_owned(),
            });
        }
    }
    reconnect_or_stop(&client, err).await
}

/// Decides what a watch loop does after its stream ended with `err`, or
/// normally when `err` is `None`.
async fn reconnect_or_stop(client: &Client, err: Option<&ClientError>) -> StreamEnd {
    let delay = client.options().reconnect_stream_delay;
    let Some(err) = err else {
        return StreamEnd::Reconnect(delay);
    };
    if matches!(client.handle_connect_error(err, true).await, Ok(true)) {
        StreamEnd::Reconnect(client.loop_retry_delay(err, delay))
    } else {
        client.set_condition(ClientCondition::WatchLoop, false);
        StreamEnd::Stop
    }
}

/// Applies one stream message (JS `handleWatchDocumentResponse`).
fn handle_response(
    client: &Client,
    key: &str,
    doc: &SharedDocument,
    response: WatchDocumentResponse,
) -> Handled {
    match response {
        WatchDocumentResponse::Initialization {
            client_ids,
            heartbeat_interval_ms,
        } => {
            if client.with_attachment(key, doc, |_| ()).is_none() {
                return Handled::Stop;
            }
            lock_document(doc).apply_watch_init(client_ids);
            Handled::Initialized {
                heartbeat_interval_ms,
            }
        }
        WatchDocumentResponse::Event(WatchDocEvent {
            event_type,
            publisher,
        }) => {
            let attached = client.with_attachment(key, doc, |attachment| {
                if event_type == WatchDocEventType::DocumentChanged {
                    attachment.change_event_received = true;
                }
            });
            if attached.is_none() {
                return Handled::Stop;
            }
            match event_type {
                WatchDocEventType::DocumentChanged => {}
                WatchDocEventType::DocumentWatched => lock_document(doc).apply_watched(publisher),
                WatchDocEventType::DocumentUnwatched => {
                    lock_document(doc).apply_unwatched(&publisher)
                }
                // JS ignores it too: broadcasts travel on channels.
                WatchDocEventType::DocumentBroadcast => {}
            }
            Handled::Continue
        }
        WatchDocumentResponse::Heartbeat => Handled::Continue,
    }
}

/// Starts the watch loop of an attached channel (JS `runWatchLoop` with
/// `createChannelWatchStream`). The receiver completes as `spawn`'s does.
pub(crate) fn spawn_channel(
    inner: &Arc<ClientInner>,
    key: String,
    channel: SharedChannel,
) -> (TaskHandle, oneshot::Receiver<()>) {
    let (ready, ready_rx) = oneshot::channel();
    let task = inner.spawn_task(run_channel(Arc::downgrade(inner), key, channel, ready));
    (task, ready_rx)
}

async fn run_channel(
    inner: Weak<ClientInner>,
    key: String,
    channel: SharedChannel,
    ready: oneshot::Sender<()>,
) {
    let mut ready = Some(ready);
    loop {
        let Some((transport, request)) = open_channel_request(&inner, &key, &channel) else {
            return;
        };
        let end =
            watch_channel_stream(&inner, &key, &channel, transport, request, &mut ready).await;
        if let Some(ready) = ready.take() {
            let _ = ready.send(());
        }
        match end {
            StreamEnd::Stop => return,
            StreamEnd::Reconnect(delay) => tokio::time::sleep(delay).await,
        }
    }
}

/// Builds the request for the next channel stream, or returns `None` when
/// the client is gone, inactive, or no longer holds the channel.
fn open_channel_request(
    inner: &Weak<ClientInner>,
    key: &str,
    channel: &SharedChannel,
) -> Option<(Arc<dyn ClientTransport>, WatchChannelRequest)> {
    let client = Client::upgrade(inner)?;
    let Ok(session) = client.session() else {
        client.set_condition(ClientCondition::WatchLoop, false);
        return None;
    };
    client.with_channel_attachment(key, channel, |_| ())?;
    let request = WatchChannelRequest {
        client_id: session.client_id,
        channel_key: key.to_owned(),
        heartbeat_interval_ms: ceil_millis(client.options().watch_heartbeat_interval),
        shard_key: client.shard_key(first_key_path(key)),
    };
    Some((client.inner.transport.clone(), request))
}

/// Opens one channel stream and handles it until it ends.
async fn watch_channel_stream(
    inner: &Weak<ClientInner>,
    key: &str,
    channel: &SharedChannel,
    transport: Arc<dyn ClientTransport>,
    request: WatchChannelRequest,
    ready: &mut Option<oneshot::Sender<()>>,
) -> StreamEnd {
    let mut stream = match transport.watch_channel(request).await {
        Ok(stream) => stream,
        Err(err) => return channel_stream_failed(inner, key, channel, Some(&err)).await,
    };

    let mut idle_timeout = None;
    loop {
        let response = match next_message(&mut stream, idle_timeout).await {
            Some(Ok(response)) => response,
            Some(Err(err)) => return channel_stream_failed(inner, key, channel, Some(&err)).await,
            None => return channel_stream_failed(inner, key, channel, None).await,
        };

        let Some(client) = Client::upgrade(inner) else {
            return StreamEnd::Stop;
        };
        if client
            .with_channel_attachment(key, channel, |_| ())
            .is_none()
        {
            return StreamEnd::Stop;
        }
        drop(client);
        let initialized = match &response {
            WatchChannelResponse::Initialization {
                heartbeat_interval_ms,
                ..
            } => {
                idle_timeout = watch_idle_timeout(*heartbeat_interval_ms);
                true
            }
            _ => false,
        };
        handle_channel_response(&mut lock_channel(channel), response);
        if initialized {
            if let Some(ready) = ready.take() {
                let _ = ready.send(());
            }
        }
    }
}

/// Handles a channel stream that failed with `err`, or ended normally when
/// `err` is `None`. Unlike a document, a channel publishes nothing, as in JS.
async fn channel_stream_failed(
    inner: &Weak<ClientInner>,
    key: &str,
    channel: &SharedChannel,
    err: Option<&ClientError>,
) -> StreamEnd {
    let Some(client) = Client::upgrade(inner) else {
        return StreamEnd::Stop;
    };
    if client
        .with_channel_attachment(key, channel, |_| ())
        .is_none()
    {
        return StreamEnd::Stop;
    }
    reconnect_or_stop(&client, err).await
}

/// Applies one channel stream message (JS `handleWatchChannelResponse`).
/// Counts older than the last one taken are dropped; a broadcast whose
/// payload is not JSON is skipped.
fn handle_channel_response(channel: &mut Channel, response: WatchChannelResponse) {
    match response {
        WatchChannelResponse::Initialization {
            session_count, seq, ..
        } => {
            if channel.update_session_count(session_count, seq) {
                channel.publish(ChannelEvent::Initialized {
                    count: session_count,
                });
            }
        }
        WatchChannelResponse::Event(WatchChannelEvent {
            event_type: WatchChannelEventType::Broadcast,
            publisher,
            topic,
            payload,
            ..
        }) => {
            if let Ok(payload) = serde_json::from_slice(&payload) {
                channel.publish(ChannelEvent::Broadcast {
                    client_id: publisher,
                    topic,
                    payload,
                });
            }
        }
        WatchChannelResponse::Event(WatchChannelEvent {
            event_type: WatchChannelEventType::Presence,
            session_count,
            seq,
            ..
        }) => {
            if channel.update_session_count(session_count, seq) {
                channel.publish(ChannelEvent::PresenceChanged {
                    count: session_count,
                });
            }
        }
        WatchChannelResponse::Heartbeat => {}
    }
}

/// Converts a duration to whole milliseconds, rounding up so a tiny positive
/// interval is not sent as 0, which asks for no heartbeats (JS
/// `toHeartbeatIntervalMs`).
fn ceil_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JS `watchIdleTimeout`.
    #[test]
    fn idle_timeout_is_three_heartbeats_within_bounds() {
        assert_eq!(None, watch_idle_timeout(0));
        assert_eq!(Some(Duration::from_secs(60)), watch_idle_timeout(20_000));
        assert_eq!(Some(Duration::from_secs(1)), watch_idle_timeout(5));
        assert_eq!(
            Some(Duration::from_secs(24 * 60 * 60)),
            watch_idle_timeout(u64::MAX)
        );
    }

    /// JS `toHeartbeatIntervalMs`.
    #[test]
    fn heartbeat_interval_rounds_up_to_milliseconds() {
        assert_eq!(0, ceil_millis(Duration::ZERO));
        assert_eq!(1, ceil_millis(Duration::from_micros(1)));
        assert_eq!(20_000, ceil_millis(Duration::from_secs(20)));
    }
}
