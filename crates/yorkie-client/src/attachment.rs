use crate::client::TaskHandle;
use crate::error::{ClientError, ClientResult};
use crate::options::{SyncMode, DEFAULT_POLLING_INTERVAL_MS};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use yorkie_core::Document;

/// A document shared between the application and the client that attaches
/// it (JS passes the same `Document` object to `client.attach`).
///
/// The client locks it only for synchronous steps and never across an
/// `.await`. Document event callbacks run while the client holds this lock,
/// so a callback must not lock the same document again: that deadlocks.
pub type SharedDocument = Arc<Mutex<Document>>;

/// A document attached to a client (JS `Attachment`).
///
/// Dropping it stops its watch stream.
#[derive(Debug)]
pub(crate) struct Attachment {
    pub(crate) doc: SharedDocument,
    pub(crate) resource_id: String,
    pub(crate) sync_mode: SyncMode,
    pub(crate) poll_interval: Duration,
    pub(crate) poll_interval_pinned: bool,
    /// The server reported a remote change not pulled yet (JS
    /// `changeEventReceived`).
    pub(crate) change_event_received: bool,
    /// When the last sync succeeded; `Polling` syncs once per interval (JS
    /// `lastHeartbeatTime`, 0 before the first sync).
    pub(crate) last_synced_at: Option<Instant>,
    /// The watch loop task, while the attachment watches the document.
    pub(crate) watch_task: Option<TaskHandle>,
}

impl Attachment {
    pub(crate) fn new(
        doc: SharedDocument,
        resource_id: impl Into<String>,
        sync_mode: SyncMode,
        poll_interval: Duration,
        poll_interval_pinned: bool,
    ) -> Self {
        Self {
            doc,
            resource_id: resource_id.into(),
            sync_mode,
            poll_interval,
            poll_interval_pinned,
            change_event_received: false,
            last_synced_at: None,
            watch_task: None,
        }
    }

    /// Whether the sync loop should sync this document now (JS
    /// `needRealtimeSync`). `has_local_changes` is read from the document by
    /// the caller, which must not hold the document lock with the client's.
    pub(crate) fn need_realtime_sync(&self, has_local_changes: bool) -> bool {
        match self.sync_mode {
            SyncMode::Manual | SyncMode::RealtimeSyncOff => false,
            SyncMode::RealtimePushOnly => has_local_changes,
            SyncMode::Polling => self
                .last_synced_at
                .is_none_or(|at| at.elapsed() >= self.poll_interval),
            SyncMode::Realtime => has_local_changes || self.change_event_received,
        }
    }

    /// Whether the doc is the one this attachment holds. A key can be
    /// detached and attached again with another document while a request is
    /// in flight, so work that resumes after an `.await` checks this before
    /// touching the attachment.
    pub(crate) fn holds(&self, doc: &SharedDocument) -> bool {
        Arc::ptr_eq(&self.doc, doc)
    }
}

/// Whether the sync mode keeps a watch stream open (JS: every mode but
/// `Manual` and `Polling`).
pub(crate) fn uses_watch_stream(sync_mode: SyncMode) -> bool {
    sync_mode != SyncMode::Manual && sync_mode != SyncMode::Polling
}

pub(crate) fn resolve_document_poll_interval(
    sync_mode: SyncMode,
    document_poll_interval: Option<Duration>,
) -> ClientResult<(Duration, bool)> {
    if let Some(interval) = document_poll_interval {
        if interval.is_zero() {
            return Err(ClientError::InvalidArgument(
                "document_poll_interval must be greater than 0".to_owned(),
            ));
        }
        return Ok((interval, true));
    }

    Ok((default_document_poll_interval(sync_mode), false))
}

pub(crate) fn default_document_poll_interval(sync_mode: SyncMode) -> Duration {
    if sync_mode == SyncMode::Polling {
        Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)
    } else {
        Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yorkie_core::Document;

    fn attachment(sync_mode: SyncMode) -> Attachment {
        let doc = Arc::new(Mutex::new(Document::new("doc-key")));
        let (interval, pinned) = resolve_document_poll_interval(sync_mode, None).unwrap();
        Attachment::new(doc, "document-id", sync_mode, interval, pinned)
    }

    /// JS `Attachment.needRealtimeSync`.
    #[test]
    fn need_realtime_sync_follows_the_sync_mode() {
        let mut realtime = attachment(SyncMode::Realtime);
        assert!(!realtime.need_realtime_sync(false));
        assert!(realtime.need_realtime_sync(true));
        realtime.change_event_received = true;
        assert!(realtime.need_realtime_sync(false));

        let mut push_only = attachment(SyncMode::RealtimePushOnly);
        push_only.change_event_received = true;
        assert!(!push_only.need_realtime_sync(false));
        assert!(push_only.need_realtime_sync(true));

        for sync_mode in [SyncMode::Manual, SyncMode::RealtimeSyncOff] {
            let mut attachment = attachment(sync_mode);
            attachment.change_event_received = true;
            assert!(!attachment.need_realtime_sync(true), "{sync_mode:?}");
        }

        let mut polling = attachment(SyncMode::Polling);
        assert!(polling.need_realtime_sync(false));
        polling.last_synced_at = Some(Instant::now());
        assert!(!polling.need_realtime_sync(true));
        polling.poll_interval = Duration::ZERO;
        assert!(polling.need_realtime_sync(false));
    }
}
