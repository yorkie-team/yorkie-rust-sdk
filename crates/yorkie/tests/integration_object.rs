//! Integration tests for objects, including object undo and redo.
//!
//! JS reference: `packages/sdk/test/integration/object_test.ts`. The cases
//! that edit a local document only run without a server. Run the rest with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.
//!
//! Several JS cases also compare the undo and redo stacks
//! (`getUndoStackForTest`, `getRedoStackForTest`), the change ID, or the
//! garbage length (`getGarbageLen`). The Rust public API exposes none of
//! these, so those assertions are left out and named in each case. An empty
//! redo stack is checked through `can_redo`.

mod integration_support;

use integration_support::two_clients::{active_client, manual_attach, shared, TwoClients};
use integration_support::{assert_undo_redo, json_value, rpc_addr, unique_key};
use serde_json::json;
use yorkie::{Client, DeactivateOptions, Document, JsonObject, JsonValue, SharedDocument};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn set(doc: &SharedDocument, key: &str, value: serde_json::Value) {
    doc.lock()
        .unwrap()
        .update(|root| root.set(key, json_value(value)).map(|_| ()))
        .unwrap();
}

fn delete(doc: &SharedDocument, key: &str) {
    doc.lock()
        .unwrap()
        .update(|root| {
            root.remove(key);
            Ok(())
        })
        .unwrap();
}

fn undo(doc: &SharedDocument) {
    doc.lock().unwrap().history_mut().undo().unwrap();
}

fn redo(doc: &SharedDocument) {
    doc.lock().unwrap().history_mut().redo().unwrap();
}

fn can_redo(doc: &SharedDocument) -> bool {
    doc.lock().unwrap().history().can_redo()
}

/// Two clients created and activated directly, as the JS undo cases do
/// instead of using `withTwoClientsAndDocuments`.
async fn two_clients(addr: &str) -> (Client, Client) {
    (active_client(addr).await, active_client(addr).await)
}

/// Like `two_clients`, but with client1 as the lower actor so that client2
/// wins a concurrent set, as the JS cases that need it arrange.
async fn two_clients_ordered(addr: &str) -> (Client, Client) {
    let (c1, c2) = two_clients(addr).await;
    if c1.actor_id() > c2.actor_id() {
        (c2, c1)
    } else {
        (c1, c2)
    }
}

