//! Integration tests for garbage collection.
//!
//! JS reference: `packages/sdk/test/integration/gc_test.ts`. Run with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.
//!
//! The JS cases check each document's version vector and garbage length
//! after every sync. `check` does both at once; the vector is written as JS
//! `vectorOf`, a list of actor and lamport pairs.

mod integration_support;

use integration_support::tree::{element, text, xml};
use integration_support::two_clients::{active_client, manual_attach, shared};
use integration_support::{max_vector_of, rpc_addr, unique_key, vector_of};
use serde_json::Value;
use std::collections::BTreeMap;
use yorkie::{
    ActorId, Client, DeactivateOptions, DetachOptions, JsonArray, JsonObject, JsonText, JsonValue,
    SharedDocument, SyncMode,
};

fn actor(client: &Client) -> ActorId {
    client.actor_id().expect("an active client has an actor")
}

/// Asserts `doc`'s version vector and garbage length, naming `step` on
/// failure.
#[track_caller]
fn check(step: &str, doc: &SharedDocument, vector: &[(&ActorId, i64)], garbage: usize) {
    let doc = doc.lock().unwrap();
    assert_eq!(&vector_of(vector), doc.version_vector(), "{step}: vector");
    assert_eq!(garbage, doc.garbage_len(), "{step}: garbage");
}

/// Asserts `doc`'s version vector alone, where JS checks only that.
#[track_caller]
fn check_vector(step: &str, doc: &SharedDocument, vector: &[(&ActorId, i64)]) {
    let doc = doc.lock().unwrap();
    assert_eq!(&vector_of(vector), doc.version_vector(), "{step}: vector");
}

#[track_caller]
fn garbage(doc: &SharedDocument) -> usize {
    doc.lock().unwrap().garbage_len()
}

fn update(doc: &SharedDocument, f: impl FnOnce(&mut JsonObject) -> yorkie::Result<()>) {
    doc.lock().unwrap().update(f).unwrap();
}

/// Two clients with a fresh document attached to each in manual sync mode,
/// both checked to start from an empty vector. Unlike `TwoClients`, the
/// clients keep the order JS creates them in: these cases never depend on
/// the actor tie-break.
async fn attach_two(addr: &str, name: &str) -> (Client, SharedDocument, Client, SharedDocument) {
    let key = unique_key(name);
    let c1 = active_client(addr).await;
    let c2 = active_client(addr).await;
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    check_vector("attach d1", &d1, &[]);
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();
    check_vector("attach d2", &d2, &[]);
    (c1, d1, c2, d2)
}

async fn detach_and_deactivate(pairs: Vec<(Client, SharedDocument)>) {
    for (client, doc) in pairs {
        if doc.lock().unwrap().is_attached() {
            client.detach(&doc, DetachOptions).await.unwrap();
        }
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "getGarbageLen should return the actual number of elements
/// garbage-collected" (gc_test.ts:31).
#[tokio::test]
async fn garbage_len_matches_what_garbage_collect_purges() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("gc-garbage-len");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    // 1. initial state
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    update(&d1, |root| {
        root.set("point", JsonObject::new())?;
        let point = root.get_object_mut("point")?;
        point.set("x", 0)?;
        point.set("y", 0)?;
        Ok(())
    });
    c1.sync(None).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();

    // 2. client1 updates doc
    update(&d1, |root| {
        root.remove("point");
        Ok(())
    });
    assert_eq!(3, garbage(&d1)); // point, x, y

    // 3. client2 updates doc
    update(&d2, |root| {
        root.get_object_mut("point")?.remove("x");
        Ok(())
    });
    assert_eq!(1, garbage(&d2)); // x

    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();

    let gc_node_len = 3; // point, x, y
    assert_eq!(gc_node_len, garbage(&d1));
    assert_eq!(gc_node_len, garbage(&d2));

    // Actual garbage-collected nodes
    let max = max_vector_of(&[&actor(&c1), &actor(&c2)]);
    assert_eq!(gc_node_len, d1.lock().unwrap().garbage_collect(&max));
    assert_eq!(gc_node_len, d2.lock().unwrap().garbage_collect(&max));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can handle tree garbage collection for multi client" (gc_test.ts:85).
#[tokio::test]
async fn collects_tree_garbage_for_multiple_clients() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-tree-multi").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        root.set_tree(
            "t",
            element(
                "doc",
                vec![element(
                    "p",
                    vec![
                        element("tn", vec![text("a"), text("b")]),
                        element("tn", vec![text("cd")]),
                    ],
                )],
            ),
        )
        .map(|_| ())
    });
    check("d1 sets", &d1, &[(&a1, 1)], 0);
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 1)]);

    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 1), (&a2, 2)]);

    update(&d2, |root| {
        root.get_tree_mut("t")?
            .edit_by_path(&[0, 0, 0], &[0, 0, 2], vec![text("gh")], 0)
    });
    check("d2 removes 2", &d2, &[(&a1, 1), (&a2, 3)], 2);
    assert_eq!(0, garbage(&d1));

    // (1, 1) -> (1, 2): syncedseqs:(0, 1)
    c2.sync(None).await.unwrap();
    check("c2 sync 2", &d2, &[(&a1, 1), (&a2, 3)], 2);
    assert_eq!(0, garbage(&d1));

    c1.sync(None).await.unwrap();
    check("c1 sync 2", &d1, &[(&a1, 4), (&a2, 3)], 2);
    assert_eq!(2, garbage(&d2));

    c2.sync(None).await.unwrap();
    check("c2 sync 3", &d2, &[(&a1, 1), (&a2, 3)], 2);
    assert_eq!(2, garbage(&d1));

    c1.sync(None).await.unwrap();
    check("c1 sync 3", &d1, &[(&a1, 4), (&a2, 3)], 0);
    assert_eq!(2, garbage(&d2));

    c2.sync(None).await.unwrap();
    check("c2 sync 4", &d2, &[(&a1, 1), (&a2, 3)], 0);
    assert_eq!(0, garbage(&d1));

    detach_and_deactivate(vec![(c1, d1), (c2, d2)]).await;
}

