//! Presence integration tests against a live server.
//!
//! JS reference: `packages/sdk/test/integration/doc_presence_test.ts`.
//!
//! JS `getPresenceForTest` reads a stored presence whether or not its client
//! is online; `presence_for_test` is its counterpart. JS `getPresences` lists
//! this client first and then the online clients in the order they came
//! online; Rust `Document::presences` sorts by actor ID, so the expected
//! lists here are sorted the same way.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::events::{collect_connection, collect_presence};
use integration_support::two_clients::{active_client, shared};
use integration_support::{presence_for_test, rpc_addr, unique_key};
use serde_json::{json, Value};
use yorkie::{
    ActorId, AttachOptions, Client, DeactivateOptions, DetachOptions, DocEvent, JsonCounter,
    Presence, SharedDocument, StreamConnectionStatus, SyncMode,
};

/// JS `DefaultSnapshotThreshold` (`test/helper/helper.ts`).
const SNAPSHOT_THRESHOLD: usize = 500;

/// JS `InitialActorID`.
const INITIAL_ACTOR_ID: &str = "000000000000000000000000";

/// A presence from a JSON object literal.
fn presence(value: Value) -> Presence {
    match value {
        Value::Object(presence) => presence,
        other => panic!("a presence is an object, got {other}"),
    }
}

fn attach_options(initial_presence: Value, sync_mode: SyncMode) -> AttachOptions {
    AttachOptions {
        initial_presence: presence(initial_presence),
        sync_mode,
        ..AttachOptions::default()
    }
}

fn manual() -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        ..AttachOptions::default()
    }
}

fn actor(client: &Client) -> ActorId {
    client.actor_id().unwrap()
}

fn my_presence(doc: &SharedDocument) -> Presence {
    doc.lock().unwrap().my_presence()
}

/// Sets presence keys in an update (JS `doc.update((root, p) => p.set(...))`).
fn set_presence(doc: &SharedDocument, value: Value) {
    doc.lock()
        .unwrap()
        .update_with_presence(|_, p| {
            p.set(presence(value));
            Ok(())
        })
        .unwrap();
}

/// The `(client, presence)` pairs sorted by actor ID, the order
/// `Document::presences` lists them in.
fn sorted(mut presences: Vec<(ActorId, Presence)>) -> Vec<(ActorId, Presence)> {
    presences.sort_by(|a, b| a.0.cmp(&b.0));
    presences
}

