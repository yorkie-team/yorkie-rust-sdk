//! Integration tests for documents: attach and detach, change events,
//! removal, status changes, history, and the initial root.
//!
//! JS reference: `packages/sdk/test/integration/document_test.ts`. The cases
//! that edit a local document only run without a server. Run the rest with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::tree::{element, text};
use integration_support::two_clients::{active_client, manual_attach, shared};
use integration_support::{json_value, rpc_addr, unique_key, EventCollector};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use yorkie::{
    AttachOptions, ClientError, CounterValue, DeactivateOptions, DetachOptions, DocEvent,
    DocStatus, Document, EditOpValue, JsonArray, JsonCounter, JsonObject, JsonText, JsonTree,
    JsonValue, OperationInfo, SharedDocument, StyleOpValue, SyncMode, YorkieError,
};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn status(doc: &SharedDocument) -> DocStatus {
    doc.lock().unwrap().status()
}

fn update(doc: &SharedDocument, f: impl FnOnce(&mut JsonObject) -> yorkie::Result<()>) {
    doc.lock().unwrap().update(f).unwrap();
}

fn set(path: &str, key: &str) -> OperationInfo {
    OperationInfo::Set {
        path: path.to_owned(),
        key: key.to_owned(),
    }
}

fn attributes(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

/// A document whose root is set from `value` before it is attached, as a
/// JS test does with `doc.update` on a new document.
fn shared_with(key: &str, value: Value) -> SharedDocument {
    let doc = shared(key);
    update(&doc, |root| {
        let JsonValue::Object(object) = json_value(value) else {
            panic!("expected an object");
        };
        for (key, value) in object.iter() {
            root.set(key, value.clone())?;
        }
        Ok(())
    });
    doc
}

/// Records the operations of every change event (JS `doc.subscribe` with a
/// callback that skips snapshot events).
fn collect_changes(doc: &SharedDocument) -> EventCollector<(bool, Vec<OperationInfo>)> {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    doc.lock().unwrap().subscribe(move |event| match event {
        DocEvent::LocalChange(info) => sink((true, info.operations.clone())),
        DocEvent::RemoteChange(info) => sink((false, info.operations.clone())),
        _ => {}
    });
    collector
}

/// Records the operations of every change event under `path`.
fn collect_path(
    doc: &SharedDocument,
    path: &str,
) -> (EventCollector<Vec<OperationInfo>>, yorkie::SubscriptionId) {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    let id = doc.lock().unwrap().subscribe_path(path, move |event| {
        if let DocEvent::LocalChange(info) | DocEvent::RemoteChange(info) = event {
            sink(info.operations.clone());
        }
    });
    (collector, id)
}

/// Records every status change (JS `doc.subscribe('status', ...)`).
fn collect_status(doc: &SharedDocument) -> EventCollector<DocStatus> {
    let collector = EventCollector::new();
    let mut sink = collector.sink();
    doc.lock().unwrap().subscribe(move |event| {
        if let DocEvent::StatusChanged(status) = event {
            sink(*status);
        }
    });
    collector
}

/// JS "Can attach/detach documents" (document_test.ts:49).
#[tokio::test]
async fn attaches_and_detaches_documents() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-attach-detach");
    let client1 = active_client(&addr).await;
    let client2 = active_client(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    update(&doc1, |root| {
        root.set("k1", json_value(json!({"k1-1": "v1"})))?;
        root.set("k2", json_value(json!(["1", "2"])))?;
        Ok(())
    });
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":{"k1-1":"v1"},"k2":["1","2"]}"#, sorted_json(&doc1));

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"k1":{"k1-1":"v1"},"k2":["1","2"]}"#, sorted_json(&doc2));

    client1.detach(&doc1, DetachOptions).await.unwrap();
    client2.detach(&doc2, DetachOptions).await.unwrap();
    for client in [client1, client2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can watch documents" (document_test.ts:78).
#[tokio::test]
async fn watches_documents() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-watch");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let events1 = collect_changes(&d1);
    let events2 = collect_changes(&d2);

    update(&d2, |root| root.set("k1", "v1").map(|_| ()));

    let ops = vec![set("$", "k1")];
    events2.wait_and_verify_nth(1, (true, ops.clone())).await;
    events1.wait_and_verify_nth(1, (false, ops)).await;
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "detects the events from doc.subscribe" (document_test.ts:116).
#[tokio::test]
async fn detects_the_events_from_subscribe() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-subscribe-events");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let events1 = collect_changes(&d1);
    let events2 = collect_changes(&d2);

    let edit_attributes = json!({"italic": true, "objAttr": {"key1": {"key2": "value"}}});
    update(&d1, |root| {
        root.set("counter", JsonCounter::integer(100))?;
        root.set("todos", json_value(json!(["todo1", "todo2", "todo3"])))?;
        root.set("content", JsonText::new())?;
        root.get_text_mut("content")?.edit(
            0,
            0,
            "hello world",
            Some(attributes(edit_attributes.clone())),
        )?;
        root.set(
            "obj",
            json_value(json!({
                "name": "josh",
                "age": 14,
                "food": ["🍏", "🍇"],
                "score": {"english": 80, "math": 90},
            })),
        )?;
        let obj = root.get_object_mut("obj")?;
        obj.set("score", json_value(json!({"science": 100})))?;
        obj.remove("food");
        Ok(())
    });
    let expected = vec![
        set("$", "counter"),
        set("$", "todos"),
        set("$", "content"),
        OperationInfo::Edit {
            path: "$.content".to_owned(),
            from: 0,
            to: 0,
            value: EditOpValue {
                attributes: attributes(edit_attributes),
                content: "hello world".to_owned(),
            },
        },
        set("$", "obj"),
        set("$.obj", "score"),
        OperationInfo::Remove {
            path: "$.obj".to_owned(),
            key: Some("food".to_owned()),
            index: None,
        },
    ];
    events1
        .wait_and_verify_nth(1, (true, expected.clone()))
        .await;
    events2.wait_and_verify_nth(1, (false, expected)).await;

    update(&d2, |root| {
        root.get_counter_mut("counter")?.increase(1)?;
        let todos = root.get_array_mut("todos")?;
        todos.push("todo4")?;
        let prev = todos.get_element_by_index(1).unwrap().id().clone();
        let curr = todos.get_element_by_index(0).unwrap().id().clone();
        todos.move_after(&prev, &curr)?;
        root.get_text_mut("content")?
            .set_style(0, 5, attributes(json!({"bold": true})))?;
        Ok(())
    });
    let expected = vec![
        OperationInfo::Increase {
            path: "$.counter".to_owned(),
            value: CounterValue::Integer(1),
        },
        OperationInfo::Add {
            path: "$.todos".to_owned(),
            index: 3,
        },
        OperationInfo::Move {
            path: "$.todos".to_owned(),
            index: 1,
            previous_index: 0,
        },
        OperationInfo::Style {
            path: "$.content".to_owned(),
            from: 0,
            to: 5,
            value: StyleOpValue {
                attributes: attributes(json!({"bold": true})),
            },
        },
    ];
    events1
        .wait_and_verify_nth(2, (false, expected.clone()))
        .await;
    events2.wait_and_verify_nth(2, (true, expected)).await;
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "specify the topic to subscribe to" (document_test.ts:249).
#[tokio::test]
async fn subscribes_to_a_topic() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-subscribe-topic");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let all = collect_changes(&d1);
    let (todos, todos_id) = collect_path(&d1, "$.todos");
    let (counter, counter_id) = collect_path(&d1, "$.counter");
    let increase = |value| OperationInfo::Increase {
        path: "$.counter".to_owned(),
        value: CounterValue::Integer(value),
    };
    let add = |index| OperationInfo::Add {
        path: "$.todos".to_owned(),
        index,
    };

    update(&d2, |root| {
        root.set("counter", JsonCounter::integer(0))?;
        root.set("todos", json_value(json!(["todo1", "todo2"])))?;
        Ok(())
    });
    all.wait_and_verify_nth(1, (false, vec![set("$", "counter"), set("$", "todos")]))
        .await;

    update(&d2, |root| {
        root.get_counter_mut("counter")?.increase(10).map(|_| ())
    });
    all.wait_and_verify_nth(2, (false, vec![increase(10)]))
        .await;
    counter.wait_and_verify_nth(1, vec![increase(10)]).await;

    update(&d2, |root| {
        root.get_array_mut("todos")?.push("todo3").map(|_| ())
    });
    all.wait_and_verify_nth(3, (false, vec![add(2)])).await;
    todos.wait_and_verify_nth(1, vec![add(2)]).await;

    d1.lock().unwrap().unsubscribe(todos_id);
    update(&d2, |root| {
        root.get_array_mut("todos")?.push("todo4").map(|_| ())
    });
    all.wait_and_verify_nth(4, (false, vec![add(3)])).await;
    assert_eq!(1, todos.len()); // No events after unsubscribing `$.todos`

    d1.lock().unwrap().unsubscribe(counter_id);
    update(&d2, |root| {
        root.get_counter_mut("counter")?.increase(10).map(|_| ())
    });
    all.wait_and_verify_nth(5, (false, vec![increase(10)]))
        .await;
    assert_eq!(1, counter.len()); // No events after unsubscribing `$.counter`

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "specify the nested topic to subscribe to" (document_test.ts:332).
#[tokio::test]
async fn subscribes_to_a_nested_topic() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-subscribe-nested-topic");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let all = collect_changes(&d1);
    let (todos0, todos0_id) = collect_path(&d1, "$.todos.0");
    let (obj_c1, obj_c1_id) = collect_path(&d1, "$.obj.c1");

    update(&d2, |root| {
        root.set(
            "todos",
            json_value(json!([{"text": "todo1", "completed": false}])),
        )?;
        root.set(
            "obj",
            json_value(json!({"c1": {"name": "josh", "age": 14}})),
        )?;
        Ok(())
    });
    all.wait_and_verify_nth(1, (false, vec![set("$", "todos"), set("$", "obj")]))
        .await;

    update(&d2, |root| {
        root.get_object_mut("obj")?
            .get_object_mut("c1")?
            .set("name", "john")
            .map(|_| ())
    });
    all.wait_and_verify_nth(2, (false, vec![set("$.obj.c1", "name")]))
        .await;
    obj_c1
        .wait_and_verify_nth(1, vec![set("$.obj.c1", "name")])
        .await;

    update(&d2, |root| {
        root.get_array_mut("todos")?
            .get_object_mut(0)?
            .set("completed", true)
            .map(|_| ())
    });
    all.wait_and_verify_nth(3, (false, vec![set("$.todos.0", "completed")]))
        .await;
    todos0
        .wait_and_verify_nth(1, vec![set("$.todos.0", "completed")])
        .await;

    d1.lock().unwrap().unsubscribe(todos0_id);
    update(&d2, |root| {
        root.get_array_mut("todos")?
            .get_object_mut(0)?
            .set("text", "todo_1")
            .map(|_| ())
    });
    all.wait_and_verify_nth(4, (false, vec![set("$.todos.0", "text")]))
        .await;
    assert_eq!(1, todos0.len()); // No events after unsubscribing `$.todos.0`

    d1.lock().unwrap().unsubscribe(obj_c1_id);
    update(&d2, |root| {
        root.get_object_mut("obj")?
            .get_object_mut("c1")?
            .set("age", 15)
            .map(|_| ())
    });
    all.wait_and_verify_nth(5, (false, vec![set("$.obj.c1", "age")]))
        .await;
    assert_eq!(1, obj_c1.len()); // No events after unsubscribing `$.obj.c1`

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can handle tombstone" (document_test.ts:423). JS keeps the CRDT array
/// object the push ran on and checks `isRemoved()` on it; the Rust public
/// API exposes no CRDT element, so this checks that the replaced array's
/// elements are now garbage instead.
#[tokio::test]
async fn handles_a_tombstone() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-tombstone");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();

    update(&d1, |root| {
        root.set("k1", json_value(json!([1, 2]))).map(|_| ())
    });
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    update(&d1, |root| root.get_array_mut("k1")?.push(3).map(|_| ()));
    update(&d2, |root| root.set("k1", JsonArray::new()).map(|_| ()));
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!(sorted_json(&d1), sorted_json(&d2));
    // The pushed-to array is removed: it and its three elements.
    assert_eq!(4, d1.lock().unwrap().garbage_len());

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can remove document" (document_test.ts:460).
#[tokio::test]
async fn removes_a_document() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-remove");
    let d1 = shared(&key);
    let c1 = yorkie::Client::new(yorkie::ClientOptions {
        rpc_addr: addr.clone(),
        ..yorkie::ClientOptions::default()
    })
    .unwrap();

    // 01. client is not activated.
    let err = c1.remove(&d1).await.unwrap_err();
    assert!(matches!(err, ClientError::ClientNotActivated(_)), "{err:?}");

    // 02. document is not attached.
    c1.activate().await.unwrap();
    let err = c1.remove(&d1).await.unwrap_err();
    assert_eq!(ClientError::NotAttached(key.clone()), err);

    // 03. document is attached.
    let d1 = c1.attach(d1, AttachOptions::default()).await.unwrap();
    c1.remove(&d1).await.unwrap();

    // 04. try to update a removed document.
    let err = d1
        .lock()
        .unwrap()
        .update(|root| root.set("k1", json_value(json!([1, 2]))).map(|_| ()))
        .unwrap_err();
    assert_eq!(YorkieError::DocumentRemoved(key.clone()), err);

    // 05. try to attach a removed document.
    let err = c1
        .attach(d1.clone(), AttachOptions::default())
        .await
        .unwrap_err();
    assert_eq!(ClientError::NotDetached(key), err);

    c1.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// JS "Can create document with the same key as the removed document key"
/// (document_test.ts:513).
#[tokio::test]
async fn creates_a_document_with_the_key_of_a_removed_one() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-same-key");

    // 01. c1 creates d1 and removes it.
    let c1 = active_client(&addr).await;
    let d1 = shared_with(&key, json!({"k1": [1, 2]}));
    let d1 = c1.attach(d1, AttachOptions::default()).await.unwrap();
    assert_eq!(r#"{"k1":[1,2]}"#, sorted_json(&d1));
    c1.remove(&d1).await.unwrap();

    // 02. c2 creates d2 with the same key.
    let c2 = active_client(&addr).await;
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    // 02. c1 creates d2 with the same key.
    let d3 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    assert_eq!("{}", sorted_json(&d2));
    assert_eq!("{}", sorted_json(&d3));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// Attaches `{"k1":[1,2]}` to c1 and the same key to c2, both in manual
/// sync mode, as the JS removal cases start.
async fn attach_removal_pair(
    addr: &str,
    name: &str,
) -> (
    yorkie::Client,
    SharedDocument,
    yorkie::Client,
    SharedDocument,
) {
    let key = unique_key(name);
    // 01. c1 attaches d1 and c2 watches same doc.
    let c1 = active_client(addr).await;
    let d1 = c1
        .attach(shared_with(&key, json!({"k1": [1, 2]})), manual_attach())
        .await
        .unwrap();
    assert_eq!(r#"{"k1":[1,2]}"#, sorted_json(&d1));

    let c2 = active_client(addr).await;
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"k1":[1,2]}"#, sorted_json(&d2));
    (c1, d1, c2, d2)
}

/// JS "Can know that document has been removed when doing client.sync()"
/// (document_test.ts:546).
#[tokio::test]
async fn learns_of_a_removal_on_sync() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_removal_pair(&addr, "document-removed-sync").await;

    // 02. c1 updates d1 and removes it.
    update(&d1, |root| root.get_array_mut("k1")?.push(3).map(|_| ()));
    c1.remove(&d1).await.unwrap();
    assert_eq!(r#"{"k1":[1,2,3]}"#, sorted_json(&d1), "d1");
    assert_eq!(DocStatus::Removed, status(&d1));

    // 03. c2 syncs and checks that d2 is removed.
    c2.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":[1,2,3]}"#, sorted_json(&d2), "d2");
    assert_eq!(DocStatus::Removed, status(&d2));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can know that document has been removed when doing client.detach()"
/// (document_test.ts:585).
#[tokio::test]
async fn learns_of_a_removal_on_detach() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_removal_pair(&addr, "document-removed-detach").await;

    // 02. c1 removes d1 and c2 detaches d2.
    c1.remove(&d1).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(DocStatus::Removed, status(&d1));
    assert_eq!(DocStatus::Removed, status(&d2));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "removed document removal test" (document_test.ts:621).
#[tokio::test]
async fn removes_a_removed_document() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_removal_pair(&addr, "document-removed-removal").await;

    // 02. c1 removes d1 and c2 removes d2.
    c1.remove(&d1).await.unwrap();
    c2.remove(&d2).await.unwrap();
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(DocStatus::Removed, status(&d1));
    assert_eq!(DocStatus::Removed, status(&d2));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "document state transition test" (document_test.ts:662).
#[tokio::test]
async fn transitions_document_states() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("document-state-transition");
    let c1 = active_client(&addr).await;
    let not_attached = ClientError::NotAttached(key.clone());

    // 01. abnormal behavior on detached state
    let d1 = shared(&key);
    assert_eq!(
        not_attached,
        c1.detach(&d1, DetachOptions).await.unwrap_err()
    );
    assert_eq!(not_attached, c1.sync(Some(&d1)).await.unwrap_err());
    assert_eq!(not_attached, c1.remove(&d1).await.unwrap_err());

    // 02. abnormal behavior on attached state
    let d1 = c1.attach(d1, AttachOptions::default()).await.unwrap();
    assert_eq!(
        ClientError::NotDetached(key.clone()),
        c1.attach(d1.clone(), AttachOptions::default())
            .await
            .unwrap_err()
    );

    // 03. abnormal behavior on removed state
    c1.remove(&d1).await.unwrap();
    assert_eq!(not_attached, c1.remove(&d1).await.unwrap_err());
    assert_eq!(not_attached, c1.sync(Some(&d1)).await.unwrap_err());
    assert_eq!(
        not_attached,
        c1.detach(&d1, DetachOptions).await.unwrap_err()
    );

    c1.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// JS "Can subscribe to events related to document status changes"
/// (document_test.ts:729). The JS attached event also carries the actor;
/// the Rust `StatusChanged` event carries only the status.
#[tokio::test]
async fn publishes_document_status_changes() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("document-status-events");
    let d1 = shared(&key);
    let d2 = shared(&key);
    let events1 = collect_status(&d1);
    let events2 = collect_status(&d2);

    // 1. When the client attaches a document, it receives an attached event.
    let d1 = c1.attach(d1, AttachOptions::default()).await.unwrap();
    let _d2 = c2.attach(d2, AttachOptions::default()).await.unwrap();
    events1.wait_and_verify_nth(1, DocStatus::Attached).await;
    events2.wait_and_verify_nth(1, DocStatus::Attached).await;

    // 2. When c1 detaches a document, it receives a detached event.
    c1.detach(&d1, DetachOptions).await.unwrap();
    events1.wait_and_verify_nth(2, DocStatus::Detached).await;

    // 3. When c2 deactivates, it should also receive a detached event.
    c2.deactivate(DeactivateOptions::default()).await.unwrap();
    events2.wait_and_verify_nth(2, DocStatus::Detached).await;

    // 4. When other document is attached, it receives an attached event.
    let key2 = unique_key("document-status-events2");
    let d3 = shared(&key2);
    let d4 = shared(&key2);
    let events3 = collect_status(&d3);
    let events4 = collect_status(&d4);
    let d3 = c1.attach(d3, manual_attach()).await.unwrap();
    events3.wait_and_verify_nth(1, DocStatus::Attached).await;
    c2.activate().await.unwrap();
    let _d4 = c2.attach(d4, manual_attach()).await.unwrap();
    events4.wait_and_verify_nth(1, DocStatus::Attached).await;

    // 5. When c1 removes a document, it receives a removed event.
    c1.remove(&d3).await.unwrap();
    events3.wait_and_verify_nth(2, DocStatus::Removed).await;

    // 6. When c2 syncs, it should also receive a removed event.
    c2.sync(None).await.unwrap();
    events4.wait_and_verify_nth(2, DocStatus::Removed).await;

    // 7. If the document is in the removed state, a detached event should
    // not occur when deactivating.
    let (count3, count4) = (events3.len(), events4.len());
    c1.deactivate(DeactivateOptions::default()).await.unwrap();
    c2.deactivate(DeactivateOptions::default()).await.unwrap();
    assert_eq!(count3, events3.len());
    assert_eq!(count4, events4.len());
}

/// JS "Should properly handle removed document when deactivating"
/// (document_test.ts:828). The actor in the JS attached event is left out
/// as above.
#[tokio::test]
async fn handles_a_removed_document_when_deactivating() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("document-removed-deactivate");
    let d1 = shared(&key);
    let d2 = shared(&key);
    let events1 = collect_status(&d1);
    let events2 = collect_status(&d2);

    // 1. When the client attaches a document, it receives an attached event.
    let d1 = c1.attach(d1, manual_attach()).await.unwrap();
    let _d2 = c2.attach(d2, manual_attach()).await.unwrap();
    events1.wait_and_verify_nth(1, DocStatus::Attached).await;
    events2.wait_and_verify_nth(1, DocStatus::Attached).await;

    // 2. When c1 removes a document, it receives a removed event.
    c1.remove(&d1).await.unwrap();
    events1.wait_and_verify_nth(2, DocStatus::Removed).await;

    // 3. When c2 deactivates, it should also receive a removed event.
    c2.deactivate(DeactivateOptions::default()).await.unwrap();
    // JS NOTE(chacha912): For now, document status changes to `Detached`
    // when deactivating. This behavior may change in the future.
    events2.wait_and_verify_nth(2, DocStatus::Detached).await;

    c1.deactivate(DeactivateOptions::default()).await.unwrap();
}

fn counter_doc(initial: i32) -> Document {
    let mut doc = Document::new("document-history");
    doc.update(|root| {
        root.set("counter", JsonCounter::integer(initial))
            .map(|_| ())
    })
    .unwrap();
    doc
}

fn increase(doc: &mut Document) {
    doc.update(|root| root.get_counter_mut("counter")?.increase(1).map(|_| ()))
        .unwrap();
}

/// JS "Can canUndo/canRedo work properly" (document_test.ts:887).
#[test]
fn reports_whether_it_can_undo_and_redo() {
    let mut doc = counter_doc(100);
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    // user increases the counter
    increase(&mut doc);
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());

    // user can only undo the latest operation
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    // user undoes the latest operation
    doc.history_mut().undo().unwrap();
    assert!(doc.history().can_undo());
    assert!(doc.history().can_redo());

    // user redoes the latest undone operation
    doc.history_mut().redo().unwrap();
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());
}