async fn deactivate(clients: [Client; 2]) {
    for client in clients {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "valid key test" (object_test.ts:30): keys with a dot are rejected.
#[test]
fn rejects_keys_with_a_dot() {
    let mut doc = Document::new("test-doc");

    assert!(doc.update(|root| root.set(".", "dot").map(|_| ())).is_err());
    assert!(doc
        .update(|root| root.set("$...hello", "world").map(|_| ()))
        .is_err());
    assert!(doc
        .update(|root| {
            let mut nested = JsonObject::new();
            nested.set(".", "dot")?;
            root.set("", nested).map(|_| ())
        })
        .is_err());
}

/// JS "should apply updates inside nested map" (object_test.ts:52).
#[test]
fn applies_updates_inside_a_nested_map() {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| {
        root.set("k1", json_value(json!({"k1-1": "v1"})))?;
        root.get_object_mut("k1")?.set("k1-2", "v2")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":{"k1-1":"v1","k1-2":"v2"}}"#, doc.to_sorted_json());

    doc.update(|root| root.get_object_mut("k1")?.set("k1-2", "v3").map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"k1":{"k1-1":"v1","k1-2":"v3"}}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.set("k2", vec!["1", "2"])?;
        root.get_array_mut("k2")?.push("3")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        r#"{"k1":{"k1-1":"v1","k1-2":"v3"},"k2":["1","2","3"]}"#,
        doc.to_sorted_json()
    );

    let result = doc.update(|root| {
        root.get_array_mut("k2")?.push("4")?;
        Err(yorkie::YorkieError::InvalidArgument(
            "dummy error".to_owned(),
        ))
    });
    assert!(result.is_err());
    assert_eq!(
        r#"{"k1":{"k1-1":"v1","k1-2":"v3"},"k2":["1","2","3"]}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| root.get_array_mut("k2")?.push("4").map(|_| ()))
        .unwrap();
    assert_eq!(
        r#"{"k1":{"k1-1":"v1","k1-2":"v3"},"k2":["1","2","3","4"]}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| {
        root.get_array_mut("k2")?
            .push(json_value(json!({"k2-5": "v4"})))
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(
        r#"{"k1":{"k1-1":"v1","k1-2":"v3"},"k2":["1","2","3","4",{"k2-5":"v4"}]}"#,
        doc.to_sorted_json()
    );
}

/// JS "should handle delete operations" (object_test.ts:109).
#[test]
fn handles_delete_operations() {
    let mut doc = Document::new("test-doc");
    let mut states = Vec::new();
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| {
        root.set("k1", json_value(json!({"k1-1": "v1", "k1-2": "v2"})))
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"k1":{"k1-1":"v1","k1-2":"v2"}}"#, doc.to_sorted_json());
    states.push(doc.to_sorted_json());

    doc.update(|root| {
        let k1 = root.get_object_mut("k1")?;
        k1.remove("k1-1");
        k1.set("k1-3", "v4")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":{"k1-2":"v2","k1-3":"v4"}}"#, doc.to_sorted_json());
    states.push(doc.to_sorted_json());

    assert_undo_redo(&mut doc, &states);
}

/// JS "should support toJS and toJSON methods" (object_test.ts:132). The JS
/// `toJS` checks convert to plain JS objects, which have no Rust
/// counterpart; the JSON checks are ported.
#[test]
fn serializes_the_root_and_nested_objects() {
    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        root.set("content", json_value(json!({"a": 1, "b": 2, "c": 3})))
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"content":{"a":1,"b":2,"c":3}}"#, doc.to_sorted_json());

    let root = doc.get_root();
    assert_eq!(r#"{"content":{"a":1,"b":2,"c":3}}"#, root.to_sorted_json());
    assert_eq!(
        r#"{"a":1,"b":2,"c":3}"#,
        root.get("content").unwrap().to_sorted_json()
    );
}

/// JS "Object.keys, Object.values and Object.entries test"
/// (object_test.ts:151), through `JsonObject::iter`.
#[test]
fn iterates_keys_and_values() {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| {
        root.set("content", json_value(json!({"a": 1, "b": 2, "c": 3})))
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"content":{"a":1,"b":2,"c":3}}"#, doc.to_sorted_json());

    let Some(JsonValue::Object(content)) = doc.get_root().get("content") else {
        panic!("expected an object at content");
    };
    let keys: Vec<&str> = content.iter().map(|(key, _)| key).collect();
    let values: Vec<String> = content.iter().map(|(_, v)| v.to_sorted_json()).collect();
    let entries: Vec<String> = content
        .iter()
        .map(|(key, v)| format!("{key},{}", v.to_sorted_json()))
        .collect();
    assert_eq!("a,b,c", keys.join(","));
    assert_eq!("1,2,3", values.join(","));
    assert_eq!("a,1,b,2,c,3", entries.join(","));
}

/// JS "Can handle concurrent set/delete operations" (object_test.ts:168).
#[tokio::test]
async fn converges_after_concurrent_sets_and_deletes() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "object-concurrent-set-delete").await;

    set(&t.d1, "k1", json!("v1"));
    set(&t.d2, "k1", json!("v2"));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    set(&t.d1, "k2", json!("3"));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    set(&t.d1, "k2", json!("v2"));
    set(&t.d2, "k2", json!("v3"));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    set(&t.d1, "k3", json!("v4"));
    set(&t.d2, "k4", json!("v5"));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    delete(&t.d1, "k3");
    set(&t.d2, "k3", json!("v6"));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Returns undefined when looking up an element that doesnt exist"
/// (object_test.ts:228).
#[test]
fn finds_nothing_for_a_deleted_member() {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| {
        root.set(
            "shapes",
            json_value(json!({"circle": {"color": "black", "point": {"x": 0, "y": 0}}})),
        )
        .map(|_| ())
    })
    .unwrap();
    assert_eq!(
        r#"{"shapes":{"circle":{"color":"black","point":{"x":0,"y":0}}}}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| {
        root.get_object_mut("shapes")?.remove("circle");
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"shapes":{}}"#, doc.to_sorted_json());
    let Some(JsonValue::Object(shapes)) = doc.get_root().get("shapes") else {
        panic!("expected an object at shapes");
    };
    assert_eq!(None, shapes.get("circle"));
}

// --- Undo/Redo ---

/// JS "can get proper reverse operations" (object_test.ts:257). The undo
/// and redo stack comparisons are left out.
#[test]
fn undoes_and_redoes_object_sets() {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("shape", json_value(json!({"color": "black"})))
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, doc.to_sorted_json());

    doc.history_mut().undo().unwrap();
    assert_eq!("{}", doc.to_sorted_json());

    doc.history_mut().redo().unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, doc.to_sorted_json());
    doc.update(|root| {
        root.get_object_mut("shape")?
            .set("color", "red")
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"shape":{"color":"red"}}"#, doc.to_sorted_json());

    doc.history_mut().undo().unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, doc.to_sorted_json());

    doc.history_mut().redo().unwrap();
    assert_eq!(r#"{"shape":{"color":"red"}}"#, doc.to_sorted_json());
}

/// JS "Can undo/redo work properly for simple object" (object_test.ts:298).
#[test]
fn undoes_and_redoes_a_simple_object() {
    let mut doc = Document::new("test-doc");
    let can = |doc: &Document| (doc.history().can_undo(), doc.history().can_redo());
    assert_eq!("{}", doc.to_sorted_json());
    assert_eq!((false, false), can(&doc));

    doc.update(|root| root.set("a", 1).map(|_| ())).unwrap();
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    assert_eq!((true, false), can(&doc));

    doc.update(|root| root.set("a", 2).map(|_| ())).unwrap();
    assert_eq!(r#"{"a":2}"#, doc.to_sorted_json());
    assert_eq!((true, false), can(&doc));

    doc.history_mut().undo().unwrap();
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    assert_eq!((true, true), can(&doc));

    doc.history_mut().redo().unwrap();
    assert_eq!(r#"{"a":2}"#, doc.to_sorted_json());
    assert_eq!((true, false), can(&doc));

    doc.history_mut().undo().unwrap();
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    assert_eq!((true, true), can(&doc));

    doc.history_mut().undo().unwrap();
    assert_eq!("{}", doc.to_sorted_json());
    assert_eq!((false, true), can(&doc));
}

/// JS "Can undo/redo work properly for nested object" (object_test.ts:341).
#[test]
fn undoes_and_redoes_a_nested_object() {
    let mut doc = Document::new("test-doc");
    let mut states = vec![doc.to_sorted_json()];
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| {
        root.set(
            "shape",
            json_value(json!({"point": {"x": 0, "y": 0}, "color": "red"})),
        )?;
        root.set("a", 0)?;
        Ok(())
    })
    .unwrap();
    states.push(doc.to_sorted_json());
    assert_eq!(
        r#"{"a":0,"shape":{"color":"red","point":{"x":0,"y":0}}}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| {
        let shape = root.get_object_mut("shape")?;
        shape.set("point", json_value(json!({"x": 1, "y": 1})))?;
        shape.set("color", "blue")?;
        Ok(())
    })
    .unwrap();
    states.push(doc.to_sorted_json());
    assert_eq!(
        r#"{"a":0,"shape":{"color":"blue","point":{"x":1,"y":1}}}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| {
        root.set("a", 1)?;
        root.set("a", 2)?;
        Ok(())
    })
    .unwrap();
    states.push(doc.to_sorted_json());
    assert_eq!(
        r#"{"a":2,"shape":{"color":"blue","point":{"x":1,"y":1}}}"#,
        doc.to_sorted_json()
    );

    assert_undo_redo(&mut doc, &states);
}

/// JS "Should ensure convergence of peer's document after undoing nested
/// objects" (object_test.ts:383).
#[tokio::test]
async fn converges_after_undoing_a_nested_object() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-nested-converge");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc1, "shape", json!({"point": {"x": 0, "y": 0}}));
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"point":{"x":0,"y":0}}}"#, sorted_json(&doc1));

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"shape":{"point":{"x":0,"y":0}}}"#, sorted_json(&doc2));

    doc1.lock()
        .unwrap()
        .update(|root| {
            root.get_object_mut("shape")?
                .set("point", json_value(json!({"x": 1, "y": 1})))
                .map(|_| ())
        })
        .unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"point":{"x":1,"y":1}}}"#, sorted_json(&doc1));
    assert_eq!(r#"{"shape":{"point":{"x":1,"y":1}}}"#, sorted_json(&doc2));

    undo(&doc1);
    assert_eq!(r#"{"shape":{"point":{"x":0,"y":0}}}"#, sorted_json(&doc1));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"point":{"x":0,"y":0}}}"#, sorted_json(&doc2));

    deactivate([client1, client2]).await;
}

/// JS "Should handle reverse set operation for elements that other peers
/// deleted" (object_test.ts:427). The JS redo-stack length check is
/// `can_redo` here.
#[tokio::test]
async fn skips_an_undo_set_on_an_object_a_peer_deleted() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-set-deleted");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc1, "shape", json!({"color": "black"}));
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc1));

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc2));

    doc1.lock()
        .unwrap()
        .update(|root| {
            root.get_object_mut("shape")?
                .set("color", "red")
                .map(|_| ())
        })
        .unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    delete(&doc2, "shape");
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc1));
    assert_eq!("{}", sorted_json(&doc2));

    // c2 deleted the shape, so the reverse operation cannot be applied.
    undo(&doc1);
    assert_eq!("{}", sorted_json(&doc1));
    assert!(!can_redo(&doc1));

    deactivate([client1, client2]).await;
}