async fn deactivate(clients: &[&Client]) {
    for client in clients {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Can be built from a snapshot" (doc_presence_test.ts:42).
#[tokio::test]
async fn can_be_built_from_a_snapshot() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("presence-from-snapshot");
    let doc1 = c1.attach(shared(&key), manual()).await.unwrap();
    let doc2 = c2.attach(shared(&key), manual()).await.unwrap();

    for i in 0..SNAPSHOT_THRESHOLD {
        set_presence(&doc1, json!({ "key": i.to_string() }));
    }
    let last = presence(json!({ "key": (SNAPSHOT_THRESHOLD - 1).to_string() }));
    assert_eq!(Some(last.clone()), presence_for_test(&doc1, &actor(&c1)));

    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(Some(last), presence_for_test(&doc2, &actor(&c1)));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Can be set initial value in attach and be removed in detach"
/// (doc_presence_test.ts:70).
#[tokio::test]
async fn can_be_set_in_attach_and_removed_in_detach() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("presence-attach-detach");
    let doc1 = c1
        .attach(
            shared(&key),
            attach_options(json!({ "key": "key1" }), SyncMode::Manual),
        )
        .await
        .unwrap();
    let doc2 = c2
        .attach(
            shared(&key),
            attach_options(json!({ "key": "key2" }), SyncMode::Manual),
        )
        .await
        .unwrap();

    let (c1_id, c2_id) = (actor(&c1), actor(&c2));
    assert_eq!(
        Some(presence(json!({ "key": "key1" }))),
        presence_for_test(&doc1, &c1_id)
    );
    assert_eq!(None, presence_for_test(&doc1, &c2_id));
    assert_eq!(
        Some(presence(json!({ "key": "key2" }))),
        presence_for_test(&doc2, &c2_id)
    );
    assert_eq!(
        Some(presence(json!({ "key": "key1" }))),
        presence_for_test(&doc2, &c1_id)
    );

    c1.sync(None).await.unwrap();
    assert_eq!(
        Some(presence(json!({ "key": "key2" }))),
        presence_for_test(&doc1, &c2_id)
    );

    c2.detach(&doc2, DetachOptions).await.unwrap();
    c1.sync(None).await.unwrap();
    assert!(!doc1.lock().unwrap().has_presence(&c2_id));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should be initialized as an empty object if no initial value is set
/// during attach" (doc_presence_test.ts:113).
#[tokio::test]
async fn is_empty_without_an_initial_value() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("presence-empty");
    let doc1 = c1.attach(shared(&key), manual()).await.unwrap();
    let doc2 = c2.attach(shared(&key), manual()).await.unwrap();

    let (c1_id, c2_id) = (actor(&c1), actor(&c2));
    let empty_object = Some(Presence::new());
    assert_eq!(empty_object, presence_for_test(&doc1, &c1_id));
    assert_eq!(None, presence_for_test(&doc1, &c2_id));
    assert_eq!(empty_object, presence_for_test(&doc2, &c2_id));
    assert_eq!(empty_object, presence_for_test(&doc2, &c1_id));

    c1.sync(None).await.unwrap();
    assert_eq!(empty_object, presence_for_test(&doc1, &c2_id));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should be synced eventually" (doc_presence_test.ts:139).
#[tokio::test]
async fn is_synced_eventually() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let (c1_id, c2_id) = (actor(&c1), actor(&c2));
    let key = unique_key("presence-synced");
    let realtime = |name: &str| attach_options(json!({ "name": name }), SyncMode::Realtime);

    let doc1 = c1.attach(shared(&key), realtime("a")).await.unwrap();
    let events1 = collect_presence(&doc1);
    assert_eq!(
        vec![(c1_id.clone(), presence(json!({ "name": "a" })))],
        doc1.lock().unwrap().presences()
    );

    let doc2 = c2.attach(shared(&key), realtime("b")).await.unwrap();
    let events2 = collect_presence(&doc2);
    assert_eq!(
        sorted(vec![
            (c2_id.clone(), presence(json!({ "name": "b" }))),
            (c1_id.clone(), presence(json!({ "name": "a" }))),
        ]),
        doc2.lock().unwrap().presences()
    );

    events1
        .wait_and_verify_nth(
            1,
            DocEvent::Watched(c2_id.clone(), presence(json!({ "name": "b" }))),
        )
        .await;

    set_presence(&doc1, json!({ "name": "A" }));
    set_presence(&doc2, json!({ "name": "B" }));

    let changed = |id: &ActorId, name: &str| {
        DocEvent::PresenceChanged(id.clone(), presence(json!({ "name": name })))
    };
    events1.wait_and_verify_nth(2, changed(&c1_id, "A")).await;
    events1.wait_and_verify_nth(3, changed(&c2_id, "B")).await;
    events2.wait_and_verify_nth(1, changed(&c2_id, "B")).await;
    events2.wait_and_verify_nth(2, changed(&c1_id, "A")).await;
    let expected = sorted(vec![
        (c2_id.clone(), presence(json!({ "name": "B" }))),
        (c1_id.clone(), presence(json!({ "name": "A" }))),
    ]);
    assert_eq!(expected, doc2.lock().unwrap().presences());
    assert_eq!(expected, doc1.lock().unwrap().presences());

    deactivate(&[&c1, &c2]).await;
}

/// JS "Can be updated partially by doc.update function"
/// (doc_presence_test.ts:221).
#[tokio::test]
async fn can_be_updated_partially() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("presence-partial");
    let doc1 = c1
        .attach(
            shared(&key),
            attach_options(
                json!({ "key": "key1", "cursor": { "x": 0, "y": 0 } }),
                SyncMode::Manual,
            ),
        )
        .await
        .unwrap();
    let doc2 = c2
        .attach(
            shared(&key),
            attach_options(
                json!({ "key": "key2", "cursor": { "x": 0, "y": 0 } }),
                SyncMode::Manual,
            ),
        )
        .await
        .unwrap();

    set_presence(&doc1, json!({ "cursor": { "x": 1, "y": 1 } }));
    let expected = presence(json!({ "key": "key1", "cursor": { "x": 1, "y": 1 } }));
    assert_eq!(
        Some(expected.clone()),
        presence_for_test(&doc1, &actor(&c1))
    );

    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(Some(expected), presence_for_test(&doc2, &actor(&c1)));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should return only online clients" (doc_presence_test.ts:257).
#[tokio::test]
async fn returns_only_online_clients() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;
    let (c1_id, c2_id, c3_id) = (actor(&c1), actor(&c2), actor(&c3));
    let key = unique_key("presence-online-only");
    let initial = |name: &str, sync_mode| {
        attach_options(
            json!({ "name": name, "cursor": { "x": 0, "y": 0 } }),
            sync_mode,
        )
    };

    let doc1 = c1
        .attach(shared(&key), initial("a1", SyncMode::Realtime))
        .await
        .unwrap();
    let events = collect_presence(&doc1);

    // 01. c2 attaches doc in realtime sync, and c3 attached doc in manual sync.
    let doc2 = c2
        .attach(shared(&key), initial("b1", SyncMode::Realtime))
        .await
        .unwrap();
    let doc3 = c3
        .attach(shared(&key), initial("c1", SyncMode::Manual))
        .await
        .unwrap();
    events
        .wait_and_verify_nth(1, DocEvent::Watched(c2_id.clone(), my_presence(&doc2)))
        .await;
    assert_eq!(
        sorted(vec![
            (c1_id.clone(), my_presence(&doc1)),
            (c2_id.clone(), my_presence(&doc2)),
        ]),
        doc1.lock().unwrap().presences()
    );
    assert_eq!(None, doc1.lock().unwrap().presence(&c3_id));

    // 02. c2 is changed to manual sync, while c3 is changed to realtime sync.
    c2.change_sync_mode(&doc2, SyncMode::Manual).await.unwrap();
    events
        .wait_and_verify_nth(2, DocEvent::Unwatched(c2_id.clone(), my_presence(&doc2)))
        .await;
    c3.change_sync_mode(&doc3, SyncMode::Realtime)
        .await
        .unwrap();
    events
        .wait_and_verify_nth(3, DocEvent::Watched(c3_id.clone(), my_presence(&doc3)))
        .await;
    assert_eq!(
        sorted(vec![
            (c1_id.clone(), my_presence(&doc1)),
            (c3_id.clone(), my_presence(&doc3)),
        ]),
        doc1.lock().unwrap().presences()
    );
    assert_eq!(None, doc1.lock().unwrap().presence(&c2_id));

    deactivate(&[&c1, &c2, &c3]).await;
}