/// JS "Can handle garbage collection for container type" (gc_test.ts:222).
#[tokio::test]
async fn collects_container_garbage() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-container").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        root.set("1", 1)?;
        root.set("2", JsonArray::new())?;
        root.get_array_mut("2")?.push(1)?.push(2)?.push(3)?;
        root.set("3", 3)?;
        Ok(())
    });
    check("d1 sets", &d1, &[(&a1, 1)], 0);
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 1)]);

    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 1), (&a2, 2)]);

    update(&d2, |root| {
        root.remove("2");
        Ok(())
    });
    check("d2 removes 2", &d2, &[(&a1, 1), (&a2, 3)], 4);
    assert_eq!(0, garbage(&d1));

    c2.sync(None).await.unwrap();
    check("c2 sync 2", &d2, &[(&a1, 1), (&a2, 3)], 4);
    assert_eq!(0, garbage(&d1));

    c1.sync(None).await.unwrap();
    check("c1 sync 2", &d1, &[(&a1, 4), (&a2, 3)], 4);
    assert_eq!(4, garbage(&d2));

    c2.sync(None).await.unwrap();
    check("c2 sync 3", &d2, &[(&a1, 1), (&a2, 3)], 4);
    assert_eq!(4, garbage(&d1));

    c1.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    check("c1 sync 3", &d1, &[(&a1, 4), (&a2, 3)], 0);
    assert_eq!(4, garbage(&d2));

    c2.sync(None).await.unwrap();
    check("c2 sync 4", &d2, &[(&a1, 1), (&a2, 3)], 0);
    assert_eq!(0, garbage(&d1));

    detach_and_deactivate(vec![(c1, d1), (c2, d2)]).await;
}

/// JS "Can handle garbage collection for text type" (gc_test.ts:345).
#[tokio::test]
async fn collects_text_garbage() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-text").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        root.set("text", JsonText::new())?;
        root.get_text_mut("text")?.edit(0, 0, "Hello World", None)?;
        root.set("textWithAttr", JsonText::new())?;
        root.get_text_mut("textWithAttr")?
            .edit(0, 0, "Hello World", None)?;
        Ok(())
    });
    check("d1 sets", &d1, &[(&a1, 1)], 0);
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 1)]);

    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 1), (&a2, 2)]);

    update(&d2, |root| {
        let text = root.get_text_mut("text")?;
        text.edit(0, 1, "a", None)?;
        text.edit(1, 2, "b", None)?;
        let b = BTreeMap::from([("b".to_owned(), Value::from("1"))]);
        root.get_text_mut("textWithAttr")?
            .edit(0, 1, "a", Some(b))?;
        Ok(())
    });
    check("d2 edits", &d2, &[(&a1, 1), (&a2, 3)], 3);
    assert_eq!(0, garbage(&d1));

    c2.sync(None).await.unwrap();
    check("c2 sync 2", &d2, &[(&a1, 1), (&a2, 3)], 3);
    assert_eq!(0, garbage(&d1));

    c1.sync(None).await.unwrap();
    check("c1 sync 2", &d1, &[(&a1, 4), (&a2, 3)], 3);
    assert_eq!(3, garbage(&d2));

    c2.sync(None).await.unwrap();
    check("c2 sync 3", &d2, &[(&a1, 1), (&a2, 3)], 3);
    assert_eq!(3, garbage(&d1));

    c1.sync(None).await.unwrap();
    check("c1 sync 3", &d1, &[(&a1, 4), (&a2, 3)], 0);
    assert_eq!(3, garbage(&d2));

    c2.sync(None).await.unwrap();
    check("c2 sync 4", &d2, &[(&a1, 1), (&a2, 3)], 0);
    assert_eq!(0, garbage(&d1));

    detach_and_deactivate(vec![(c1, d1), (c2, d2)]).await;
}

fn bold(value: &str) -> Option<BTreeMap<String, Value>> {
    Some(BTreeMap::from([("b".to_owned(), Value::from(value))]))
}