/// JS "doc.update should clear redo stack" (document_test.ts:921).
#[test]
fn clears_the_redo_stack_on_update() {
    let mut doc = counter_doc(100);
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    for i in 0..5 {
        increase(&mut doc);
        assert_eq!(
            format!(r#"{{"counter":{}}}"#, 100 + i + 1),
            doc.to_sorted_json()
        );
    }
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    doc.history_mut().undo().unwrap();
    assert!(doc.history().can_undo());
    assert!(doc.history().can_redo());

    increase(&mut doc);
    // doc.update() clears redo stack
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());
}

/// JS "undo/redo with empty stack must throw error" (document_test.ts:955).
/// Its body checks that undo and redo on empty stacks are no-ops.
#[test]
fn undoes_and_redoes_nothing_on_empty_stacks() {
    let mut doc = Document::new("document-history-empty");
    doc.history_mut().undo().unwrap();
    doc.history_mut().redo().unwrap();
    assert_eq!("{}", doc.to_sorted_json());
}

/// JS "maximum undo/redo stack test" (document_test.ts:1002).
#[test]
fn caps_the_undo_and_redo_stacks() {
    let mut doc = counter_doc(0);
    assert_eq!(r#"{"counter":0}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    for _ in 0..100 {
        increase(&mut doc);
    }
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());

    for _ in 0..100 {
        doc.history_mut().undo().unwrap();
    }
    assert_eq!(r#"{"counter":50}"#, doc.to_sorted_json());

    for _ in 0..100 {
        doc.history_mut().redo().unwrap();
    }
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());
}