/// JS "Can get presence value using p.get() within doc.update function"
/// (doc_presence_test.ts:324).
#[tokio::test]
async fn can_get_presence_value_within_update() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("presence-get");
    let doc1 = c1
        .attach(
            shared(&key),
            attach_options(json!({ "counter": 0 }), SyncMode::Manual),
        )
        .await
        .unwrap();
    let doc2 = c2
        .attach(
            shared(&key),
            attach_options(json!({ "counter": 0 }), SyncMode::Manual),
        )
        .await
        .unwrap();

    doc1.lock()
        .unwrap()
        .update_with_presence(|_, p| {
            let counter = p.get("counter").and_then(Value::as_i64).unwrap();
            p.set(presence(json!({ "counter": counter + 1 })));
            Ok(())
        })
        .unwrap();
    let expected = presence(json!({ "counter": 1 }));
    assert_eq!(
        Some(expected.clone()),
        presence_for_test(&doc1, &actor(&c1))
    );

    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    assert_eq!(Some(expected), presence_for_test(&doc2, &actor(&c1)));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should not be accessible to other clients' presence when the stream
/// is disconnected" (doc_presence_test.ts:357).
#[tokio::test]
async fn hides_other_presences_when_the_stream_is_disconnected() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c2_id = actor(&c2);
    let key = unique_key("presence-disconnected");

    let doc1 = c1
        .attach(
            shared(&key),
            attach_options(json!({ "name": "a" }), SyncMode::Realtime),
        )
        .await
        .unwrap();
    let events = collect_presence(&doc1);
    let connection_events = collect_connection(&doc1);

    let doc2 = c2
        .attach(
            shared(&key),
            attach_options(json!({ "name": "b" }), SyncMode::Realtime),
        )
        .await
        .unwrap();
    events
        .wait_and_verify_nth(1, DocEvent::Watched(c2_id.clone(), my_presence(&doc2)))
        .await;
    assert_eq!(
        Some(presence(json!({ "name": "b" }))),
        doc1.lock().unwrap().presence(&c2_id)
    );

    c1.change_sync_mode(&doc1, SyncMode::Manual).await.unwrap();
    connection_events
        .wait_and_verify_nth(1, StreamConnectionStatus::Disconnected)
        .await;
    assert_eq!(None, doc1.lock().unwrap().presence(&c2_id));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should receive presence-changed event for final presence if there
