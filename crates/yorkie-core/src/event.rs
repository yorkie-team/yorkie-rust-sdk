//! Document events and subscriptions, mirroring JS `DocEventType`/`DocEvent`
//! and `subscribe`/`publish` in `document/document.ts`, and the `OpInfo`
//! types in `document/operation/operation.ts`.

use crate::crdt::counter::CounterValue;
use crate::document::DocStatus;
use crate::presence::Presence;
use crate::time::ActorId;
use crate::TreeNodeInput;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// Whether the watch stream is connected (JS `StreamConnectionStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamConnectionStatus {
    Connected,
    Disconnected,
}

/// The result of synchronizing the document with the server
/// (JS `DocSyncStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocSyncStatus {
    Synced,
    SyncFailed,
}

/// An event that occurs in a `Document` (JS `DocEvent`).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DocEvent {
    /// The document status changed (JS `status-changed`).
    StatusChanged(DocStatus),
    /// The watch stream connection changed (JS `connection-changed`).
    ConnectionChanged(StreamConnectionStatus),
    /// A sync with the server finished (JS `sync-status-changed`).
    SyncStatusChanged(DocSyncStatus),
    /// A snapshot from the server replaced the document (JS `snapshot`).
    Snapshot,
    /// A local change was applied (JS `local-change`).
    LocalChange(ChangeInfo),
    /// A remote change was applied (JS `remote-change`).
    RemoteChange(ChangeInfo),
    /// The online clients were loaded from the server (JS `initialized`).
    Initialized(Vec<(ActorId, Presence)>),
    /// A client started watching the document (JS `watched`).
    Watched(ActorId, Presence),
    /// A client stopped watching the document (JS `unwatched`).
    Unwatched(ActorId, Presence),
    /// A client's presence changed (JS `presence-changed`).
    PresenceChanged(ActorId, Presence),
    /// The server refused this client's requests for the document (JS
    /// `auth-error`): a sync (`method` `"PushPull"`) with
    /// `ErrUnauthenticated` or `ErrPermissionDenied`, or the watch stream
    /// (`"Watch"`) with `ErrUnauthenticated`. `reason` is the auth webhook's
    /// reason, if the server gave one. After `ErrUnauthenticated` the client
    /// asks the auth token injector for a new token and retries.
    AuthError {
        reason: Option<String>,
        method: String,
    },
}

/// The operations made by one change and its message (JS `ChangeInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeInfo {
    pub actor: ActorId,
    pub message: Option<String>,
    pub operations: Vec<OperationInfo>,
}

/// What an executed operation did (JS `OpInfo`). `path` is the JSON path of
/// the target element, such as `$.todos.0`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum OperationInfo {
    /// JS `SetOpInfo`.
    Set { path: String, key: String },
    /// JS `RemoveOpInfo`: `key` for an object member, `index` for an array
    /// element.
    Remove {
        path: String,
        key: Option<String>,
        index: Option<usize>,
    },
    /// JS `AddOpInfo`.
    Add { path: String, index: usize },
    /// JS `MoveOpInfo`.
    Move {
        path: String,
        index: usize,
        previous_index: usize,
    },
    /// JS `ArraySetOpInfo`.
    ArraySet { path: String },
    /// JS `IncreaseOpInfo`.
    Increase { path: String, value: CounterValue },
    /// JS `EditOpInfo`.
    Edit {
        path: String,
        from: usize,
        to: usize,
        value: EditOpValue,
    },
    /// JS `StyleOpInfo`.
    Style {
        path: String,
        from: usize,
        to: usize,
        value: StyleOpValue,
    },
    /// JS `TreeEditOpInfo`. `value` holds the inserted nodes, if any.
    TreeEdit {
        path: String,
        from: usize,
        to: usize,
        value: Option<Vec<TreeNodeInput>>,
        split_level: Option<usize>,
        from_path: Vec<usize>,
        to_path: Vec<usize>,
    },
    /// JS `TreeStyleOpInfo`.
    TreeStyle {
        path: String,
        from: usize,
        to: usize,
        from_path: Vec<usize>,
        to_path: Vec<usize>,
        value: TreeStyleOpValue,
    },
}