fn initial_root(value: impl IntoIterator<Item = (&'static str, JsonValue)>) -> AttachOptions {
    let mut root = JsonObject::new();
    for (key, value) in value {
        root.set(key, value).unwrap();
    }
    AttachOptions {
        initial_root: Some(root),
        ..AttachOptions::default()
    }
}

/// JS "Can attach with InitialRoot" (document_test.ts:1034).
#[tokio::test]
async fn attaches_with_an_initial_root() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("document-initial-root");

    // 01. attach and initialize document
    let doc1 = c1
        .attach(
            shared(&key),
            initial_root([
                ("counter", JsonValue::Counter(JsonCounter::integer(0))),
                ("content", json_value(json!({"x": 1, "y": 1}))),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(
        r#"{"content":{"x":1,"y":1},"counter":0}"#,
        sorted_json(&doc1)
    );
    c1.sync(None).await.unwrap();

    // 02. attach and initialize document with new fields and if key already
    // exists, it will be discarded
    let doc2 = c2
        .attach(
            shared(&key),
            initial_root([
                ("counter", JsonValue::Counter(JsonCounter::integer(1))),
                ("content", json_value(json!({"x": 1, "y": 2}))),
                ("new", json_value(json!({"k": "v"}))),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(
        r#"{"content":{"x":1,"y":1},"counter":0,"new":{"k":"v"}}"#,
        sorted_json(&doc2)
    );

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS `attachDocument` clears the history after the initial-root update
/// (`client.ts` 1850-1853), so the setup cannot be undone. No JS test checks
/// it; this one does.
#[tokio::test]
async fn an_initial_root_cannot_be_undone() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let key = unique_key("document-initial-root-undo");

    let doc = client
        .attach(shared(&key), initial_root([("k", JsonValue::from(1))]))
        .await
        .unwrap();

    assert_eq!(r#"{"k":1}"#, sorted_json(&doc));
    assert!(!doc.lock().unwrap().history().can_undo());
    assert!(!doc.lock().unwrap().history().can_redo());

    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "Can handle concurrent attach with InitialRoot" (document_test.ts:1073).
#[tokio::test]
async fn handles_concurrent_attaches_with_an_initial_root() {
    let Some(addr) = rpc_addr() else { return };
    let mut c1 = active_client(&addr).await;
    let mut c2 = active_client(&addr).await;
    // The concurrent-root conflict is decided by the actor tie-break; pin
    // c2 as the higher actor to keep it the winner this case expects.
    if c1.actor_id() > c2.actor_id() {
        std::mem::swap(&mut c1, &mut c2);
    }
    let key = unique_key("document-concurrent-initial-root");
    let manual_with = |writer: &str| AttachOptions {
        sync_mode: SyncMode::Manual,
        ..initial_root([("writer", JsonValue::String(writer.to_owned()))])
    };

    // 01. user1 attach with initialRoot and client doesn't sync
    let doc1 = c1.attach(shared(&key), manual_with("user1")).await.unwrap();
    assert_eq!(r#"{"writer":"user1"}"#, sorted_json(&doc1));

    // 02. user2 attach with initialRoot and client doesn't sync
    let doc2 = c2.attach(shared(&key), manual_with("user2")).await.unwrap();
    assert_eq!(r#"{"writer":"user2"}"#, sorted_json(&doc2));

    // 03. user1 sync first and user2 seconds
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();

    // 04. user1's local document's writer was user1
    assert_eq!(r#"{"writer":"user1"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"writer":"user2"}"#, sorted_json(&doc2));

    // 05. user1's local document's writer is overwritten by user2
    c1.sync(None).await.unwrap();
    assert_eq!(r#"{"writer":"user2"}"#, sorted_json(&doc1));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can support various types: <name>" (document_test.ts:1191, cases
/// from :1122). The `bytes` case is left out: the Rust `JsonValue` has no
/// bytes variant.
#[tokio::test]
async fn supports_various_types_in_an_initial_root() {
    let Some(addr) = rpc_addr() else { return };
    let cases: Vec<(&'static str, JsonValue, &str)> = vec![
        (
            "tree",
            JsonValue::Tree(JsonTree::new(element(
                "doc",
                vec![element("p", vec![text("ab")])],
            ))),
            r#"{"tree":{"type":"doc","children":[{"type":"p","children":[{"type":"text","value":"ab"}]}]}}"#,
        ),
        ("text", JsonValue::Text(JsonText::new()), r#"{"text":[]}"#),
        (
            "counter",
            JsonValue::Counter(JsonCounter::integer(1)),
            r#"{"counter":1}"#,
        ),
        ("null", JsonValue::Null, r#"{"null":null}"#),
        ("boolean", JsonValue::Bool(true), r#"{"boolean":true}"#),
        ("number", JsonValue::Integer(1), r#"{"number":1}"#),
        (
            "long",
            JsonValue::Long(9_223_372_036_854_775_807),
            r#"{"long":9223372036854775807}"#,
        ),
        (
            "object",
            json_value(json!({"k": "v"})),
            r#"{"object":{"k":"v"}}"#,
        ),
        ("array", json_value(json!([1, 2])), r#"{"array":[1,2]}"#),
    ];

    for (name, input, expected) in cases {
        let c1 = active_client(&addr).await;
        let key = unique_key(&format!("document-various-types-{name}"));
        let doc = c1
            .attach(shared(&key), initial_root([(name, input)]))
            .await
            .unwrap();
        assert_eq!(expected, sorted_json(&doc), "{name}");
        c1.deactivate(DeactivateOptions::default()).await.unwrap();
    }
}