/// are multiple presence changes within doc.update"
/// (doc_presence_test.ts:405).
#[tokio::test]
async fn receives_the_final_presence_of_an_update() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let (c1_id, c2_id) = (actor(&c1), actor(&c2));
    let key = unique_key("presence-final");
    let initial = |name: &str| {
        attach_options(
            json!({ "name": name, "cursor": { "x": 0, "y": 0 } }),
            SyncMode::Realtime,
        )
    };

    let doc1 = c1.attach(shared(&key), initial("a")).await.unwrap();
    let events_p1 = collect_presence(&doc1);
    let doc2 = c2.attach(shared(&key), initial("b")).await.unwrap();
    let events_p2 = collect_presence(&doc2);
    events_p1
        .wait_and_verify_nth(1, DocEvent::Watched(c2_id, my_presence(&doc2)))
        .await;

    doc1.lock()
        .unwrap()
        .update_with_presence(|_, p| {
            p.set(presence(json!({ "name": "A" })));
            p.set(presence(json!({ "cursor": { "x": 1, "y": 1 } })));
            p.set(presence(json!({ "name": "X" })));
            Ok(())
        })
        .unwrap();

    let changed = DocEvent::PresenceChanged(c1_id, my_presence(&doc1));
    events_p1.wait_and_verify_nth(2, changed.clone()).await;
    events_p2.wait_and_verify_nth(1, changed).await;

    deactivate(&[&c1, &c2]).await;
}