impl OperationInfo {
    /// Returns the JSON path of the element the operation targeted.
    pub fn path(&self) -> &str {
        match self {
            Self::Set { path, .. }
            | Self::Remove { path, .. }
            | Self::Add { path, .. }
            | Self::Move { path, .. }
            | Self::ArraySet { path }
            | Self::Increase { path, .. }
            | Self::Edit { path, .. }
            | Self::Style { path, .. }
            | Self::TreeEdit { path, .. }
            | Self::TreeStyle { path, .. } => path,
        }
    }
}

/// The `value` of a text edit (JS `EditOpInfo.value`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditOpValue {
    pub attributes: BTreeMap<String, Value>,
    pub content: String,
}

/// The `value` of a text style change (JS `StyleOpInfo.value`). A removed
/// attribute is reported with a `null` value, as JS does for editors.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleOpValue {
    pub attributes: BTreeMap<String, Value>,
}

/// The `value` of a tree style change (JS `TreeStyleOpInfo.value`): either
/// the attributes set or the attribute keys removed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TreeStyleOpValue {
    pub attributes: Option<BTreeMap<String, Value>>,
    pub attributes_to_remove: Option<Vec<String>>,
}

/// Identifies a subscription made with `Document::subscribe` or
/// `Document::subscribe_path` (JS `Unsubscribe` handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

type Callback = Box<dyn FnMut(&DocEvent) + Send>;

struct Subscriber {
    id: SubscriptionId,
    target_path: Option<String>,
    callback: Callback,
}

/// The subscribers of a document.
///
/// A cloned document has no subscribers, as a JS document clone has none,
/// and subscribers never make two documents unequal.
#[derive(Default)]
pub(crate) struct Subscribers {
    next_id: u64,
    entries: Vec<Subscriber>,
}

impl Subscribers {
    pub(crate) fn add(
        &mut self,
        target_path: Option<String>,
        callback: Callback,
    ) -> SubscriptionId {
        let id = SubscriptionId(self.next_id);
        self.next_id += 1;
        self.entries.push(Subscriber {
            id,
            target_path,
            callback,
        });
        id
    }

    pub(crate) fn remove(&mut self, id: SubscriptionId) {
        self.entries.retain(|subscriber| subscriber.id != id);
    }

    pub(crate) fn publish(&mut self, event: &DocEvent) {
        for subscriber in &mut self.entries {
            match &subscriber.target_path {
                None => (subscriber.callback)(event),
                Some(target) => {
                    if let Some(event) = filter_by_path(event, target) {
                        (subscriber.callback)(&event);
                    }
                }
            }
        }
    }
}

impl Clone for Subscribers {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for Subscribers {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl fmt::Debug for Subscribers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Subscribers({})", self.entries.len())
    }
}

/// Keeps only the operations under `target` of a change event, as JS
/// `subscribe(targetPath, cb)` does. Other events and change events left
/// without operations are not delivered.
fn filter_by_path(event: &DocEvent, target: &str) -> Option<DocEvent> {
    let (info, remote) = match event {
        DocEvent::LocalChange(info) => (info, false),
        DocEvent::RemoteChange(info) => (info, true),
        _ => return None,
    };

    let operations: Vec<OperationInfo> = info
        .operations
        .iter()
        .filter(|op| is_same_element_or_child_of(op.path(), target))
        .cloned()
        .collect();
    if operations.is_empty() {
        return None;
    }

    let info = ChangeInfo {
        actor: info.actor.clone(),
        message: info.message.clone(),
        operations,
    };
    Some(if remote {
        DocEvent::RemoteChange(info)
    } else {
        DocEvent::LocalChange(info)
    })
}