/// JS "Should handle reverse set operation for elements (nested objects)
/// that other peers deleted" (object_test.ts:476). The undo-stack
/// comparison is left out.
#[tokio::test]
async fn skips_an_undo_set_on_a_nested_object_a_peer_deleted() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-set-nested-deleted");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    set(
        &doc1,
        "shape",
        json!({"circle": {"point": {"x": 0, "y": 0}}}),
    );
    client1.sync(None).await.unwrap();
    assert_eq!(
        r#"{"shape":{"circle":{"point":{"x":0,"y":0}}}}"#,
        sorted_json(&doc1)
    );

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(
        r#"{"shape":{"circle":{"point":{"x":0,"y":0}}}}"#,
        sorted_json(&doc2)
    );

    doc1.lock()
        .unwrap()
        .update(|root| {
            root.get_object_mut("shape")?
                .get_object_mut("circle")?
                .set("point", json_value(json!({"x": 1, "y": 1})))
                .map(|_| ())
        })
        .unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(
        r#"{"shape":{"circle":{"point":{"x":1,"y":1}}}}"#,
        sorted_json(&doc1)
    );
    assert_eq!(
        r#"{"shape":{"circle":{"point":{"x":1,"y":1}}}}"#,
        sorted_json(&doc2)
    );
    delete(&doc2, "shape");
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc1));
    assert_eq!("{}", sorted_json(&doc2));

    undo(&doc1);
    assert_eq!("{}", sorted_json(&doc1));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc2));
    assert!(!can_redo(&doc1));

    deactivate([client1, client2]).await;
}

