//! The watch loop that keeps a realtime document's stream open.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`runWatchLoop`,
//! `createDocumentWatchStream`, `handleWatchDocumentResponse`),
//! `client/attachment.ts` (`runWatchLoop`), and `client/watch.ts`
//! (`runWatchStream`, `watchIdleTimeout`).
//!
//! The loop opens the stream, applies what it reports to the document, and
//! when the stream fails or ends publishes `ConnectionChanged(Disconnected)`
//! and opens it again after `reconnect_stream_delay`. An error that is not
//! retryable stops the loop. Once the server reports a heartbeat interval, a
//! stream silent for three times that long is treated as half-open and
//! reconnected.

use crate::attachment::SharedDocument;
use crate::client::{lock_document, Client, ClientCondition, ClientInner, TaskHandle};
use crate::error::ClientError;
use crate::transport::{
    ClientTransport, WatchDocEvent, WatchDocEventType, WatchDocumentRequest, WatchDocumentResponse,
};
use futures_util::StreamExt;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::oneshot;
use yorkie_core::{DocEvent, Document, StreamConnectionStatus};

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
    Reconnect,
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
        let Some((transport, request, reconnect_delay)) = open_request(&inner, &key, &doc) else {
            return;
        };
        let end = watch_stream(&inner, &key, &doc, transport, request, &mut ready).await;
        if let Some(ready) = ready.take() {
            let _ = ready.send(());
        }
        match end {
            StreamEnd::Stop => return,
            StreamEnd::Reconnect => tokio::time::sleep(reconnect_delay).await,
        }
    }
}

/// Builds the request for the next stream and announces it, or returns
/// `None` when the client is gone, inactive, or no longer holds the document.
fn open_request(
    inner: &Weak<ClientInner>,
    key: &str,
    doc: &SharedDocument,
) -> Option<(Arc<dyn ClientTransport>, WatchDocumentRequest, Duration)> {
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
    Some((
        client.inner.transport.clone(),
        request,
        client.options().reconnect_stream_delay,
    ))
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
        Err(err) => return stream_failed(inner, key, doc, Some(&err)),
    };

    let mut idle_timeout = None;
    loop {
        let next = match idle_timeout {
            Some(timeout) => match tokio::time::timeout(timeout, stream.next()).await {
                Ok(next) => next,
                Err(_) => Some(Err(ClientError::Transport(format!(
                    "watch stream is idle for {}ms",
                    timeout.as_millis()
                )))),
            },
            None => stream.next().await,
        };
        let response = match next {
            Some(Ok(response)) => response,
            Some(Err(err)) => return stream_failed(inner, key, doc, Some(&err)),
            None => return stream_failed(inner, key, doc, None),
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

/// Handles a stream that failed with `err`, or ended normally when `err` is
/// `None` (JS `onError`/`onStreamEnd` and `handleConnectError`).
fn stream_failed(
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
    publish_disconnected(&mut lock_document(doc));
    match err {
        Some(err) if !err.is_retryable() => {
            client.set_condition(ClientCondition::WatchLoop, false);
            StreamEnd::Stop
        }
        _ => StreamEnd::Reconnect,
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