/// JS "Can receive 'unwatched' event when a client detaches"
/// (doc_presence_test.ts:462).
#[tokio::test]
async fn receives_unwatched_when_a_client_detaches() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let (c1_id, c2_id) = (actor(&c1), actor(&c2));
    let key = unique_key("presence-unwatched");

    let doc1 = c1
        .attach(
            shared(&key),
            attach_options(json!({ "name": "a" }), SyncMode::Realtime),
        )
        .await
        .unwrap();
    let events = collect_presence(&doc1);

    let doc2 = c2
        .attach(
            shared(&key),
            attach_options(json!({ "name": "b" }), SyncMode::Realtime),
        )
        .await
        .unwrap();
    events
        .wait_and_verify_nth(
            1,
            DocEvent::Watched(c2_id.clone(), presence(json!({ "name": "b" }))),
        )
        .await;

    c2.detach(&doc2, DetachOptions).await.unwrap();
    events
        .wait_and_verify_nth(
            2,
            DocEvent::Unwatched(c2_id, presence(json!({ "name": "b" }))),
        )
        .await;

    assert_eq!(
        vec![(c1_id, presence(json!({ "name": "a" })))],
        doc1.lock().unwrap().presences()
    );
    assert_eq!(
        vec![(ActorId::new(INITIAL_ACTOR_ID), Presence::new())],
        doc2.lock().unwrap().presences()
    );

    deactivate(&[&c1, &c2]).await;
}

/// JS "Can receive presence-related event only when using realtime sync"
/// (doc_presence_test.ts:510).
#[tokio::test]
async fn receives_presence_events_only_in_realtime_sync() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("presence-realtime-only");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;
    let (c2_id, c3_id) = (actor(&c2), actor(&c3));
    let initial = |name: &str, sync_mode| {
        attach_options(
            json!({ "name": name, "cursor": { "x": 0, "y": 0 } }),
            sync_mode,
        )
    };
    let with_name = |name: &str| presence(json!({ "cursor": { "x": 0, "y": 0 }, "name": name }));

    let doc1 = c1
        .attach(shared(&key), initial("a1", SyncMode::Realtime))
        .await
        .unwrap();
    let events = collect_presence(&doc1);

    // 01. c2 attaches doc in realtime sync, and c3 attached doc in manual
    //     sync. c1 receives the watched event from c2.
    let doc2 = c2
        .attach(shared(&key), initial("b1", SyncMode::Realtime))
        .await
        .unwrap();
    let doc3 = c3
        .attach(shared(&key), initial("c1", SyncMode::Manual))
        .await
        .unwrap();
    events
        .wait_and_verify_nth(1, DocEvent::Watched(c2_id.clone(), with_name("b1")))
        .await;

    // 02. c2 and c3 update the presence.
    //     c1 receives the presence-changed event from c2.
    set_presence(&doc2, json!({ "name": "b2" }));
    set_presence(&doc3, json!({ "name": "c2" }));
    events
        .wait_and_verify_nth(2, DocEvent::PresenceChanged(c2_id.clone(), with_name("b2")))
        .await;

    // 03-1. c2 is changed to manual sync, c1 receives an unwatched event
    //       from c2.
    c2.change_sync_mode(&doc2, SyncMode::Manual).await.unwrap();
    events
        .wait_and_verify_nth(3, DocEvent::Unwatched(c2_id.clone(), with_name("b2")))
        .await;
    // 03-2. c3 is changed to realtime sync, c1 receives a watched event from
    //       c3. As in JS, the mode changes after the syncs: they bring c1
    //       every earlier presence change of c3, so only the watched event
    //       follows.
    c3.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    c3.change_sync_mode(&doc3, SyncMode::Realtime)
        .await
        .unwrap();
    events
        .wait_and_verify_nth(4, DocEvent::Watched(c3_id.clone(), with_name("c2")))
        .await;

    // 04. c2 and c3 update the presence.
    //     c1 receives the presence-changed event from c3.
    set_presence(&doc2, json!({ "name": "b3" }));
    set_presence(&doc3, json!({ "name": "c3" }));
    events
        .wait_and_verify_nth(5, DocEvent::PresenceChanged(c3_id.clone(), with_name("c3")))
        .await;

    // 05-1. c3 is changed to manual sync, c1 receives an unwatched event
    //       from c3.
    c3.change_sync_mode(&doc3, SyncMode::Manual).await.unwrap();
    events
        .wait_and_verify_nth(6, DocEvent::Unwatched(c3_id, with_name("c3")))
        .await;

    // 05-2. c2 is changed to realtime sync, c1 receives a watched event from
    //       c2.
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    c2.change_sync_mode(&doc2, SyncMode::Realtime)
        .await
        .unwrap();
    events
        .wait_and_verify_nth(7, DocEvent::Watched(c2_id, with_name("b3")))
        .await;

    deactivate(&[&c1, &c2, &c3]).await;
}