/// JS "Should handle reverse remove operation for elements that other peers
/// deleted" (object_test.ts:547).
#[tokio::test]
async fn skips_an_undo_remove_on_an_object_a_peer_deleted() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-remove-deleted");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc1, "shape", json!({"color": "black"}));
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc1));

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc2));

    delete(&doc2, "shape");
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc1));
    assert_eq!("{}", sorted_json(&doc2));

    // c2 deleted the shape, so the reverse operation cannot be applied.
    undo(&doc1);
    assert_eq!("{}", sorted_json(&doc1));
    assert!(!can_redo(&doc1));

    deactivate([client1, client2]).await;
}

/// JS "Should handle reverse remove operation for elements (nested
/// objects) that other peers deleted" (object_test.ts:591). The undo-stack
/// comparison is left out.
#[tokio::test]
async fn skips_an_undo_remove_on_a_nested_object_a_peer_deleted() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-remove-nested-deleted");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc1, "shape", json!({"circle": {"color": "red"}}));
    client1.sync(None).await.unwrap();
    assert_eq!(
        r#"{"shape":{"circle":{"color":"red"}}}"#,
        sorted_json(&doc1)
    );

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(
        r#"{"shape":{"circle":{"color":"red"}}}"#,
        sorted_json(&doc2)
    );

    doc1.lock()
        .unwrap()
        .update(|root| {
            root.get_object_mut("shape")?
                .get_object_mut("circle")?
                .set("point", json_value(json!({"x": 0, "y": 0})))
                .map(|_| ())
        })
        .unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(
        r#"{"shape":{"circle":{"color":"red","point":{"x":0,"y":0}}}}"#,
        sorted_json(&doc1)
    );
    assert_eq!(
        r#"{"shape":{"circle":{"color":"red","point":{"x":0,"y":0}}}}"#,
        sorted_json(&doc2)
    );
    delete(&doc2, "shape");
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc1));
    assert_eq!("{}", sorted_json(&doc2));

    undo(&doc1);
    assert_eq!("{}", sorted_json(&doc1));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc2));
    assert!(!can_redo(&doc1));

    deactivate([client1, client2]).await;
}

