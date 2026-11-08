use crate::error::{ClientError, ClientResult};
use crate::options::{SyncMode, DEFAULT_POLLING_INTERVAL_MS};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yorkie_core::Document;

/// A document shared between the application and the client that attaches
/// it (JS passes the same `Document` object to `client.attach`).
///
/// The client locks it only for synchronous steps and never across an
/// `.await`. Document event callbacks run while the client holds this lock,
/// so a callback must not lock the same document again: that deadlocks.
pub type SharedDocument = Arc<Mutex<Document>>;

/// A document attached to a client (JS `Attachment`).
#[derive(Debug, Clone)]
pub(crate) struct Attachment {
    pub(crate) doc: SharedDocument,
    pub(crate) resource_id: String,
    pub(crate) sync_mode: SyncMode,
    pub(crate) poll_interval: Duration,
    pub(crate) poll_interval_pinned: bool,
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
        }
    }
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
