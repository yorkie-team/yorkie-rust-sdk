//! The sync loop that pushes and pulls realtime documents and keeps channel
//! sessions alive in the background.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`runSyncLoop`) and
//! `client/attachment.ts` (`needRealtimeSync`, `needSync`). Each round runs on
//! the client's task queue, syncs every attachment that needs it at once, and
//! then waits `sync_loop_duration`. A failed round is handled as JS
//! `handleConnectError` does, after the round leaves the queue: a retryable
//! error waits `retry_sync_loop_delay` (longer while the auth token refresh
//! backs off, see `Client::loop_retry_delay`), and any other error stops the
//! loop.
//!
//! A document whose push the server refuses for its content
//! (`ErrDocumentSizeExceedsLimit`, `ErrChangeTooLarge`) is parked instead:
//! the loop skips it until a sync of it succeeds, and goes on with the rest.
//! A sync refused with `ErrUnauthenticated` or `ErrPermissionDenied`
//! publishes `DocEvent::AuthError` on the document.

use crate::attachment::SharedDocument;
use crate::channel::SharedChannel;
use crate::client::{lock_document, Client, ClientCondition, ClientInner, SyncTarget, TaskHandle};
use crate::error::{ClientError, ClientResult, UNRETRYABLE_WRITE_CODES};
use futures_util::future::{join, join_all};
use std::sync::{Arc, Weak};
use yorkie_core::{DocEvent, ErrorCode};

/// Starts the sync loop of a client that is active or has channels.
pub(crate) fn spawn(inner: &Arc<ClientInner>) -> TaskHandle {
    inner.spawn_task(run(Arc::downgrade(inner)))
}

async fn run(inner: Weak<ClientInner>) {
    loop {
        // Hold the client only for one round, so dropping every handle to it
        // ends the loop.
        let Some(client) = Client::upgrade(&inner) else {
            return;
        };
        let failure = {
            let _queue = client.lock_task_queue().await;
            let result = sync_attachments(&client).await;
            if let Err(err) = &result {
                if !err.is_retryable() {
                    // Cleared before the queue is released: a call queued
                    // behind this round, such as `attach_channel`, must see
                    // the loop as stopped and start a new one.
                    client.set_condition(ClientCondition::SyncLoop, false);
                }
            }
            result.err()
        };
        let delay = match failure {
            None => client.options().sync_loop_duration,
            // Outside the queue, as in JS: a token refresh waits for the
            // injector without holding up the app's calls.
            Some(err) => {
                if !matches!(client.handle_connect_error(&err, true).await, Ok(true)) {
                    return;
                }
                client.loop_retry_delay(&err, client.options().retry_sync_loop_delay)
            }
        };
        drop(client);
        tokio::time::sleep(delay).await;
    }
}

/// Runs one round, on the task queue the caller holds: syncs, in parallel, every document whose sync mode and
/// state call for it and refreshes every channel whose heartbeat is due, and
/// returns the first error.
///
/// As in JS, a client that is not active yet keeps the loop for its
/// channels, whose first refresh activates it. With no channels either, the
/// round fails with `ClientNotActivated`, which ends the loop.
async fn sync_attachments(client: &Client) -> ClientResult<()> {
    let (session, channels) = {
        let state = client.state();
        let session = state.session(client.key());
        if session.is_err() && state.channels.is_empty() {
            return session.map(|_| ());
        }
        let heartbeat_interval = client.options().channel_heartbeat_interval;
        let channels: Vec<(String, SharedChannel)> = state
            .channels
            .iter()
            .filter(|(_, attachment)| attachment.need_sync(heartbeat_interval))
            .map(|(key, attachment)| (key.clone(), attachment.channel.clone()))
            .collect();
        (session.ok(), channels)
    };

    let client_id = session.map(|session| session.client_id);
    let targets = match client_id {
        Some(_) => document_targets(client),
        None => Vec::new(),
    };
    let syncs = targets
        .into_iter()
        .filter_map(|(target, change_event_received)| {
            let client_id = client_id.as_ref()?;
            Some(async move {
                let key = target.key.clone();
                let doc = target.doc.clone();
                let result = client.sync_document(client_id, target).await;
                if let Err(err) = &result {
                    if change_event_received {
                        // The pull this sync owed never happened; ask again.
                        client.with_attachment(&key, &doc, |attachment| {
                            attachment.change_event_received = true;
                        });
                    }
                    if handle_document_error(client, &key, &doc, err) {
                        return Ok(());
                    }
                }
                result
            })
        });
    let refreshes = channels
        .into_iter()
        .map(|(key, channel)| client.refresh_channel(key, channel));

    let (synced, refreshed) = join(join_all(syncs), join_all(refreshes)).await;
    synced.into_iter().chain(refreshed).collect()
}

/// Handles a failed sync of one document as the JS sync loop does before the
/// error reaches `handleConnectError`: publishes `AuthError` for a refusal,
/// and parks the document for a push refused for its content. Returns
/// whether it parked it, in which case the error does not fail the round:
/// that would stop the loop for every other document and channel.
fn handle_document_error(
    client: &Client,
    key: &str,
    doc: &SharedDocument,
    err: &ClientError,
) -> bool {
    if err.is_error_code(ErrorCode::ErrUnauthenticated)
        || err.is_error_code(ErrorCode::ErrPermissionDenied)
    {
        lock_document(doc).publish(DocEvent::AuthError {
            reason: err.error_reason().map(str::to_owned),
            method: "PushPull".to_owned(),
        });
    }
    if !UNRETRYABLE_WRITE_CODES
        .iter()
        .any(|code| err.is_error_code(*code))
    {
        return false;
    }
    client.with_attachment(key, doc, |attachment| {
        attachment.write_rejected = true;
    });
    true
}

/// Picks the documents to sync this round, each with the remote-change
/// notice it consumed. Parked documents are skipped.
fn document_targets(client: &Client) -> Vec<(SyncTarget, bool)> {
    // Read local changes without the client lock held, then decide under it.
    let docs: Vec<(String, SharedDocument)> = client
        .state()
        .attachments
        .iter()
        .map(|(key, attachment)| (key.clone(), attachment.doc.clone()))
        .collect();
    let docs: Vec<(String, SharedDocument, bool)> = docs
        .into_iter()
        .map(|(key, doc)| {
            let has_local_changes = lock_document(&doc).has_local_changes();
            (key, doc, has_local_changes)
        })
        .collect();
    let mut state = client.state();
    docs.into_iter()
        .filter_map(|(key, doc, has_local_changes)| {
            let attachment = state
                .attachments
                .get_mut(&key)
                .filter(|attachment| attachment.holds(&doc))?;
            if attachment.write_rejected || !attachment.need_realtime_sync(has_local_changes) {
                return None;
            }
            // Cleared before the request goes out, so a change event that
            // arrives meanwhile asks for another round.
            let change_event_received = std::mem::take(&mut attachment.change_event_received);
            let sync_mode = attachment.sync_mode;
            Some((
                SyncTarget::new(key, attachment, sync_mode),
                change_event_received,
            ))
        })
        .collect()
}