/// Asserts doc1's checkpoint as `(client_seq, server_seq)`.
fn assert_checkpoint(doc: &SharedDocument, client_seq: u32, server_seq: i64) {
    let checkpoint = doc.lock().unwrap().checkpoint();
    assert_eq!(
        (client_seq, server_seq),
        (checkpoint.client_seq(), checkpoint.server_seq())
    );
}

/// JS "Should not propagate changes when there is no applied undo
/// operation" (object_test.ts:656, and again, identical, at
/// object_test.ts:732). JS also checks the change ID's client seq, which
/// the Rust public API does not expose; it always equals the checkpoint's
/// client seq in this scenario, which is checked.
#[tokio::test]
async fn does_not_push_an_undo_that_applied_nothing() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-applied-nothing");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    assert_checkpoint(&doc1, 1, 1);

    set(&doc1, "shape", json!({"color": "black"}));
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc1));
    assert_checkpoint(&doc1, 2, 2);

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc2));

    delete(&doc2, "shape");
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc1));
    assert_eq!("{}", sorted_json(&doc2));
    assert_checkpoint(&doc1, 2, 4);

    // c2 deleted the shape, so the reverse operation cannot be applied.
    undo(&doc1);
    assert_eq!("{}", sorted_json(&doc1));
    assert!(!can_redo(&doc1));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    // Since no operation applied, the sequences do not change.
    assert_checkpoint(&doc1, 2, 4);

    set(&doc1, "shape", json!({"color": "red"}));
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"red"}}"#, sorted_json(&doc1));
    assert_checkpoint(&doc1, 3, 5);

    deactivate([client1, client2]).await;
}

