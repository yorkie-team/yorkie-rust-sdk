//! Document event collectors for the subscriptions JS tests make: the
//! default one (`doc.subscribe(cb)`), `'sync'`, `'presence'`, and
//! `'connection'`. A Rust subscription gets every event, so each collector
//! keeps only the events of its JS topic.

use super::EventCollector;
use yorkie::{DocEvent, DocSyncStatus, SharedDocument, StreamConnectionStatus};

/// The JS `DocEventType` names of the events a default JS subscription
/// delivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeEventType {
    Snapshot,
    LocalChange,
    RemoteChange,
}

/// Collects the types of snapshot and change events (JS `doc.subscribe(cb)`
/// adding `event.type`).
pub fn collect_change_types(doc: &SharedDocument) -> EventCollector<ChangeEventType> {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    doc.lock().unwrap().subscribe(move |event| {
        let event_type = match event {
            DocEvent::Snapshot => ChangeEventType::Snapshot,
            DocEvent::LocalChange(_) => ChangeEventType::LocalChange,
            DocEvent::RemoteChange(_) => ChangeEventType::RemoteChange,
            _ => return,
        };
        sink(event_type);
    });
    collector
}

/// Collects sync results (JS `doc.subscribe('sync', cb)` adding
/// `event.value`).
pub fn collect_sync_status(doc: &SharedDocument) -> EventCollector<DocSyncStatus> {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    doc.lock().unwrap().subscribe(move |event| {
        if let DocEvent::SyncStatusChanged(status) = event {
            sink(*status);
        }
    });
    collector
}

/// Collects presence events: `Initialized`, `Watched`, `Unwatched`, and
/// `PresenceChanged` (JS `doc.subscribe('presence', cb)`).
pub fn collect_presence(doc: &SharedDocument) -> EventCollector<DocEvent> {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    doc.lock().unwrap().subscribe(move |event| {
        if matches!(
            event,
            DocEvent::Initialized(_)
                | DocEvent::Watched(..)
                | DocEvent::Unwatched(..)
                | DocEvent::PresenceChanged(..)
        ) {
            sink(event.clone());
        }
    });
    collector
}

/// Collects watch stream status changes (JS `doc.subscribe('connection',
/// cb)`).
pub fn collect_connection(doc: &SharedDocument) -> EventCollector<StreamConnectionStatus> {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    doc.lock().unwrap().subscribe(move |event| {
        if let DocEvent::ConnectionChanged(status) = event {
            sink(*status);
        }
    });
    collector
}