/// Whether `elem` equals `parent` or lies below it, compared by `.`-separated
/// path segments (JS `isSameElementOrChildOf`).
fn is_same_element_or_child_of(elem: &str, parent: &str) -> bool {
    if elem == parent {
        return true;
    }

    let node_path: Vec<&str> = elem.split('.').collect();
    parent
        .split('.')
        .enumerate()
        .all(|(index, segment)| node_path.get(index) == Some(&segment))
}

/// Decodes a stored attribute value, falling back to the raw string when it
/// is not JSON (JS `parseAttrValue`).
pub(crate) fn parse_attr_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

pub(crate) fn parse_attrs(attributes: BTreeMap<String, String>) -> BTreeMap<String, Value> {
    attributes
        .into_iter()
        .map(|(key, value)| {
            let value = parse_attr_value(&value);
            (key, value)
        })
        .collect()
}

// Cases follow JS `test/unit/document/document_test.ts` ("changeInfo test for
// object/array", "check OpInfo type for subscribe path") and the presence
// transitions of JS `Document.reconcilePresence`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::ChangePack;
    use crate::operation::OpInfo;
    use crate::{Checkpoint, Document, JsonArray, JsonObject, Result};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Events = Arc<Mutex<Vec<DocEvent>>>;

    fn collect(doc: &mut Document) -> (SubscriptionId, Events) {
        let events: Events = Arc::default();
        let sink = events.clone();
        let id = doc.subscribe(move |event| sink.lock().unwrap().push(event.clone()));
        (id, events)
    }

    fn collect_path(doc: &mut Document, path: &str) -> Events {
        let events: Events = Arc::default();
        let sink = events.clone();
        doc.subscribe_path(path, move |event| sink.lock().unwrap().push(event.clone()));
        events
    }

    fn take(events: &Events) -> Vec<DocEvent> {
        std::mem::take(&mut *events.lock().unwrap())
    }

    fn actor(n: u8) -> ActorId {
        ActorId::from(format!("{n:024x}"))
    }

    fn attached_doc(n: u8) -> Document {
        let mut doc = Document::new("doc-key");
        doc.set_actor(actor(n));
        doc.apply_status(DocStatus::Attached);
        doc
    }

    fn presence_of(value: Value) -> Presence {
        value.as_object().unwrap().clone()
    }

    fn set_presence(doc: &mut Document, value: Value) -> Result<()> {
        doc.update_with_presence(|_, presence| {
            presence.set(presence_of(value));
            Ok(())
        })
    }

    /// Moves the changes `doc` has not handed over yet into `target`.
    fn deliver(doc: &mut Document, sent: &mut usize, target: &mut Document) -> Result<()> {
        let changes = doc.create_change_pack().changes()[*sent..].to_vec();
        *sent += changes.len();
        let pack = ChangePack::create("doc-key", Checkpoint::new(1, 0), false, changes, None, None);
        target.apply_change_pack(&pack)
    }

    fn set(path: &str, key: &str) -> OperationInfo {
        OperationInfo::Set {
            path: path.to_owned(),
            key: key.to_owned(),
        }
    }

    fn remove_key(path: &str, key: &str) -> OperationInfo {
        OperationInfo::Remove {
            path: path.to_owned(),
            key: Some(key.to_owned()),
            index: None,
        }
    }

    fn add(path: &str, index: usize) -> OperationInfo {
        OperationInfo::Add {
            path: path.to_owned(),
            index,
        }
    }

    fn local_ops(event: &DocEvent) -> &[OperationInfo] {
        match event {
            DocEvent::LocalChange(info) => &info.operations,
            other => panic!("expected a local change, got {other:?}"),
        }
    }

    #[test]
    fn local_change_event_carries_set_and_remove_ops() -> Result<()> {
        let mut doc = Document::new("test-doc");
        let (_, events) = collect(&mut doc);

        doc.update(|root| {
            root.set("", JsonObject::new())?;
            root.set("obj", JsonObject::new())?;
            let obj = root.get_object_mut("obj")?;
            obj.set("a", 1i32)?;
            obj.remove("a");
            obj.set("$hello", 1i32)?;
            obj.remove("$hello");
            root.remove("obj");
            Ok(())
        })?;

        let events = take(&events);
        assert_eq!(1, events.len());
        let DocEvent::LocalChange(info) = &events[0] else {
            panic!("expected a local change, got {:?}", events[0]);
        };
        assert_eq!(doc.actor_id(), &info.actor);
        assert_eq!(None, info.message);
        assert_eq!(
            vec![
                set("$", ""),
                set("$", "obj"),
                set("$.obj", "a"),
                remove_key("$.obj", "a"),
                set("$.obj", "$hello"),
                remove_key("$.obj", "$hello"),
                remove_key("$", "obj"),
            ],
            info.operations
        );
        Ok(())
    }

    #[test]
    fn array_remove_is_reported_as_remove_with_index() -> Result<()> {
        let mut doc = Document::new("test-doc");
        let (_, events) = collect(&mut doc);

        doc.update(|root| {
            root.set("arr", JsonArray::new())?;
            let arr = root.get_array_mut("arr")?;
            arr.push(0i32)?;
            arr.push(1i32)?;
            arr.remove(1);
            root.set("$$hello", JsonArray::new())?;
            root.get_array_mut("$$hello")?.push(0i32)?;
            Ok(())
        })?;

        let events = take(&events);
        assert_eq!(1, events.len());
        assert_eq!(
            &[
                set("$", "arr"),
                add("$.arr", 0),
                add("$.arr", 1),
                OperationInfo::Remove {
                    path: "$.arr".to_owned(),
                    key: None,
                    index: Some(1),
                },
                set("$", "$$hello"),
                add("$.$$hello", 0),
            ],
            local_ops(&events[0])
        );
        Ok(())
    }

    #[test]
    fn remote_change_event_after_apply_change_pack() -> Result<()> {
        let mut source = attached_doc(2);
        source.update(|root| root.set("title", "hello").map(|_| ()))?;
        let mut target = attached_doc(1);
        let (_, events) = collect(&mut target);

        deliver(&mut source, &mut 0, &mut target)?;

        assert_eq!(
            vec![DocEvent::RemoteChange(ChangeInfo {
                actor: actor(2),
                message: None,
                operations: vec![set("$", "title")],
            })],
            take(&events)
        );
        Ok(())
    }

    #[test]
    fn subscribe_path_filters_operations() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let mut todos = JsonArray::new();
            todos.push(JsonObject::new())?;
            root.set("todos", todos)?;
            root.set("todos2", JsonArray::new())?;
            root.set("other", JsonObject::new())?;
            Ok(())
        })?;
        let events = collect_path(&mut doc, "$.todos");

        doc.update(|root| {
            root.get_array_mut("todos")?.push(JsonObject::new())?;
            root.get_array_mut("todos")?
                .get_object_mut(0)?
                .set("done", true)?;
            root.get_array_mut("todos2")?.push(1i32)?;
            root.get_object_mut("other")?.set("x", 1i32)?;
            Ok(())
        })?;
        // Nothing under the path: the event is not delivered at all.
        doc.update(|root| root.get_object_mut("other")?.set("y", 1i32).map(|_| ()))?;

        let events = take(&events);
        assert_eq!(1, events.len());
        assert_eq!(
            &[add("$.todos", 1), set("$.todos.0", "done")],
            local_ops(&events[0])
        );
        Ok(())
    }

    #[test]
    fn path_matching_uses_whole_segments() {
        assert!(is_same_element_or_child_of("$.todos", "$.todos"));
        assert!(is_same_element_or_child_of("$.todos.0", "$.todos"));
        assert!(is_same_element_or_child_of("$.todos.0.text", "$.todos"));
        assert!(!is_same_element_or_child_of("$.todos2", "$.todos"));
        assert!(!is_same_element_or_child_of("$", "$.todos"));
        assert!(!is_same_element_or_child_of("$.other", "$.todos"));
    }

    #[test]
    fn unsubscribe_stops_delivery() -> Result<()> {
        let mut doc = Document::new("test-doc");
        let (id, events) = collect(&mut doc);
        let (_, kept) = collect(&mut doc);

        doc.update(|root| root.set("a", 1i32).map(|_| ()))?;
        doc.unsubscribe(id);
        doc.update(|root| root.set("b", 1i32).map(|_| ()))?;

        assert_eq!(1, take(&events).len());
        assert_eq!(2, take(&kept).len());
        Ok(())
    }

    #[test]
    fn status_changed_on_apply_status() -> Result<()> {
        let mut doc = Document::new("doc-key");
        let (_, events) = collect(&mut doc);

        doc.set_actor(actor(1));
        doc.apply_status(DocStatus::Attached);
        assert_eq!(
            vec![DocEvent::StatusChanged(DocStatus::Attached)],
            take(&events)
        );

        doc.apply_status(DocStatus::Detached);
        assert_eq!(
            vec![DocEvent::StatusChanged(DocStatus::Detached)],
            take(&events)
        );

        let removed = ChangePack::create(
            "doc-key",
            Checkpoint::new(1, 1),
            true,
            Vec::new(),
            None,
            None,
        );
        doc.apply_change_pack(&removed)?;
        assert_eq!(
            vec![DocEvent::StatusChanged(DocStatus::Removed)],
            take(&events)
        );
        Ok(())
    }

    #[test]
    fn attach_and_detach_report_the_self_presence_like_js() -> Result<()> {
        let mut doc = Document::new("doc-key");
        set_presence(&mut doc, json!({"a": 1}))?;
        doc.set_actor(actor(1));
        let (_, events) = collect(&mut doc);

        doc.apply_status(DocStatus::Attached);
        assert_eq!(
            vec![
                DocEvent::PresenceChanged(actor(1), presence_of(json!({"a": 1}))),
                DocEvent::StatusChanged(DocStatus::Attached),
            ],
            take(&events)
        );

        // JS reconciles the old actor after the reset, which is no longer
        // self, so it reports it as unwatched.
        doc.apply_status(DocStatus::Detached);
        assert_eq!(
            vec![
                DocEvent::Unwatched(actor(1), presence_of(json!({"a": 1}))),
                DocEvent::StatusChanged(DocStatus::Detached),
            ],
            take(&events)
        );
        Ok(())
    }

    #[test]
    fn local_presence_update_publishes_presence_changed() -> Result<()> {
        let mut doc = attached_doc(1);
        let (_, events) = collect(&mut doc);

        set_presence(&mut doc, json!({"name": "a"}))?;
        assert_eq!(
            vec![DocEvent::PresenceChanged(
                actor(1),
                presence_of(json!({"name": "a"}))
            )],
            take(&events)
        );

        doc.update_with_presence(|root, presence| {
            root.set("k", 1i32)?;
            presence.set(presence_of(json!({"n": 2})));
            Ok(())
        })?;
        let events = take(&events);
        assert_eq!(2, events.len());
        assert_eq!(&[set("$", "k")], local_ops(&events[0]));
        assert_eq!(
            DocEvent::PresenceChanged(actor(1), presence_of(json!({"name": "a", "n": 2}))),
            events[1]
        );
        Ok(())
    }

    #[test]
    fn local_presence_update_while_detached_is_silent() -> Result<()> {
        let mut doc = Document::new("doc-key");
        let (_, events) = collect(&mut doc);
        set_presence(&mut doc, json!({"name": "a"}))?;
        assert!(take(&events).is_empty());
        Ok(())
    }

    #[test]
    fn remote_presence_changes_from_an_online_actor() -> Result<()> {
        let mut source = attached_doc(2);
        let mut sent = 0;
        let mut target = attached_doc(1);
        target.add_online_client(actor(2));
        let (_, events) = collect(&mut target);

        // The first presence of an online actor makes it watched.
        set_presence(&mut source, json!({"name": "b"}))?;
        deliver(&mut source, &mut sent, &mut target)?;
        assert_eq!(
            vec![DocEvent::Watched(
                actor(2),
                presence_of(json!({"name": "b"}))
            )],
            take(&events)
        );

        set_presence(&mut source, json!({"name": "c"}))?;
        deliver(&mut source, &mut sent, &mut target)?;
        assert_eq!(
            vec![DocEvent::PresenceChanged(
                actor(2),
                presence_of(json!({"name": "c"}))
            )],
            take(&events)
        );

        // A clear takes the actor offline and reports its last presence.
        source.update_with_presence(|_, presence| {
            presence.clear();
            Ok(())
        })?;
        deliver(&mut source, &mut sent, &mut target)?;
        assert_eq!(
            vec![DocEvent::Unwatched(
                actor(2),
                presence_of(json!({"name": "c"}))
            )],
            take(&events)
        );
        assert!(target.others_presences().is_empty());
        Ok(())
    }

    #[test]
    fn remote_presence_from_an_offline_actor_is_silent() -> Result<()> {
        let mut source = attached_doc(2);
        set_presence(&mut source, json!({"name": "b"}))?;
        let mut target = attached_doc(1);
        let (_, events) = collect(&mut target);

        deliver(&mut source, &mut 0, &mut target)?;

        assert!(take(&events).is_empty());
        assert!(target.has_presence(&actor(2)));
        Ok(())
    }

    /// JS `applyWatchInit`: self is skipped, `Initialized` lists the online
    /// presences, and stale presences survive for a later reconnect.
    #[test]
    fn watch_init_sets_online_clients_and_publishes_initialized() -> Result<()> {
        let mut source = attached_doc(2);
        set_presence(&mut source, json!({"name": "b"}))?;
        let mut target = attached_doc(1);
        set_presence(&mut target, json!({"name": "a"}))?;
        deliver(&mut source, &mut 0, &mut target)?;
        let (_, events) = collect(&mut target);

        target.apply_watch_init([actor(1), actor(2), actor(3)]);
        assert_eq!(
            vec![DocEvent::Initialized(vec![
                (actor(1), presence_of(json!({"name": "a"}))),
                (actor(2), presence_of(json!({"name": "b"}))),
            ])],
            take(&events)
        );
        assert_eq!(None, target.presence(&actor(3)));

        target.apply_watch_init([]);
        assert_eq!(
            vec![DocEvent::Initialized(vec![(
                actor(1),
                presence_of(json!({"name": "a"}))
            )])],
            take(&events)
        );
        assert!(target.has_presence(&actor(2)));
        Ok(())
    }

    /// JS `applyDocEvent(DOCUMENT_WATCHED | DOCUMENT_UNWATCHED)`.
    #[test]
    fn watched_and_unwatched_follow_the_online_set() -> Result<()> {
        let mut source = attached_doc(2);
        set_presence(&mut source, json!({"name": "b"}))?;
        let mut target = attached_doc(1);
        let (_, events) = collect(&mut target);

        // Watched before the presence arrives is silent; the presence then
        // makes the client watched.
        target.apply_watched(actor(2));
        assert!(take(&events).is_empty());
        deliver(&mut source, &mut 0, &mut target)?;
        assert_eq!(
            vec![DocEvent::Watched(
                actor(2),
                presence_of(json!({"name": "b"}))
            )],
            take(&events)
        );

        // A duplicate watched event is ignored.
        target.apply_watched(actor(2));
        assert!(take(&events).is_empty());

        target.apply_unwatched(&actor(2));
        assert_eq!(
            vec![DocEvent::Unwatched(
                actor(2),
                presence_of(json!({"name": "b"}))
            )],
            take(&events)
        );
        assert!(target.has_presence(&actor(2)));

        // The kept presence makes a reconnect visible again.
        target.apply_watched(actor(2));
        assert_eq!(
            vec![DocEvent::Watched(
                actor(2),
                presence_of(json!({"name": "b"}))
            )],
            take(&events)
        );

        target.reset_online_clients();
        assert!(target.others_presences().is_empty());
        Ok(())
    }

    #[test]
    fn publish_delivers_client_events() {
        let mut doc = Document::new("doc-key");
        let (_, events) = collect(&mut doc);
        let path_events = collect_path(&mut doc, "$");

        doc.publish(DocEvent::ConnectionChanged(
            StreamConnectionStatus::Connected,
        ));
        doc.publish(DocEvent::SyncStatusChanged(DocSyncStatus::Synced));

        assert_eq!(
            vec![
                DocEvent::ConnectionChanged(StreamConnectionStatus::Connected),
                DocEvent::SyncStatusChanged(DocSyncStatus::Synced),
            ],
            take(&events)
        );
        assert!(take(&path_events).is_empty());
    }

    #[test]
    fn cloned_document_has_no_subscribers() -> Result<()> {
        let mut doc = Document::new("doc-key");
        let (_, events) = collect(&mut doc);
        let mut clone = doc.clone();
        assert_eq!(doc, clone);
        assert!(format!("{doc:?}").contains("Subscribers(1)"));
        assert!(format!("{clone:?}").contains("Subscribers(0)"));

        clone.update(|root| root.set("a", 1i32).map(|_| ()))?;
        assert!(take(&events).is_empty());
        Ok(())
    }

    #[test]
    fn converts_text_and_tree_op_infos_to_js_shape() {
        let attrs = BTreeMap::from([
            ("bold".to_owned(), "true".to_owned()),
            ("color".to_owned(), "red".to_owned()),
        ]);
        assert_eq!(
            OperationInfo::Edit {
                path: "$.text".to_owned(),
                from: 0,
                to: 1,
                value: EditOpValue {
                    attributes: BTreeMap::from([
                        ("bold".to_owned(), json!(true)),
                        ("color".to_owned(), json!("red")),
                    ]),
                    content: "a".to_owned(),
                },
            },
            OperationInfo::from(OpInfo::Edit {
                path: "$.text".to_owned(),
                from: 0,
                to: 1,
                content: "a".to_owned(),
                attributes: attrs,
            })
        );

        assert_eq!(
            OperationInfo::Style {
                path: "$.text".to_owned(),
                from: 0,
                to: 1,
                value: StyleOpValue {
                    attributes: BTreeMap::from([("bold".to_owned(), Value::Null)]),
                },
            },
            OperationInfo::from(OpInfo::Style {
                path: "$.text".to_owned(),
                from: 0,
                to: 1,
                attributes: BTreeMap::new(),
                attributes_to_remove: vec!["bold".to_owned()],
            })
        );

        assert_eq!(
            OperationInfo::TreeStyle {
                path: "$.tree".to_owned(),
                from: 0,
                to: 1,
                from_path: vec![0],
                to_path: vec![1],
                value: TreeStyleOpValue {
                    attributes: None,
                    attributes_to_remove: Some(vec!["bold".to_owned()]),
                },
            },
            OperationInfo::from(OpInfo::TreeStyle {
                path: "$.tree".to_owned(),
                from: 0,
                to: 1,
                from_path: vec![0],
                to_path: vec![1],
                attributes: BTreeMap::new(),
                attributes_to_remove: vec!["bold".to_owned()],
            })
        );
    }
}