fn set_presence_with_history(doc: &SharedDocument, value: Value) {
    doc.lock()
        .unwrap()
        .update_with_presence(|_, p| {
            p.set_with_history(presence(value));
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

/// JS "Can undo/redo with presence" (doc_presence_test.ts:633).
#[tokio::test]
async fn can_undo_and_redo_with_presence() {
    let Some(addr) = rpc_addr() else { return };
    let doc = shared(&unique_key("presence-undo-redo"));
    doc.lock()
        .unwrap()
        .update(|root| root.set("counter", JsonCounter::integer(100)).map(|_| ()))
        .unwrap();

    let client = active_client(&addr).await;
    let doc = client
        .attach(
            doc,
            attach_options(json!({ "color": "red" }), SyncMode::Realtime),
        )
        .await
        .unwrap();

    // 1. Presence update only
    set_presence_with_history(&doc, json!({ "color": "blue" }));
    assert_eq!(presence(json!({ "color": "blue" })), my_presence(&doc));

    undo(&doc);
    assert_eq!(presence(json!({ "color": "red" })), my_presence(&doc));

    redo(&doc);
    assert_eq!(presence(json!({ "color": "blue" })), my_presence(&doc));

    // 2. Presence update with root update
    doc.lock()
        .unwrap()
        .update_with_presence(|root, p| {
            root.get_counter_mut("counter")?.increase(1)?;
            p.set_with_history(presence(json!({ "color": "green" })));
            Ok(())
        })
        .unwrap();
    assert_eq!(r#"{"counter":101}"#, doc.lock().unwrap().to_sorted_json());
    assert_eq!(presence(json!({ "color": "green" })), my_presence(&doc));

    undo(&doc);
    assert_eq!(r#"{"counter":100}"#, doc.lock().unwrap().to_sorted_json());
    assert_eq!(presence(json!({ "color": "blue" })), my_presence(&doc));

    redo(&doc);
    assert_eq!(r#"{"counter":101}"#, doc.lock().unwrap().to_sorted_json());
    assert_eq!(presence(json!({ "color": "green" })), my_presence(&doc));

    deactivate(&[&client]).await;
}

/// JS "Should not impact undo if presence is not added to history"
/// (doc_presence_test.ts:689).
///
/// The JS checks of `getUndoStackForTest` are left out: the Rust undo stack
/// is only readable from the core crate's unit tests. The undo and redo
/// results below are what those stacks decide.
#[tokio::test]
async fn presence_without_history_does_not_impact_undo() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let doc = client
        .attach(
            shared(&unique_key("presence-not-in-history")),
            attach_options(
                json!({ "color": "red", "cursor": { "x": 0, "y": 0 } }),
                SyncMode::Realtime,
            ),
        )
        .await
        .unwrap();
    let expect = |value: Value| assert_eq!(presence(value), my_presence(&doc));

    // 1. Setting addToHistory for both color and cursor
    set_presence_with_history(
        &doc,
        json!({ "color": "blue", "cursor": { "x": 1, "y": 1 } }),
    );
    expect(json!({ "color": "blue", "cursor": { "x": 1, "y": 1 } }));

    undo(&doc);
    expect(json!({ "color": "red", "cursor": { "x": 0, "y": 0 } }));

    redo(&doc);
    expect(json!({ "color": "blue", "cursor": { "x": 1, "y": 1 } }));

    // 2. Setting addToHistory only for the cursor
    doc.lock()
        .unwrap()
        .update_with_presence(|_, p| {
            p.set(presence(json!({ "color": "green" })));
            p.set_with_history(presence(json!({ "cursor": { "x": 2, "y": 2 } })));
            Ok(())
        })
        .unwrap();
    expect(json!({ "color": "green", "cursor": { "x": 2, "y": 2 } }));

    undo(&doc);
    expect(json!({ "color": "green", "cursor": { "x": 1, "y": 1 } }));

    redo(&doc);
    expect(json!({ "color": "green", "cursor": { "x": 2, "y": 2 } }));

    // 3. Not setting addToHistory
    doc.lock()
        .unwrap()
        .update_with_presence(|_, p| {
            p.set(presence(json!({ "color": "black" })));
            p.set(presence(json!({ "cursor": { "x": 3, "y": 3 } })));
            Ok(())
        })
        .unwrap();
    expect(json!({ "color": "black", "cursor": { "x": 3, "y": 3 } }));

    undo(&doc);
    expect(json!({ "color": "black", "cursor": { "x": 1, "y": 1 } }));

    redo(&doc);
    expect(json!({ "color": "black", "cursor": { "x": 3, "y": 3 } }));

    deactivate(&[&client]).await;
}

/// JS "Should handle undo/redo correctly for multiple changes to a single
/// presence key within update" (doc_presence_test.ts:809).
#[tokio::test]
async fn undoes_several_changes_to_one_presence_key_in_an_update() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let doc = client
        .attach(
            shared(&unique_key("presence-one-key-history")),
            attach_options(json!({ "color": "red" }), SyncMode::Realtime),
        )
        .await
        .unwrap();
    let expect = |color: &str| assert_eq!(presence(json!({ "color": color })), my_presence(&doc));
    let update = |first: (&str, bool), second: (&str, bool)| {
        doc.lock()
            .unwrap()
            .update_with_presence(|_, p| {
                for (color, add_to_history) in [first, second] {
                    let value = presence(json!({ "color": color }));
                    if add_to_history {
                        p.set_with_history(value);
                    } else {
                        p.set(value);
                    }
                }
                Ok(())
            })
            .unwrap();
    };

    // 1. When multiple changes are made to the "color" key,
    // it should revert to the value before doc.update() call.
    update(("blue", true), ("green", true));
    expect("green");

    undo(&doc);
    expect("red");

    redo(&doc);
    expect("green");

    // 2. `addToHistory` option accumulates for a single key,
    // applying to the last key only. When set to true for the
    // last "color" key, it adds the color to the undo stack.
    update(("black", false), ("purple", true));
    expect("purple");

    undo(&doc);
    expect("green");

    redo(&doc);
    expect("purple");

    // 3. When `addToHistory` is false for the last key, it
    // will not be added to the undo stack. The default value
    // when the option is not set is false.
    update(("yellow", true), ("orange", false));
    expect("orange");

    undo(&doc);
    expect("green");

    redo(&doc);
    expect("orange");

    deactivate(&[&client]).await;
}

/// JS "Should emit presence-changed event with initial presence value on
/// attach" (doc_presence_test.ts:885). A Rust `PresenceChanged` carries no
/// `OpSource`, so the JS `source: OpSource.Local` check has nothing to check.
#[tokio::test]
async fn emits_presence_changed_with_the_initial_presence_on_attach() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c1_id = actor(&c1);

    let doc1 = shared(&unique_key("presence-on-attach"));
    let events1 = collect_presence(&doc1);

    let _doc1 = c1
        .attach(
            doc1,
            attach_options(json!({ "key": "val1" }), SyncMode::Realtime),
        )
        .await
        .unwrap();

    events1
        .wait_and_verify_nth(
            1,
            DocEvent::PresenceChanged(c1_id, presence(json!({ "key": "val1" }))),
        )
        .await;

    deactivate(&[&c1]).await;
}
