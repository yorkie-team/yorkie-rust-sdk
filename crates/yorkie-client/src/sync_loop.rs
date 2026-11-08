//! The sync loop that pushes and pulls realtime documents in the background.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`runSyncLoop`) and
//! `client/attachment.ts` (`needRealtimeSync`). Each round runs on the
//! client's task queue, syncs every attachment that needs it at once, and
//! then waits `sync_loop_duration`; after a retryable error it waits
//! `retry_sync_loop_delay` instead, and any other error stops the loop.

use crate::attachment::SharedDocument;
use crate::client::{lock_document, Client, ClientCondition, ClientInner, SyncTarget, TaskHandle};
use crate::error::ClientResult;
use futures_util::future::join_all;
use std::sync::{Arc, Weak};

/// Starts the sync loop of an active client.
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
        let delay = match sync_attachments(&client).await {
            Ok(()) => client.options().sync_loop_duration,
            Err(err) if err.is_retryable() => client.options().retry_sync_loop_delay,
            Err(_) => {
                client.set_condition(ClientCondition::SyncLoop, false);
                return;
            }
        };
        drop(client);
        tokio::time::sleep(delay).await;
    }
}

/// Runs one round: syncs, in parallel, every attachment whose sync mode and
/// state call for it, and returns the first error.
async fn sync_attachments(client: &Client) -> ClientResult<()> {
    let _queue = client.lock_task_queue().await;
    let session = client.session()?;

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
    let targets: Vec<(SyncTarget, bool)> = {
        let mut state = client.state();
        docs.into_iter()
            .filter_map(|(key, doc, has_local_changes)| {
                let attachment = state
                    .attachments
                    .get_mut(&key)
                    .filter(|attachment| attachment.holds(&doc))?;
                if !attachment.need_realtime_sync(has_local_changes) {
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
    };

    let syncs = targets.into_iter().map(|(target, change_event_received)| {
        let client_id = &session.client_id;
        async move {
            let key = target.key.clone();
            let doc = target.doc.clone();
            let result = client.sync_document(client_id, target).await;
            if result.is_err() && change_event_received {
                // The pull this sync owed never happened; ask again.
                client.with_attachment(&key, &doc, |attachment| {
                    attachment.change_event_received = true;
                });
            }
            result
        }
    });
    join_all(syncs).await.into_iter().collect()
}