/// JS "Can handle concurrent undo/redo: local undo & global redo"
/// (object_test.ts:808).
#[tokio::test]
async fn undoes_locally_and_redoes_globally() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-local-undo-global-redo");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc1, "color", json!("black"));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc2));

    set(&doc1, "color", json!("red"));
    set(&doc2, "color", json!("green"));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc2));

    undo(&doc1);
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc1)); // local undo
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc2));

    redo(&doc1);
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc1)); // global redo
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc2));

    deactivate([client1, client2]).await;
}

/// JS "Can properly apply remote operations that occurred before local
/// undo" (object_test.ts:858). JS attaches both documents with
/// `disableGC: true`, which is not ported; the case passes with collection
/// on.
#[tokio::test]
async fn applies_remote_sets_made_before_a_local_undo() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-remote-before-undo");
    let (client1, client2) = two_clients_ordered(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc1, "color", json!("black"));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc2));

    set(&doc1, "color", json!("red"));
    set(&doc2, "color", json!("green"));

    assert_eq!(r#"{"color":"red"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc2));

    undo(&doc1);
    assert_eq!(r#"{"color":"black"}"#, sorted_json(&doc1));

    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"color":"green"}"#, sorted_json(&doc2));

    redo(&doc1);
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"color":"red"}"#, sorted_json(&doc1));
    assert_eq!(r#"{"color":"red"}"#, sorted_json(&doc2));

    deactivate([client1, client2]).await;
}

/// JS "Should handle case of reverse operations referencing already
/// garbage-collected elements" (object_test.ts:923). JS attaches both
/// documents with `disableGC: true` (with a TODO to remove it). With
/// collection on, the undo's reverse operation names the replaced object
/// after it has been purged, and the undo fails with `MissingCrdtElement`.
#[tokio::test]
#[ignore = "disable-GC not ported: with collection on, the undo names a purged object, see docs/current-porting-gaps.md"]
async fn skips_an_undo_of_a_garbage_collected_object() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-gc-element");
    let (client1, client2) = two_clients_ordered(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();

    set(&doc1, "shape", json!({"color": "black"}));
    set(&doc2, "shape", json!({"color": "yellow"}));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"yellow"}}"#, sorted_json(&doc1));
    assert_eq!(r#"{"shape":{"color":"yellow"}}"#, sorted_json(&doc2));

    doc2.lock()
        .unwrap()
        .update(|root| {
            root.get_object_mut("shape")?
                .set("color", "red")
                .map(|_| ())
        })
        .unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"red"}}"#, sorted_json(&doc1));
    assert_eq!(r#"{"shape":{"color":"red"}}"#, sorted_json(&doc2));

    undo(&doc1);
    assert_eq!(r#"{"shape":{"color":"red"}}"#, sorted_json(&doc1));
    assert!(!can_redo(&doc1));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"red"}}"#, sorted_json(&doc2));

    deactivate([client1, client2]).await;
}

/// JS "Should clean up the references to a previously deleted node when the
/// deleted node is restored through undo" (object_test.ts:979).
#[tokio::test]
async fn restores_a_replaced_object_through_undo() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("object-undo-restore-replaced");
    let (client1, client2) = two_clients(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();

    set(&doc1, "shape", json!({"color": "black"}));
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc1));
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc2));

    set(&doc2, "shape", json!({"color": "yellow"}));
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"yellow"}}"#, sorted_json(&doc1));
    assert_eq!(r#"{"shape":{"color":"yellow"}}"#, sorted_json(&doc2));

    undo(&doc2);
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc1));
    assert_eq!(r#"{"shape":{"color":"black"}}"#, sorted_json(&doc2));

    // JS NOTE(chacha912): the removed set should only retain the entry for
    // `{shape: {color: 'yellow'}}`.
    assert_eq!(2, doc2.lock().unwrap().garbage_len());

    deactivate([client1, client2]).await;
}