/// JS "Can handle garbage collection with detached document test"
/// (gc_test.ts:467).
#[tokio::test]
async fn collects_garbage_after_a_peer_detaches() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-detached").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        root.set("1", 1)?;
        root.set("2", JsonArray::new())?;
        root.get_array_mut("2")?.push(1)?.push(2)?.push(3)?;
        root.set("3", 3)?;
        root.set_text("4")?.edit(0, 0, "hi", None)?;
        root.set_text("5")?.edit(0, 0, "hi", None)?;
        Ok(())
    });
    check("d1 sets", &d1, &[(&a1, 1)], 0);
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 1)]);

    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 1), (&a2, 2)]);

    update(&d1, |root| {
        root.remove("2");
        root.get_text_mut("4")?.edit(0, 1, "h", None)?;
        root.get_text_mut("5")?.edit(0, 1, "h", bold("1"))?;
        Ok(())
    });
    check("d1 removes", &d1, &[(&a1, 2)], 6);
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check("c1 sync 2", &d1, &[(&a1, 2)], 6);
    assert_eq!(0, garbage(&d2));

    c2.detach(&d2, DetachOptions).await.unwrap();

    c2.sync(None).await.unwrap();
    assert_eq!(6, garbage(&d1));
    assert_eq!(6, garbage(&d2));

    c1.sync(None).await.unwrap();
    // JS TODO(JOOHOJANG): we have to consider removing detached client's
    // lamport from version vector
    check("c1 sync 3", &d1, &[(&a1, 2)], 0);
    assert_eq!(6, garbage(&d2));

    detach_and_deactivate(vec![(c1, d1), (c2, d2)]).await;
}

/// JS "Can collect removed elements from both root and clone"
/// (gc_test.ts:565). JS also checks `getGarbageLenFromClone`; a Rust
/// document keeps no clone (a local change runs on a copy that replaces the
/// root), so only the root is checked.
#[tokio::test]
async fn collects_removed_elements_from_the_root() {
    let Some(addr) = rpc_addr() else { return };
    let cli = active_client(&addr).await;
    let a = actor(&cli);
    let doc = cli
        .attach(shared(&unique_key("gc-root-clone")), manual_attach())
        .await
        .unwrap();
    check_vector("attach", &doc, &[]);

    for (n, lamport) in [(0, 1), (1, 2), (2, 3)] {
        update(&doc, |root| {
            root.set("point", JsonObject::new())?;
            let point = root.get_object_mut("point")?;
            point.set("x", n)?;
            point.set("y", n)?;
            Ok(())
        });
        check_vector(&format!("point {n}"), &doc, &[(&a, lamport)]);
    }
    assert_eq!(6, garbage(&doc));

    detach_and_deactivate(vec![(cli, doc)]).await;
}

/// JS "Can collect removed elements from both root and clone for nested
/// array" (gc_test.ts:601). The clone check is left out as above.
#[tokio::test]
async fn collects_removed_elements_from_the_root_for_a_nested_array() {
    let Some(addr) = rpc_addr() else { return };
    let cli = active_client(&addr).await;
    let a = actor(&cli);
    let doc = cli
        .attach(shared(&unique_key("gc-root-clone-nested")), manual_attach())
        .await
        .unwrap();
    check_vector("attach", &doc, &[]);

    update(&doc, |root| {
        root.set("list", JsonArray::new())?;
        let list = root.get_array_mut("list")?;
        list.push(0)?.push(1)?.push(2)?;
        let mut nested = JsonArray::new();
        nested.push(3)?.push(4)?.push(5)?;
        list.push(nested)?;
        Ok(())
    });
    check_vector("sets", &doc, &[(&a, 1)]);
    assert_eq!(
        r#"{"list":[0,1,2,[3,4,5]]}"#,
        doc.lock().unwrap().to_sorted_json()
    );

    update(&doc, |root| {
        root.get_array_mut("list")?.delete(1);
        Ok(())
    });
    check_vector("delete list[1]", &doc, &[(&a, 2)]);
    assert_eq!(
        r#"{"list":[0,2,[3,4,5]]}"#,
        doc.lock().unwrap().to_sorted_json()
    );

    update(&doc, |root| {
        root.get_array_mut("list")?.get_array_mut(2)?.delete(1);
        Ok(())
    });
    check_vector("delete list[2][1]", &doc, &[(&a, 3)]);
    assert_eq!(
        r#"{"list":[0,2,[3,5]]}"#,
        doc.lock().unwrap().to_sorted_json()
    );
    assert_eq!(2, garbage(&doc));

    detach_and_deactivate(vec![(cli, doc)]).await;
}

/// JS "Can purges removed elements after peers can not access them"
/// (gc_test.ts:642).
#[tokio::test]
async fn purges_removed_elements_once_no_peer_can_reach_them() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("gc-purge-after-peers");
    let client1 = active_client(&addr).await;
    let client2 = active_client(&addr).await;
    let (a1, a2) = (actor(&client1), actor(&client2));

    let d1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    check_vector("attach d1", &d1, &[]);
    let set_point = |doc: &SharedDocument, n: i32| {
        update(doc, |root| {
            root.set("point", JsonObject::new())?;
            let point = root.get_object_mut("point")?;
            point.set("x", n)?;
            point.set("y", n)?;
            Ok(())
        })
    };
    set_point(&d1, 0);
    check_vector("d1 point", &d1, &[(&a1, 1)]);
    update(&d1, |root| {
        root.get_object_mut("point")?.set("x", 1).map(|_| ())
    });
    check("d1 x=1", &d1, &[(&a1, 2)], 1);
    client1.sync(None).await.unwrap();
    check("client1 sync", &d1, &[(&a1, 2)], 0);

    let d2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    check("attach d2", &d2, &[(&a1, 2), (&a2, 3)], 1);
    update(&d2, |root| {
        root.get_object_mut("point")?.set("x", 2).map(|_| ())
    });
    check("d2 x=2", &d2, &[(&a1, 2), (&a2, 4)], 2);

    set_point(&d1, 3);
    check("d1 point 3", &d1, &[(&a1, 3)], 3);
    client1.sync(None).await.unwrap();
    check("client1 sync 2", &d1, &[(&a1, 3)], 3);

    client1.sync(None).await.unwrap();
    check("client1 sync 3", &d1, &[(&a1, 3)], 3);

    client2.sync(None).await.unwrap();
    {
        let d2 = d2.lock().unwrap();
        assert_eq!(&vector_of(&[(&a1, 3), (&a2, 5)]), d2.version_vector());
    }
    assert_eq!(3, garbage(&d1));
    client1.sync(None).await.unwrap();
    check("client1 sync 4", &d1, &[(&a1, 5), (&a2, 4)], 4);
    client2.sync(None).await.unwrap();
    assert_eq!(4, garbage(&d1));
    client1.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d1));
    client2.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d2));

    for client in [client1, client2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "garbage collection test for nested object" (gc_test.ts:744).
#[test]
fn collects_a_nested_object() {
    let mut doc = yorkie::Document::new("gc-nested-object");
    doc.update(|root| {
        root.set("shape", JsonObject::new())?;
        let shape = root.get_object_mut("shape")?;
        shape.set("point", JsonObject::new())?;
        let point = shape.get_object_mut("point")?;
        point.set("x", 0)?;
        point.set("y", 0)?;
        root.remove("shape");
        Ok(())
    })
    .unwrap();
    assert_eq!(4, doc.garbage_len()); // shape, point, x, y
    let actor = doc.actor_id().clone();
    // The number of GC nodes must also be 4.
    assert_eq!(4, doc.garbage_collect(&max_vector_of(&[&actor])));
}

fn text_of(doc: &SharedDocument) -> String {
    match doc.lock().unwrap().get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

fn edit_text(doc: &SharedDocument, from: usize, to: usize, content: &str) {
    update(doc, |root| {
        root.get_text_mut("t")?
            .edit(from, to, content, None)
            .map(|_| ())
    });
}

fn edit_tree_by_path(doc: &SharedDocument, from: &[usize], to: &[usize], content: Option<&str>) {
    update(doc, |root| {
        root.get_tree_mut("t")?
            .edit_by_path(from, to, content.map(text).into_iter().collect(), 0)
    });
}

/// JS "Should work properly when there are multiple nodes to be collected
/// in text type" (gc_test.ts:760).
#[tokio::test]
async fn collects_several_text_nodes() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-text-multi").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        root.set_text("t")?.edit(0, 0, "z", None).map(|_| ())
    });
    check_vector("z", &d1, &[(&a1, 1)]);
    edit_text(&d1, 0, 1, "a");
    check_vector("a", &d1, &[(&a1, 2)]);
    edit_text(&d1, 1, 1, "b");
    check_vector("b", &d1, &[(&a1, 3)]);
    edit_text(&d1, 2, 2, "d");
    check_vector("d", &d1, &[(&a1, 4)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 4)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 4), (&a2, 5)]);
    assert_eq!("abd", text_of(&d1));
    assert_eq!("abd", text_of(&d2));
    assert_eq!(1, garbage(&d1)); // z

    edit_text(&d1, 2, 2, "c");
    check_vector("c", &d1, &[(&a1, 5)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 2", &d1, &[(&a1, 5)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 2", &d2, &[(&a1, 5), (&a2, 6)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 3", &d1, &[(&a1, 5)]);
    assert_eq!("abcd", text_of(&d1));
    assert_eq!("abcd", text_of(&d2));

    edit_text(&d1, 1, 3, "");
    check_vector("remove", &d1, &[(&a1, 6)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 4", &d1, &[(&a1, 6)]);
    assert_eq!("ad", text_of(&d1));
    assert_eq!(2, garbage(&d1)); // b,c

    c2.sync(None).await.unwrap();
    check_vector("c2 sync 3", &d2, &[(&a1, 6), (&a2, 7)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 5", &d1, &[(&a1, 6)]);
    c2.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d2));
    assert_eq!("ad", text_of(&d2));
    c1.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d1));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Should work properly when there are multiple nodes to be collected
/// in tree type" (gc_test.ts:889).
#[tokio::test]
async fn collects_several_tree_nodes() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-tree-multi-nodes").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        root.set_tree("t", element("r", vec![text("z")]))
            .map(|_| ())
    });
    check_vector("z", &d1, &[(&a1, 1)]);
    edit_tree_by_path(&d1, &[0], &[1], Some("a"));
    check_vector("a", &d1, &[(&a1, 2)]);
    edit_tree_by_path(&d1, &[1], &[1], Some("b"));
    check_vector("b", &d1, &[(&a1, 3)]);
    edit_tree_by_path(&d1, &[2], &[2], Some("d"));
    check_vector("d", &d1, &[(&a1, 4)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 4)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 4), (&a2, 5)]);
    assert_eq!("<r>abd</r>", xml(&d1));
    assert_eq!("<r>abd</r>", xml(&d2));
    assert_eq!(1, garbage(&d1)); // z

    edit_tree_by_path(&d1, &[2], &[2], Some("c"));
    check_vector("c", &d1, &[(&a1, 5)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 2", &d1, &[(&a1, 5)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 2", &d2, &[(&a1, 5), (&a2, 6)]);
    assert_eq!("<r>abcd</r>", xml(&d1));
    assert_eq!("<r>abcd</r>", xml(&d2));

    edit_tree_by_path(&d1, &[1], &[3], None);
    check_vector("remove", &d1, &[(&a1, 6)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 3", &d1, &[(&a1, 6)]);
    assert_eq!("<r>ad</r>", xml(&d1));
    assert_eq!(2, garbage(&d1)); // b,c

    c2.sync(None).await.unwrap();
    check_vector("c2 sync 3", &d2, &[(&a1, 6), (&a2, 7)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 4", &d1, &[(&a1, 6)]);
    assert_eq!("<r>ad</r>", xml(&d2));
    assert_eq!(2, garbage(&d1));

    c2.sync(None).await.unwrap();
    check_vector("c2 sync 4", &d2, &[(&a1, 6), (&a2, 7)]);
    assert_eq!("<r>ad</r>", xml(&d2));
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 5", &d1, &[(&a1, 6)]);
    assert_eq!("<r>ad</r>", xml(&d1));
    assert_eq!(0, garbage(&d1));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "concurrent garbage collection test" (gc_test.ts:1047).
#[tokio::test]
async fn collects_garbage_with_concurrent_edits() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-concurrent").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    update(&d1, |root| {
        let t = root.set_text("t")?;
        t.edit(0, 0, "a", None)?;
        t.edit(1, 1, "b", None)?;
        t.edit(2, 2, "c", None)?;
        Ok(())
    });
    check_vector("sets text", &d1, &[(&a1, 1)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 1)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 1), (&a2, 2)]);

    edit_text(&d2, 2, 2, "c");
    check_vector("insert c", &d2, &[(&a1, 1), (&a2, 3)]);

    edit_text(&d1, 1, 3, "");
    check("delete bd", &d1, &[(&a1, 2)], 2);
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 2", &d1, &[(&a1, 2)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 2", &d2, &[(&a1, 2), (&a2, 4)]);
    assert_eq!(2, garbage(&d1));
    assert_eq!(2, garbage(&d2));

    edit_text(&d2, 2, 2, "1");
    check_vector("insert 1", &d2, &[(&a1, 2), (&a2, 5)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 3", &d2, &[(&a1, 2), (&a2, 5)]);
    assert_eq!(2, garbage(&d1));
    // "b" and "c" meet the GC condition, but one of them is still held by
    // the successor barrier: purging it would move the stopping point of a
    // forward skip that c2's concurrent insert still depends on. It drains on
    // the next round, which `drains_a_successor_barrier_within_one_round`
    // pins.
    assert_eq!(1, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 3", &d1, &[(&a1, 6), (&a2, 5)]);
    // One tombstone each is still held by the successor barrier; see above.
    assert_eq!(1, garbage(&d1));
    assert_eq!(1, garbage(&d2));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "concurrent garbage collection test(with pushonly)" (gc_test.ts:1173).
#[tokio::test]
async fn collects_garbage_with_concurrent_edits_while_push_only() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-concurrent-pushonly").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    // d1/vv = [c1:1]
    update(&d1, |root| {
        let t = root.set_text("t")?;
        t.edit(0, 0, "a", None)?;
        t.edit(1, 1, "b", None)?;
        Ok(())
    });
    check_vector("insert ab", &d1, &[(&a1, 1)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync", &d1, &[(&a1, 1)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync", &d2, &[(&a1, 1), (&a2, 2)]);

    // d2/vv = [c1:1, c2:3]
    edit_text(&d2, 2, 2, "d");
    check_vector("insert d", &d2, &[(&a1, 1), (&a2, 3)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 2", &d2, &[(&a1, 1), (&a2, 3)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 2", &d1, &[(&a1, 4), (&a2, 3)]);

    // d2/vv = [c1:1, c2:4]
    edit_text(&d2, 2, 2, "c");
    check_vector("insert c", &d2, &[(&a1, 1), (&a2, 4)]);

    // c1/vv = [c1:5, c2:3]
    edit_text(&d1, 1, 3, "");
    check_vector("remove ac", &d1, &[(&a1, 5), (&a2, 3)]);

    // Sync with PushOnly
    c2.change_sync_mode(&d2, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    c2.sync(None).await.unwrap();
    check_vector("c2 push-only sync", &d2, &[(&a1, 1), (&a2, 4)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 3", &d1, &[(&a1, 6), (&a2, 4)]);

    // d2/vv = [c1:1, c2:5]
    edit_text(&d2, 2, 2, "1");
    check_vector("insert 1 (pushonly)", &d2, &[(&a1, 1), (&a2, 5)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 push-only sync 2", &d2, &[(&a1, 1), (&a2, 5)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 4", &d1, &[(&a1, 7), (&a2, 5)]);
    assert_eq!(2, garbage(&d1));
    assert_eq!(0, garbage(&d2));

    // c2/vv = [c1:1, c2:6]
    edit_text(&d2, 2, 2, "2");
    check_vector("insert 2 (pushonly)", &d2, &[(&a1, 1), (&a2, 6)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 push-only sync 3", &d2, &[(&a1, 1), (&a2, 6)]);
    c2.change_sync_mode(&d2, SyncMode::Manual).await.unwrap();
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 3", &d2, &[(&a1, 5), (&a2, 7)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync 5", &d1, &[(&a1, 8), (&a2, 6)]);
    assert_eq!(2, garbage(&d1));
    assert_eq!(2, garbage(&d2));

    c2.sync(None).await.unwrap();
    check_vector("c2 sync 4", &d2, &[(&a1, 5), (&a2, 7)]);
    assert_eq!(2, garbage(&d1));
    // One is held by the successor barrier for a round.
    assert_eq!(1, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 6", &d1, &[(&a1, 8), (&a2, 6)]);
    assert_eq!(0, garbage(&d1));
    // d2 still holds one behind the successor barrier.
    assert_eq!(1, garbage(&d2));

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// Sets `t` to "abc" on d1 and runs the steps the JS lifecycle cases share:
/// c2 inserts "c" while c1 deletes "bc", and both sync once.
async fn concurrent_insert_and_delete(
    c1: &Client,
    d1: &SharedDocument,
    c2: &Client,
    d2: &SharedDocument,
) {
    let (a1, a2) = (actor(c1), actor(c2));
    update(d1, |root| {
        let t = root.set_text("t")?;
        t.edit(0, 0, "a", None)?;
        t.edit(1, 1, "b", None)?;
        t.edit(2, 2, "c", None)?;
        Ok(())
    });
    check_vector("sets text", d1, &[(&a1, 1)]);
    c1.sync(None).await.unwrap();
    check_vector("c1 sync", d1, &[(&a1, 1)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync", d2, &[(&a1, 1), (&a2, 2)]);

    edit_text(d2, 2, 2, "c");
    check_vector("insert c", d2, &[(&a1, 1), (&a2, 3)]);
    edit_text(d1, 1, 3, "");
    check_vector("delete bd", d1, &[(&a1, 2)]);
}

/// JS "gc targeting nodes made by deactivated client" (gc_test.ts:1397).
///
/// JS deactivates c1 synchronously, so the server drops its version vector
/// row before c2 syncs. JS notes that a memory-backed server fails that
/// detach ("change not found"), and so does the Rust port against the local
/// memory-backed server; it needs a Mongo-backed server.
#[tokio::test]
#[ignore = "needs a Mongo-backed server: synchronous deactivate fails on the memory backend"]
async fn collects_nodes_made_by_a_deactivated_client() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-deactivated-client").await;

    concurrent_insert_and_delete(&c1, &d1, &c2, &d2).await;
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();

    c1.deactivate(DeactivateOptions {
        synchronous: true,
        ..DeactivateOptions::default()
    })
    .await
    .unwrap();
    assert_eq!(2, garbage(&d2));
    assert_eq!(2, d2.lock().unwrap().version_vector().size());

    c2.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d2));
    // JS TODO(JOOHOJANG): we have to consider removing detached client's
    // lamport from version vector
    assert_eq!(2, d2.lock().unwrap().version_vector().size());

    c2.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// JS "attach > pushpull > detach lifecycle version vector test (run gc at
/// last client detaches document, but no tombstone exsits)"
/// (gc_test.ts:1483).
#[tokio::test]
async fn collects_when_the_last_client_detaches_without_tombstones() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-lifecycle-no-tombstone").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    concurrent_insert_and_delete(&c1, &d1, &c2, &d2).await;
    assert_eq!(2, garbage(&d1));
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 2", &d1, &[(&a1, 2)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 2", &d2, &[(&a1, 2), (&a2, 4)]);
    assert_eq!(2, garbage(&d1));
    assert_eq!(2, garbage(&d2));

    edit_text(&d2, 2, 2, "1");
    check_vector("insert 1", &d2, &[(&a1, 2), (&a2, 5)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 3", &d2, &[(&a1, 2), (&a2, 5)]);
    assert_eq!(2, garbage(&d1));
    // One is held by the successor barrier for a round; see
    // `collects_garbage_with_concurrent_edits`.
    assert_eq!(1, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 3", &d1, &[(&a1, 6), (&a2, 5)]);

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 4", &d2, &[(&a1, 2), (&a2, 5)]);

    edit_text(&d2, 0, 3, "");
    check("delete all", &d2, &[(&a1, 2), (&a2, 6)], 3);
    c2.sync(None).await.unwrap();
    check("c2 sync 5", &d2, &[(&a1, 2), (&a2, 6)], 0);

    detach_and_deactivate(vec![(c1, d1), (c2, d2)]).await;
}

/// JS "attach > pushpull > detach lifecycle version vector test (run gc at
/// last client detaches document)" (gc_test.ts:1637).
#[tokio::test]
async fn collects_when_the_last_client_detaches() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-lifecycle").await;
    let (a1, a2) = (actor(&c1), actor(&c2));

    concurrent_insert_and_delete(&c1, &d1, &c2, &d2).await;
    assert_eq!(2, garbage(&d1));
    assert_eq!(0, garbage(&d2));

    c1.sync(None).await.unwrap();
    check_vector("c1 sync 2", &d1, &[(&a1, 2)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 2", &d2, &[(&a1, 2), (&a2, 4)]);
    assert_eq!(2, garbage(&d1));
    assert_eq!(2, garbage(&d2));

    edit_text(&d2, 2, 2, "1");
    check_vector("insert 1", &d2, &[(&a1, 2), (&a2, 5)]);
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 3", &d2, &[(&a1, 2), (&a2, 5)]);
    assert_eq!(2, garbage(&d1));
    // One is held by the successor barrier for a round; see
    // `collects_garbage_with_concurrent_edits`.
    assert_eq!(1, garbage(&d2));

    c1.sync(None).await.unwrap();
    // JS TODO(JOOHOJANG): we have to consider removing detached client's
    // lamport from version vector
    check_vector("c1 sync 3", &d1, &[(&a1, 6), (&a2, 5)]);

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.sync(None).await.unwrap();
    check_vector("c2 sync 4", &d2, &[(&a1, 2), (&a2, 5)]);

    edit_text(&d2, 0, 3, "");
    check("delete all", &d2, &[(&a1, 2), (&a2, 6)], 3);
    c2.detach(&d2, DetachOptions).await.unwrap();
    assert_eq!(0, garbage(&d2));

    detach_and_deactivate(vec![(c1, d1), (c2, d2)]).await;
}

/// JS "detach gc test" (gc_test.ts:1784).
#[tokio::test]
async fn collects_after_a_third_client_detaches() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("gc-detach");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;
    let (a1, a2, a3) = (actor(&c1), actor(&c2), actor(&c3));
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();
    let d3 = c3.attach(shared(&key), manual_attach()).await.unwrap();
    for client in [&c1, &c2, &c3] {
        client.sync(None).await.unwrap();
    }
    check_vector("d1", &d1, &[]);
    check_vector("d2", &d2, &[]);
    check_vector("d3", &d3, &[]);

    update(&d1, |root| {
        let t = root.set_text("t")?;
        t.edit(0, 0, "a", None)?;
        t.edit(1, 1, "b", None)?;
        t.edit(2, 2, "c", None)?;
        Ok(())
    });
    for client in [&c1, &c2, &c3] {
        client.sync(None).await.unwrap();
    }
    check_vector("d1 sets", &d1, &[(&a1, 1)]);
    check_vector("d2 sets", &d2, &[(&a1, 1), (&a2, 2)]);
    check_vector("d3 sets", &d3, &[(&a1, 1), (&a3, 2)]);

    edit_text(&d3, 1, 3, "");
    edit_text(&d1, 0, 0, "1");
    edit_text(&d1, 0, 0, "2");
    edit_text(&d1, 0, 0, "3");
    edit_text(&d2, 3, 3, "x");
    edit_text(&d2, 4, 4, "y");

    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    let json = r#"{"t":[{"val":"3"},{"val":"2"},{"val":"1"},{"val":"a"},{"val":"b"},{"val":"c"},{"val":"x"},{"val":"y"}]}"#;
    assert_eq!(json, d1.lock().unwrap().to_sorted_json());
    assert_eq!(json, d2.lock().unwrap().to_sorted_json());
    check_vector("d1 synced", &d1, &[(&a1, 6), (&a2, 4)]);
    check_vector("d2 synced", &d2, &[(&a1, 4), (&a2, 7)]);
    check_vector("d3 synced", &d3, &[(&a1, 1), (&a3, 3)]);

    c3.detach(&d3, DetachOptions).await.unwrap();
    edit_text(&d2, 5, 5, "z");

    c1.sync(None).await.unwrap();
    assert_eq!(2, garbage(&d1));
    c1.sync(None).await.unwrap();
    assert_eq!(2, garbage(&d1));

    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    // JS TODO(JOOHOJANG): we have to consider removing detached client's
    // lamport from version vector
    check_vector("d1 after detach", &d1, &[(&a1, 9), (&a2, 8), (&a3, 3)]);
    check_vector("d2 after detach", &d2, &[(&a1, 4), (&a2, 9), (&a3, 3)]);
    let json = r#"{"t":[{"val":"3"},{"val":"2"},{"val":"1"},{"val":"a"},{"val":"z"},{"val":"x"},{"val":"y"}]}"#;
    assert_eq!(json, d1.lock().unwrap().to_sorted_json());
    assert_eq!(json, d2.lock().unwrap().to_sorted_json());
    assert_eq!(2, garbage(&d1));
    assert_eq!(2, garbage(&d2));

    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d1));
    // One tombstone is still held by the successor barrier for a round; see
    // `collects_garbage_with_concurrent_edits`.
    assert_eq!(1, garbage(&d2));

    for client in [c1, c2, c3] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS `DefaultSnapshotThreshold` (`test/helper/helper.ts`).
const SNAPSHOT_THRESHOLD: usize = 500;

/// JS "snapshot version vector test" (gc_test.ts:1945).
#[tokio::test]
async fn keeps_version_vectors_through_a_snapshot() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("gc-snapshot-vector");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;
    let (a1, a2, a3) = (actor(&c1), actor(&c2), actor(&c3));
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();
    let d3 = c3.attach(shared(&key), manual_attach()).await.unwrap();

    update(&d1, |root| {
        root.set_text("t")?.edit(0, 0, "a", None).map(|_| ())
    });
    for client in [&c1, &c2, &c3] {
        client.sync(None).await.unwrap();
    }
    check_vector("d1 sets", &d1, &[(&a1, 1)]);
    check_vector("d2 sets", &d2, &[(&a1, 1), (&a2, 2)]);
    check_vector("d3 sets", &d3, &[(&a1, 1), (&a3, 2)]);

    // 01. Updates changes over snapshot threshold.
    for idx in 0..SNAPSHOT_THRESHOLD / 2 {
        edit_text(&d1, 0, 0, &(idx % 10).to_string());
        c1.sync(None).await.unwrap();
        c2.sync(None).await.unwrap();
        edit_text(&d2, 0, 0, &(idx % 10).to_string());
        c2.sync(None).await.unwrap();
        c1.sync(None).await.unwrap();
    }
    check_vector("d1 updates", &d1, &[(&a1, 1001), (&a2, 1000)]);
    check_vector("d2 updates", &d2, &[(&a1, 998), (&a2, 1000)]);
    check_vector("d3 updates", &d3, &[(&a1, 1), (&a3, 2)]);

    // 02. Makes local changes then pull a snapshot from the server.
    edit_text(&d3, 0, 0, "c");
    c3.sync(None).await.unwrap();
    let initial = ActorId::from("000000000000000000000000");
    check_vector(
        "d3 snapshot",
        &d3,
        &[(&a1, 998), (&a2, 1000), (&initial, 1002), (&a3, 1003)],
    );
    assert_eq!(SNAPSHOT_THRESHOLD + 2, text_of(&d3).len());

    // 03. Delete text after receiving the snapshot.
    edit_text(&d3, 1, 3, "");
    assert_eq!(SNAPSHOT_THRESHOLD, text_of(&d3).len());
    c3.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!(SNAPSHOT_THRESHOLD, text_of(&d2).len());
    assert_eq!(SNAPSHOT_THRESHOLD, text_of(&d1).len());

    for client in [c1, c2, c3] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "successor barrier drains within one round" (gc_test.ts:2070).
///
/// The successor barrier delays a purge when the node that would become a
/// forward skip's new stopping point is not yet causally stable. A delay is
/// acceptable; a leak is not. This replays
/// `collects_garbage_with_concurrent_edits` and keeps syncing with no
/// further edits: retention has to reach zero and the replicas have to
/// agree.
#[tokio::test]
async fn drains_a_successor_barrier_within_one_round() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, d1, c2, d2) = attach_two(&addr, "gc-barrier-drains").await;

    update(&d1, |root| {
        let t = root.set_text("t")?;
        t.edit(0, 0, "a", None)?;
        t.edit(1, 1, "b", None)?;
        t.edit(2, 2, "c", None)?;
        Ok(())
    });
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();

    // c2 inserts next to what c1 is about to delete. The tombstone c1 leaves
    // is what stops the forward skip c2's insert is positioned by.
    edit_text(&d2, 2, 2, "c");
    edit_text(&d1, 1, 3, "");
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    edit_text(&d2, 2, 2, "1");
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();

    // Where the assertions above stop: one tombstone each, held back.
    assert_eq!(1, garbage(&d1));
    assert_eq!(1, garbage(&d2));

    // One more round with no edits at all has to drain it.
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(0, garbage(&d1), "the barrier must delay, not prevent");
    assert_eq!(0, garbage(&d2), "the barrier must delay, not prevent");

    // Further rounds must not resurrect anything.
    for _ in 0..3 {
        c1.sync(None).await.unwrap();
        c2.sync(None).await.unwrap();
        assert_eq!(0, garbage(&d1));
        assert_eq!(0, garbage(&d2));
    }
    let json = d1.lock().unwrap().to_sorted_json();
    assert_eq!(json, d2.lock().unwrap().to_sorted_json());
    assert_eq!(r#"{"t":[{"val":"a"},{"val":"c"},{"val":"1"}]}"#, json);

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "successor barrier drains after the successor author detaches"
/// (gc_test.ts:2136).
///
/// A barrier ticket is authored by whoever created the successor node, not
/// by the remover, so that author may have left the document by the time the
/// purge is due. The author leaving must not make the barrier unsatisfiable.
#[tokio::test]
async fn drains_a_successor_barrier_after_its_author_detaches() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("gc-barrier-author-detaches");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();
    let d3 = c3.attach(shared(&key), manual_attach()).await.unwrap();

    update(&d1, |root| {
        let t = root.set_text("t")?;
        t.edit(0, 0, "a", None)?;
        t.edit(1, 1, "b", None)?;
        t.edit(2, 2, "c", None)?;
        Ok(())
    });
    for client in [&c1, &c2, &c3] {
        client.sync(None).await.unwrap();
    }

    // c3 inserts "X" right behind "b" while c1 deletes "b": "X" becomes the
    // successor of the tombstone, so its ticket is c3's.
    edit_text(&d3, 2, 2, "X");
    edit_text(&d1, 1, 2, "");
    for client in [&c1, &c2, &c3, &c1] {
        client.sync(None).await.unwrap();
    }
    assert_eq!("aXc", text_of(&d1));

    // c3 leaves; its row is removed from the server.
    c3.detach(&d3, DetachOptions).await.unwrap();

    // No further edits. The purge may be held back while c2 has not seen
    // "X", but it has to drain once c2 catches up, although "X"'s author has
    // gone.
    for _ in 0..3 {
        c2.sync(None).await.unwrap();
        c1.sync(None).await.unwrap();
    }
    assert_eq!(0, garbage(&d1), "d1 retained the tombstone");
    assert_eq!(0, garbage(&d2), "d2 retained the tombstone");
    assert_eq!(text_of(&d1), text_of(&d2));

    for client in [c1, c2, c3] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}
