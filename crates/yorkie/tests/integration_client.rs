//! Integration tests for the client against a live server.
//!
//! JS reference: `packages/sdk/test/integration/client_test.ts`. Its realtime
//! cases "Can recover from temporary disconnect (realtime sync)" (watch
//! stream side) and "Can change sync mode(realtime <-> manual)" live in
//! `integration_realtime.rs`.
//!
//! JS "Can activate with gRPC-Web transport" (client_test.ts:70) is not
//! ported: the Rust client has only the Connect transport.
//!
//! JS stubs the global `fetch` to simulate a network failure. Here a
//! `StubTransport` fails the push-pull calls of the clients built on one
//! `Network` instead, and stands in for replacing `rpcClient.pushPullChanges`.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::events::{
    collect_change_types, collect_presence, collect_sync_status, ChangeEventType,
};
use integration_support::stub_transport::{
    failed_to_fetch, rpc_error, stubbed_client, Network, PushPullHook,
};
use integration_support::tree::{edit_tree, element, set_tree, text, xml};
use integration_support::two_clients::{active_client, manual_attach, shared, TwoClients};
use integration_support::{deactivate, eventually, rpc_addr, unique_key};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use yorkie::{
    AttachOptions, Client, ClientCondition, ClientOptions, CounterValue, DeactivateOptions,
    DetachOptions, DocEvent, DocStatus, DocSyncStatus, ErrorCode, JsonCounter, JsonValue,
    SharedDocument, SyncMode,
};

/// JS `DefaultSnapshotThreshold` (`test/helper/helper.ts`).
const SNAPSHOT_THRESHOLD: i32 = 500;

fn options(addr: &str) -> ClientOptions {
    ClientOptions {
        rpc_addr: addr.to_owned(),
        ..ClientOptions::default()
    }
}

fn attach_in(sync_mode: SyncMode) -> AttachOptions {
    AttachOptions {
        sync_mode,
        ..AttachOptions::default()
    }
}

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn set(doc: &SharedDocument, key: &str, value: impl Into<JsonValue>) {
    let value = value.into();
    doc.lock()
        .unwrap()
        .update(|root| root.set(key, value).map(|_| ()))
        .unwrap();
}

fn create_counter(doc: &SharedDocument) {
    doc.lock()
        .unwrap()
        .update(|root| root.set("counter", JsonCounter::integer(0)).map(|_| ()))
        .unwrap();
}

fn increase_counter(doc: &SharedDocument) {
    doc.lock()
        .unwrap()
        .update(|root| root.get_counter_mut("counter")?.increase(1).map(|_| ()))
        .unwrap();
}

fn counter(doc: &SharedDocument) -> CounterValue {
    match doc.lock().unwrap().get_root().get("counter") {
        Some(JsonValue::Counter(counter)) => counter.value(),
        other => panic!("expected a counter, got {other:?}"),
    }
}

fn text_value(doc: &SharedDocument) -> String {
    match doc.lock().unwrap().get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected a text, got {other:?}"),
    }
}

fn edit_text(doc: &SharedDocument, from: usize, content: &str) {
    doc.lock()
        .unwrap()
        .update(|root| {
            root.get_text_mut("t")?
                .edit(from, from, content, None)
                .map(|_| ())
        })
        .unwrap();
}

/// The `<doc><p>12</p><p>34</p></doc>` tree of the push-only and sync-off
/// cases.
fn set_two_paragraphs(doc: &SharedDocument) {
    set_tree(
        doc,
        element(
            "doc",
            vec![
                element("p", vec![text("12")]),
                element("p", vec![text("34")]),
            ],
        ),
    );
}

#[tokio::test]
async fn activate_attach_sync_detach_roundtrip() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("roundtrip");

    let a = active_client(&addr).await;
    let doc_a = a.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc_a, "k1", "v1");
    a.sync(Some(&doc_a)).await.unwrap();
    a.detach(&doc_a, DetachOptions).await.unwrap();

    let b = active_client(&addr).await;
    let doc_b = b.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&doc_b));

    deactivate(&[&a, &b]).await;
}

/// JS "Can be activated, deactivated" (client_test.ts:45).
#[tokio::test]
async fn can_be_activated_and_deactivated() {
    let Some(addr) = rpc_addr() else { return };
    let client_key = unique_key("activate-deactivate");
    let client_with_key = Client::new(ClientOptions {
        key: Some(client_key.clone()),
        sync_loop_duration: Duration::from_millis(50),
        reconnect_stream_delay: Duration::from_millis(1000),
        ..options(&addr)
    })
    .unwrap();
    assert!(!client_with_key.is_active());
    client_with_key.activate().await.unwrap();
    assert!(client_with_key.is_active());
    assert_eq!(client_key, client_with_key.key());
    deactivate(&[&client_with_key]).await;
    assert!(!client_with_key.is_active());

    let client_without_key = Client::new(options(&addr)).unwrap();
    assert!(!client_without_key.is_active());
    client_without_key.activate().await.unwrap();
    assert!(client_without_key.is_active());
    assert_eq!(36, client_without_key.key().len());
    deactivate(&[&client_without_key]).await;
    assert!(!client_without_key.is_active());
}

/// JS "Can attach/detach document" (client_test.ts:84).
#[tokio::test]
async fn can_attach_and_detach_document() {
    let Some(addr) = rpc_addr() else { return };
    let cli = active_client(&addr).await;
    let doc = shared(&unique_key("attach-detach"));

    cli.attach(doc.clone(), AttachOptions::default())
        .await
        .unwrap();
    let err = cli
        .attach(doc.clone(), AttachOptions::default())
        .await
        .unwrap_err();
    assert_eq!(Some(ErrorCode::ErrNotDetached), err.code(), "{err:?}");

    cli.detach(&doc, DetachOptions).await.unwrap();
    let err = cli.detach(&doc, DetachOptions).await.unwrap_err();
    assert_eq!(Some(ErrorCode::ErrNotAttached), err.code(), "{err:?}");

    deactivate(&[&cli]).await;
    deactivate(&[&cli]).await;
}

/// JS "Can handle sync" (client_test.ts:105).
#[tokio::test]
async fn can_handle_sync() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "handle-sync").await;
    // JS `d2.subscribe(spy)`: the default subscription, which gets snapshot
    // and change events.
    let calls = Arc::new(AtomicUsize::new(0));
    let unsub = t.d2.lock().unwrap().subscribe({
        let calls = calls.clone();
        move |event| {
            if matches!(
                event,
                DocEvent::Snapshot | DocEvent::LocalChange(_) | DocEvent::RemoteChange(_)
            ) {
                calls.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    assert_eq!(0, calls.load(Ordering::SeqCst));

    set(&t.d1, "k1", "v1");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(1, calls.load(Ordering::SeqCst));

    set(&t.d1, "k2", "v2");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(2, calls.load(Ordering::SeqCst));

    t.d2.lock().unwrap().unsubscribe(unsub);

    set(&t.d1, "k3", "v3");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(2, calls.load(Ordering::SeqCst));

    t.deactivate().await;
}

/// Both clients push and pull by hand, in both directions, and converge.
#[tokio::test]
async fn two_clients_converge_with_manual_sync() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("two-clients-converge");
    let a = active_client(&addr).await;
    let b = active_client(&addr).await;
    let doc_a = a.attach(shared(&key), manual_attach()).await.unwrap();
    let doc_b = b.attach(shared(&key), manual_attach()).await.unwrap();

    set(&doc_a, "k1", "v1");
    a.sync(None).await.unwrap();
    b.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&doc_b));
    assert_eq!(sorted_json(&doc_a), sorted_json(&doc_b));

    set(&doc_b, "k2", "v2");
    b.sync(Some(&doc_b)).await.unwrap();
    a.sync(Some(&doc_a)).await.unwrap();
    assert_eq!(r#"{"k1":"v1","k2":"v2"}"#, sorted_json(&doc_a));
    assert_eq!(sorted_json(&doc_a), sorted_json(&doc_b));

    deactivate(&[&a, &b]).await;
}

/// JS "Can recover from temporary disconnect (manual sync)"
/// (client_test.ts:138).
#[tokio::test]
async fn can_recover_from_temporary_disconnect_in_manual_sync() {
    let Some(addr) = rpc_addr() else { return };
    let network = Arc::new(Network::default());
    let (c1, _) = stubbed_client(options(&addr), network.clone());
    let (c2, _) = stubbed_client(options(&addr), network.clone());
    c1.activate().await.unwrap();
    c2.activate().await.unwrap();
    let key = unique_key("disconnect-manual");
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();

    // Normal Condition
    set(&d2, "k1", "undefined");
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    // Simulate network error
    network.fail_with(failed_to_fetch());
    set(&d2, "k1", "v1");
    if let Err(err) = c2.sync(None).await {
        assert_eq!(failed_to_fetch(), err);
    }
    if let Err(err) = c1.sync(None).await {
        assert_eq!(failed_to_fetch(), err);
    }
    assert_eq!(r#"{"k1":"undefined"}"#, sorted_json(&d1));
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&d2));

    // Back to normal condition
    network.restore();
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Can recover from temporary disconnect (realtime sync)"
/// (client_test.ts:180). `integration_realtime.rs` covers a failed watch
/// stream; this is the JS case, where push-pull requests fail.
#[tokio::test]
async fn can_recover_from_temporary_disconnect_in_realtime_sync() {
    let Some(addr) = rpc_addr() else { return };
    let network = Arc::new(Network::default());
    let (c1, _) = stubbed_client(options(&addr), network.clone());
    let (c2, _) = stubbed_client(options(&addr), network.clone());
    c1.activate().await.unwrap();
    c2.activate().await.unwrap();
    let key = unique_key("disconnect-realtime");
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let events_d1 = collect_change_types(&d1);
    let events_d2 = collect_change_types(&d2);
    let sync1 = collect_sync_status(&d1);
    let sync2 = collect_sync_status(&d2);

    // Normal Condition
    set(&d2, "k1", "undefined");
    events_d2
        .wait_and_verify_nth(1, ChangeEventType::LocalChange)
        .await;
    events_d1
        .wait_and_verify_nth(1, ChangeEventType::RemoteChange)
        .await;
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    sync1.reset();
    sync2.reset();

    // Simulate network error
    network.fail_with(failed_to_fetch());
    set(&d2, "k1", "v1");

    events_d2
        .wait_and_verify_nth(2, ChangeEventType::LocalChange)
        .await;
    sync2.wait_for(DocSyncStatus::SyncFailed).await; // c2 should fail to sync

    if let Err(err) = c1.sync(None).await {
        assert_eq!(failed_to_fetch(), err); // c1 should also fail to sync
    }
    sync1.wait_for(DocSyncStatus::SyncFailed).await;
    assert_eq!(r#"{"k1":"undefined"}"#, sorted_json(&d1));
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&d2));

    // Back to normal condition
    sync1.reset();
    sync2.reset();
    network.restore();

    sync1.wait_for(DocSyncStatus::Synced).await; // wait for c1 to sync
    sync2.wait_for(DocSyncStatus::Synced).await;
    events_d1
        .wait_and_verify_nth(2, ChangeEventType::RemoteChange)
        .await;
    // d1 should be able to receive d2's update
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&d1));

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    deactivate(&[&c1, &c2]).await;
}

/// JS "Can change sync mode in realtime" (client_test.ts:329).
#[tokio::test]
async fn can_change_sync_mode_in_realtime() {
    // |    | Step1    | Step2    | Step3    | Step4    |
    // | c1 | PushPull | PushOnly | SyncOff  | PushPull |
    // | c2 | PushPull | SyncOff  | PushOnly | PushPull |
    // | c3 | PushPull | PushPull | PushPull | PushPull |
    use ChangeEventType::{LocalChange, RemoteChange};
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;
    let key = unique_key("change-sync-mode-in-realtime");

    // 01. c1, c2, c3 attach to the same document in realtime sync.
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d3 = c3
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let events_d1 = collect_change_types(&d1);
    let events_d2 = collect_change_types(&d2);
    let events_d3 = collect_change_types(&d3);

    // 02. [Step1] c1, c2, c3 sync in realtime.
    set(&d1, "c1", 0);
    set(&d2, "c2", 0);
    set(&d3, "c3", 0);
    events_d1.wait_and_verify_nth(1, LocalChange).await;
    events_d1.wait_and_verify_nth(2, RemoteChange).await;
    events_d1.wait_and_verify_nth(3, RemoteChange).await;
    events_d2.wait_and_verify_nth(1, LocalChange).await;
    events_d2.wait_and_verify_nth(2, RemoteChange).await;
    events_d2.wait_and_verify_nth(3, RemoteChange).await;
    events_d3.wait_and_verify_nth(1, LocalChange).await;
    events_d3.wait_and_verify_nth(2, RemoteChange).await;
    events_d3.wait_and_verify_nth(3, RemoteChange).await;
    assert_eq!(r#"{"c1":0,"c2":0,"c3":0}"#, sorted_json(&d1), "d1");
    assert_eq!(r#"{"c1":0,"c2":0,"c3":0}"#, sorted_json(&d2), "d2");
    assert_eq!(r#"{"c1":0,"c2":0,"c3":0}"#, sorted_json(&d3), "d3");

    // 03. [Step2] c1 sync with push-only mode, c2 sync with sync-off mode.
    // c3 can get the changes of c1 and c2, because c3 sync with push-pull mode.
    c1.change_sync_mode(&d1, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    c2.change_sync_mode(&d2, SyncMode::RealtimeSyncOff)
        .await
        .unwrap();
    set(&d1, "c1", 1);
    set(&d2, "c2", 1);
    set(&d3, "c3", 1);

    events_d1.wait_and_verify_nth(4, LocalChange).await;
    events_d2.wait_and_verify_nth(4, LocalChange).await;
    events_d3.wait_and_verify_nth(4, LocalChange).await;
    events_d3.wait_and_verify_nth(5, RemoteChange).await;
    assert_eq!(r#"{"c1":1,"c2":0,"c3":0}"#, sorted_json(&d1), "d1");
    assert_eq!(r#"{"c1":0,"c2":1,"c3":0}"#, sorted_json(&d2), "d2");
    assert_eq!(r#"{"c1":1,"c2":0,"c3":1}"#, sorted_json(&d3), "d3");

    // 04. [Step3] c1 sync with sync-off mode, c2 sync with push-only mode.
    c1.change_sync_mode(&d1, SyncMode::RealtimeSyncOff)
        .await
        .unwrap();
    c2.change_sync_mode(&d2, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    set(&d1, "c1", 2);
    set(&d2, "c2", 2);
    set(&d3, "c3", 2);

    events_d1.wait_and_verify_nth(5, LocalChange).await;
    events_d2.wait_and_verify_nth(5, LocalChange).await;
    events_d3.wait_and_verify_nth(6, LocalChange).await;
    events_d3.wait_and_verify_nth(7, RemoteChange).await;
    events_d3.wait_and_verify_nth(8, RemoteChange).await;
    assert_eq!(r#"{"c1":2,"c2":0,"c3":0}"#, sorted_json(&d1), "d1");
    assert_eq!(r#"{"c1":0,"c2":2,"c3":0}"#, sorted_json(&d2), "d2");
    assert_eq!(r#"{"c1":1,"c2":2,"c3":2}"#, sorted_json(&d3), "d3");

    // 05. [Step4] c1 and c2 sync with push-pull mode.
    c1.change_sync_mode(&d1, SyncMode::Realtime).await.unwrap();
    c2.change_sync_mode(&d2, SyncMode::Realtime).await.unwrap();
    for n in 6..=9 {
        events_d1.wait_and_verify_nth(n, RemoteChange).await;
    }
    for n in 6..=9 {
        events_d2.wait_and_verify_nth(n, RemoteChange).await;
    }
    events_d3.wait_and_verify_nth(9, RemoteChange).await;
    assert_eq!(r#"{"c1":2,"c2":2,"c3":2}"#, sorted_json(&d1), "d1");
    assert_eq!(r#"{"c1":2,"c2":2,"c3":2}"#, sorted_json(&d2), "d2");
    assert_eq!(r#"{"c1":2,"c2":2,"c3":2}"#, sorted_json(&d3), "d3");

    deactivate(&[&c1, &c2, &c3]).await;
}

/// JS "Should apply previous changes when switching to realtime sync"
/// (client_test.ts:462).
#[tokio::test]
async fn applies_previous_changes_when_switching_to_realtime_sync() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("previous-changes-to-realtime");
    let d2 = shared(&key);
    let events = collect_sync_status(&d2);

    // 01. c2 attach the doc with realtime sync mode at first.
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(d2, AttachOptions::default()).await.unwrap();
    set(&d1, "version", "v1");
    c1.sync(None).await.unwrap();
    assert_eq!(r#"{"version":"v1"}"#, sorted_json(&d1), "d1");
    events.wait_for(DocSyncStatus::Synced).await;
    assert_eq!(r#"{"version":"v1"}"#, sorted_json(&d2), "d2");

    // 02. c2 is changed to manual sync. So, c2 doesn't get the changes of c1.
    c2.change_sync_mode(&d2, SyncMode::Manual).await.unwrap();
    set(&d1, "version", "v2");
    c1.sync(None).await.unwrap();
    assert_eq!(r#"{"version":"v2"}"#, sorted_json(&d1), "d1");
    assert_eq!(r#"{"version":"v1"}"#, sorted_json(&d2), "d2");

    // 03. c2 is changed to realtime sync. c2 should be able to apply changes
    // made to the document while c2 is not in realtime sync.
    events.reset();
    c2.change_sync_mode(&d2, SyncMode::Realtime).await.unwrap();

    events.wait_for(DocSyncStatus::Synced).await;
    assert_eq!(r#"{"version":"v2"}"#, sorted_json(&d2), "d2");

    // 04. c2 should automatically synchronize changes.
    events.reset();
    set(&d1, "version", "v3");
    c1.sync(None).await.unwrap();

    events.wait_for(DocSyncStatus::Synced).await;
    assert_eq!(r#"{"version":"v3"}"#, sorted_json(&d1), "d1");
    assert_eq!(r#"{"version":"v3"}"#, sorted_json(&d2), "d2");

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should not include changes applied in push-only mode when switching
/// to realtime sync" (client_test.ts:523).
#[tokio::test]
async fn excludes_changes_pushed_in_push_only_mode_when_switching_to_realtime() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;

    // 01. cli attach to the document having counter.
    let d1 = c1
        .attach(
            shared(&unique_key("push-only-to-realtime")),
            manual_attach(),
        )
        .await
        .unwrap();

    // 02. cli update the document with creating a counter
    //     and sync with push-pull mode: CP(1, 1) -> CP(2, 2)
    create_counter(&d1);

    let checkpoint = d1.lock().unwrap().checkpoint();
    assert_eq!(1, checkpoint.client_seq());
    assert_eq!(1, checkpoint.server_seq());

    c1.sync(None).await.unwrap();
    let checkpoint = d1.lock().unwrap().checkpoint();
    assert_eq!(2, checkpoint.client_seq());
    assert_eq!(2, checkpoint.server_seq());

    // 03. cli update the document with increasing the counter(0 -> 1)
    //     and sync with push-only mode: CP(2, 2) -> CP(3, 2)
    let events = collect_sync_status(&d1);
    increase_counter(&d1);
    let change_pack = d1.lock().unwrap().create_change_pack();
    assert_eq!(1, change_pack.change_size());
    c1.change_sync_mode(&d1, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    events.wait_for(DocSyncStatus::Synced).await;
    let checkpoint = d1.lock().unwrap().checkpoint();
    assert_eq!(3, checkpoint.client_seq());
    assert_eq!(2, checkpoint.server_seq());
    c1.change_sync_mode(&d1, SyncMode::Manual).await.unwrap();

    // 04. cli update the document with increasing the counter(1 -> 2)
    //     and sync with push-pull mode. CP(3, 2) -> CP(4, 4)
    increase_counter(&d1);

    // The previous increase(0 -> 1) is already pushed to the server,
    // so the ChangePack of the request only has the increase(1 -> 2).
    let change_pack = d1.lock().unwrap().create_change_pack();
    assert_eq!(1, change_pack.change_size());

    c1.sync(None).await.unwrap();
    let checkpoint = d1.lock().unwrap().checkpoint();
    assert_eq!(4, checkpoint.client_seq());
    assert_eq!(4, checkpoint.server_seq());
    assert_eq!(CounterValue::Integer(2), counter(&d1));

    deactivate(&[&c1]).await;
}

/// The body of JS "Should prevent remote changes in push-only mode" and
/// "... in sync-off mode": c2 pauses with `pause_mode` while a sync of its
/// own is in flight, must not apply remote changes while paused, and
/// converges after resuming. JS keeps the pause for `pause_window`.
async fn prevents_remote_changes_while_paused(
    name: &str,
    pause_mode: SyncMode,
    pause_window: Duration,
) {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key(name);
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    let events_d1 = collect_change_types(&d1);
    let events_d2 = collect_change_types(&d2);

    set_two_paragraphs(&d1);
    events_d2
        .wait_and_verify_nth(1, ChangeEventType::RemoteChange)
        .await;

    assert_eq!("<doc><p>12</p><p>34</p></doc>", xml(&d1));
    assert_eq!("<doc><p>12</p><p>34</p></doc>", xml(&d2));

    edit_tree(&d1, |t| t.edit(2, 2, vec![text("a")], 0));
    c1.sync(None).await.unwrap();

    // Simulate the situation in the runSyncLoop where a pushpull request has
    // been sent but a response has not yet been received. The spawned sync
    // runs up to its request before the mode change queues behind it.
    let in_flight = tokio::spawn({
        let c2 = c2.clone();
        async move { c2.sync(None).await }
    });
    tokio::task::yield_now().await;

    // In the paused mode, remote-change events should not occur.
    c2.change_sync_mode(&d2, pause_mode).await.unwrap();
    let remote_change_occurred = Arc::new(AtomicBool::new(false));
    let unsub = d2.lock().unwrap().subscribe({
        let occurred = remote_change_occurred.clone();
        move |event| {
            if matches!(event, DocEvent::RemoteChange(_)) {
                occurred.store(true, Ordering::SeqCst);
            }
        }
    });
    // Keep the paused state. An absence can only be checked over a window.
    tokio::time::sleep(pause_window).await;
    d2.lock().unwrap().unsubscribe(unsub);
    assert!(!remote_change_occurred.load(Ordering::SeqCst));
    in_flight.await.unwrap().unwrap();

    c2.change_sync_mode(&d2, SyncMode::Realtime).await.unwrap();

    edit_tree(&d2, |t| t.edit(2, 2, vec![text("b")], 0));
    events_d1
        .wait_and_verify_nth(3, ChangeEventType::RemoteChange)
        .await;

    assert_eq!("<doc><p>1ba2</p><p>34</p></doc>", xml(&d1));
    assert_eq!("<doc><p>1ba2</p><p>34</p></doc>", xml(&d2));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should prevent remote changes in push-only mode"
/// (client_test.ts:588).
#[tokio::test]
async fn prevents_remote_changes_in_push_only_mode() {
    prevents_remote_changes_while_paused(
        "prevent-remote-push-only",
        SyncMode::RealtimePushOnly,
        Duration::from_millis(100),
    )
    .await;
}

/// JS "Should prevent remote changes in sync-off mode"
/// (client_test.ts:671).
#[tokio::test]
async fn prevents_remote_changes_in_sync_off_mode() {
    prevents_remote_changes_while_paused(
        "prevent-remote-sync-off",
        SyncMode::RealtimeSyncOff,
        Duration::ZERO,
    )
    .await;
}

/// JS "Should avoid unnecessary syncs in push-only mode"
/// (client_test.ts:748).
#[tokio::test]
async fn avoids_unnecessary_syncs_in_push_only_mode() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("unnecessary-syncs-push-only");
    let (d1, d2) = (shared(&key), shared(&key));
    let events_d1 = collect_sync_status(&d1);
    let events_d2 = collect_sync_status(&d2);

    let d1 = c1.attach(d1, AttachOptions::default()).await.unwrap();
    let d2 = c2.attach(d2, AttachOptions::default()).await.unwrap();
    events_d1
        .wait_and_verify_nth(1, DocSyncStatus::Synced)
        .await;

    d1.lock()
        .unwrap()
        .update(|root| root.set_text("t")?.edit(0, 0, "a", None).map(|_| ()))
        .unwrap();
    events_d2
        .wait_and_verify_nth(2, DocSyncStatus::Synced)
        .await;

    assert_eq!("a", text_value(&d1));
    assert_eq!("a", text_value(&d2));

    events_d1.reset();
    c1.change_sync_mode(&d1, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    edit_text(&d2, 1, "b");
    events_d2
        .wait_and_verify_nth(2, DocSyncStatus::Synced)
        .await;
    edit_text(&d2, 2, "c");
    events_d2
        .wait_and_verify_nth(3, DocSyncStatus::Synced)
        .await;

    assert_eq!(0, events_d1.len());
    c1.change_sync_mode(&d1, SyncMode::Realtime).await.unwrap();
    events_d1
        .wait_and_verify_nth(1, DocSyncStatus::Synced)
        .await;

    assert_eq!("abc", text_value(&d1));
    assert_eq!("abc", text_value(&d2));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should handle each request one by one" (client_test.ts:802).
#[tokio::test]
async fn handles_each_request_one_by_one() {
    let Some(addr) = rpc_addr() else { return };
    for i in 0..10 {
        let cli = active_client(&addr).await;
        let doc = cli
            .attach(
                shared(&unique_key(&format!("one-by-one-{i}"))),
                AttachOptions::default(),
            )
            .await
            .unwrap();

        cli.detach(&doc, DetachOptions).await.unwrap();
        deactivate(&[&cli]).await;
    }
}

/// JS "Should retry on network failure and eventually succeed"
/// (client_test.ts:817). JS checks the loop condition after a fixed 30ms;
/// this waits for the failed attempts instead.
#[tokio::test]
async fn retries_on_network_failure() {
    let Some(addr) = rpc_addr() else { return };
    let network = Arc::new(Network::default());
    let (cli, _) = stubbed_client(
        ClientOptions {
            retry_sync_loop_delay: Duration::from_millis(10),
            ..options(&addr)
        },
        network.clone(),
    );
    cli.activate().await.unwrap();
    let doc = cli
        .attach(
            shared(&unique_key("retry-network-failure")),
            AttachOptions::default(),
        )
        .await
        .unwrap();

    // 01. Simulate Unknown error.
    network.fail_with(rpc_error("unknown", "Failed to fetch"));
    doc.lock()
        .unwrap()
        .update(|root| root.set_text("t")?.edit(0, 0, "a", None).map(|_| ()))
        .unwrap();

    eventually("the sync loop to retry", || network.failed() >= 2).await;
    assert!(cli.condition(ClientCondition::SyncLoop));

    // 02. Simulate FailedPrecondition error.
    network.fail_with(rpc_error("failed_precondition", "Failed to fetch"));

    eventually("the sync loop to stop", || {
        !cli.condition(ClientCondition::SyncLoop)
    })
    .await;

    network.restore();
    deactivate(&[&cli]).await;
}

/// JS "Should handle local changes correctly when receiving snapshot"
/// (client_test.ts:853).
#[tokio::test]
async fn handles_local_changes_when_receiving_a_snapshot() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "local-changes-with-snapshot").await;
    create_counter(&t.d1);
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    // 01. c1 increases the counter for creating snapshot.
    for _ in 0..SNAPSHOT_THRESHOLD {
        increase_counter(&t.d1);
    }
    t.c1.sync(None).await.unwrap();

    // 02. c2 receives the snapshot and increases the counter simultaneously.
    // The spawned sync sends its request before the increase is made.
    let in_flight = tokio::spawn({
        let c2 = t.c2.clone();
        async move { c2.sync(None).await }
    });
    tokio::task::yield_now().await;
    increase_counter(&t.d2);

    t.c2.sync(None).await.unwrap();
    in_flight.await.unwrap().unwrap();
    t.c1.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Should not apply a snapshot pulled while in push-only mode"
/// (client_test.ts:878).
#[tokio::test]
async fn does_not_apply_a_snapshot_pulled_in_push_only_mode() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, transport) = stubbed_client(options(&addr), Arc::default());
    c1.activate().await.unwrap();
    let c2 = active_client(&addr).await;
    let key = unique_key("snapshot-in-push-only");
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();

    create_counter(&d1);
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();

    c1.change_sync_mode(&d1, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    let snapshots = Arc::new(AtomicUsize::new(0));
    d1.lock().unwrap().subscribe({
        let snapshots = snapshots.clone();
        move |event| {
            if matches!(event, DocEvent::Snapshot) {
                snapshots.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    // Record what the server actually answered c1 with. Without this the
    // test cannot tell a dropped snapshot from a dropped change list, and
    // would keep passing while the snapshot branch never runs.
    let resp_snapshot_bytes = Arc::new(Mutex::new(Vec::new()));
    let hook: PushPullHook = {
        let resp_snapshot_bytes = resp_snapshot_bytes.clone();
        Arc::new(move |_, response| {
            let len = response.change_pack.snapshot().map_or(0, <[u8]>::len);
            resp_snapshot_bytes.lock().unwrap().push(len);
            Box::pin(async {})
        })
    };
    transport.set_hook(hook);

    // 01. c2 makes enough changes for the server to answer c1 with a
    // snapshot instead of changes.
    for _ in 0..SNAPSHOT_THRESHOLD {
        increase_counter(&d2);
    }
    c2.sync(None).await.unwrap();

    // 02. An explicit sync always pulls, even in push-only mode. The
    // snapshot it brings back must be dropped like changes are.
    increase_counter(&d1);
    c1.sync(Some(&d1)).await.unwrap();
    // The response carried a snapshot, so the drop below is the snapshot
    // branch and not the pre-existing changes branch.
    assert!(
        resp_snapshot_bytes
            .lock()
            .unwrap()
            .iter()
            .any(|len| *len > 0),
        "the server answered with a snapshot"
    );
    assert_eq!(0, snapshots.load(Ordering::SeqCst));
    assert_eq!(CounterValue::Integer(1), counter(&d1));
    // The push landed, so it must not be sent again: a snapshot built for a
    // re-push would count the change twice.
    assert!(!d1.lock().unwrap().has_local_changes());

    // 03. The local change still reached the server.
    c2.sync(None).await.unwrap();
    assert_eq!(CounterValue::Integer(SNAPSHOT_THRESHOLD + 1), counter(&d2));

    // 04. Back in realtime, c1 catches up on what it dropped.
    c1.change_sync_mode(&d1, SyncMode::Realtime).await.unwrap();
    c1.sync(Some(&d1)).await.unwrap();
    assert_eq!(sorted_json(&d1), sorted_json(&d2));
    transport.clear_hook();

    deactivate(&[&c1, &c2]).await;
}

/// The body of JS "Should drop a pull already in flight when switching to
/// %s" (client_test.ts:944), run with `RealtimePushOnly` and
/// `RealtimeSyncOff`.
async fn drops_a_pull_in_flight_when_pausing(name: &str, pause_mode: SyncMode) {
    let Some(addr) = rpc_addr() else { return };
    // JS `withTwoClientsAndDocuments` also pins c1 to the lower actor. Only
    // c1 is stubbed here, so the order is left as is: the counter makes no
    // conflict for a tie-break to decide.
    let (c1, transport) = stubbed_client(options(&addr), Arc::default());
    c1.activate().await.unwrap();
    let c2 = active_client(&addr).await;
    let key = unique_key(name);
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    create_counter(&d1);
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();

    // Hold c1's next pull that brings changes back, after the server has
    // answered and before c1 handles the answer. This is the sync the pause
    // below has to wait behind on the task queue.
    let holding = Arc::new(AtomicBool::new(true));
    let entered_hold = Arc::new(Notify::new());
    let release_hold = Arc::new(Notify::new());
    let hook: PushPullHook = {
        let (holding, entered_hold, release_hold) =
            (holding.clone(), entered_hold.clone(), release_hold.clone());
        Arc::new(move |request, response| {
            let hold = !request.push_only
                && response.change_pack.has_changes()
                && holding.swap(false, Ordering::SeqCst);
            let (entered_hold, release_hold) = (entered_hold.clone(), release_hold.clone());
            Box::pin(async move {
                if hold {
                    entered_hold.notify_one();
                    release_hold.notified().await;
                }
            })
        })
    };
    transport.set_hook(hook);
    let remote_changes = Arc::new(AtomicUsize::new(0));
    d1.lock().unwrap().subscribe({
        let remote_changes = remote_changes.clone();
        move |event| {
            if matches!(event, DocEvent::RemoteChange(_)) {
                remote_changes.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    // 01. c2 changes the counter; c1 starts pulling it in realtime.
    increase_counter(&d2);
    c2.sync(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), entered_hold.notified())
        .await
        .expect("timed out: c1 pulls the change");

    // 02. While that pull is in flight, c1 asks to stop receiving (an IME
    // composition starts). Then the pull's answer arrives. The switch is
    // spawned and run up to the task queue, where it waits behind the pull.
    let switching = tokio::spawn({
        let (c1, d1) = (c1.clone(), d1.clone());
        async move { c1.change_sync_mode(&d1, pause_mode).await }
    });
    tokio::task::yield_now().await;
    release_hold.notify_one();
    switching.await.unwrap().unwrap();

    // 03. The pulled change must not be applied after the pause.
    assert_eq!(0, remote_changes.load(Ordering::SeqCst));
    assert_eq!(CounterValue::Integer(0), counter(&d1));

    // 04. Back in realtime, the sync loop picks up what was dropped by
    // itself.
    c1.change_sync_mode(&d1, SyncMode::Realtime).await.unwrap();
    eventually("c1 catches up in realtime", || {
        remote_changes.load(Ordering::SeqCst) > 0 && counter(&d1) == CounterValue::Integer(1)
    })
    .await;
    assert_eq!(sorted_json(&d1), sorted_json(&d2));

    release_hold.notify_one();
    transport.clear_hook();
    for (client, doc) in [(&c1, &d1), (&c2, &d2)] {
        client.detach(doc, DetachOptions).await.unwrap();
    }
    deactivate(&[&c1, &c2]).await;
}

/// JS "Should drop a pull already in flight when switching to %s" with
/// `RealtimePushOnly` (client_test.ts:944).
#[tokio::test]
async fn drops_a_pull_in_flight_when_switching_to_push_only() {
    drops_a_pull_in_flight_when_pausing("drop-pull-push-only", SyncMode::RealtimePushOnly).await;
}

/// JS "Should drop a pull already in flight when switching to %s" with
/// `RealtimeSyncOff` (client_test.ts:944).
#[tokio::test]
async fn drops_a_pull_in_flight_when_switching_to_sync_off() {
    drops_a_pull_in_flight_when_pausing("drop-pull-sync-off", SyncMode::RealtimeSyncOff).await;
}

/// JS "Should cancel watch stream when changing to manual sync mode"
/// (client_test.ts:1026).
#[tokio::test]
async fn cancels_watch_stream_when_changing_to_manual_sync_mode() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let key = unique_key("cancel-watch-to-manual");
    let d1 = c1
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    // Set up event collectors
    let events = collect_change_types(&d1);
    let presence_events = collect_presence(&d2);

    // Verify initial sync works
    set(&d2, "version", "v1");
    events.wait_for(ChangeEventType::RemoteChange).await;
    assert_eq!(r#"{"version":"v1"}"#, sorted_json(&d1));

    // Switch to manual mode and verify stream closes
    c1.change_sync_mode(&d1, SyncMode::Manual).await.unwrap();
    eventually("c2 to see c1 unwatched", || {
        presence_events
            .values()
            .iter()
            .any(|event| matches!(event, DocEvent::Unwatched(..)))
    })
    .await;

    // Verify stream stays closed: nothing may arrive within the window.
    set(&d2, "version", "v2");
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(r#"{"version":"v1"}"#, sorted_json(&d1), "keeps old value");

    // Verify manual sync still works
    c1.sync(None).await.unwrap();
    assert_eq!(r#"{"version":"v2"}"#, sorted_json(&d1), "manual sync");

    deactivate(&[&c1, &c2]).await;
}

/// JS "Should not produce document not attached error on detach during
/// sync" (client_test.ts:1072).
#[tokio::test]
async fn detach_during_sync_does_not_fail_with_not_attached() {
    let Some(addr) = rpc_addr() else { return };
    // Use a short sync loop to increase the chance of overlap.
    let c1 = Client::new(ClientOptions {
        sync_loop_duration: Duration::from_millis(50),
        reconnect_stream_delay: Duration::from_millis(1000),
        ..options(&addr)
    })
    .unwrap();
    c1.activate().await.unwrap();

    let doc = c1
        .attach(
            shared(&unique_key("detach-during-sync")),
            attach_in(SyncMode::Realtime),
        )
        .await
        .unwrap();

    // Make a local change so the sync loop has something to push.
    set(&doc, "key", "value");

    // Detach immediately; the sync loop may be trying to sync concurrently.
    c1.detach(&doc, DetachOptions).await.unwrap();

    // Verify the document is properly detached without errors.
    assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());

    deactivate(&[&c1]).await;
}

/// Review Focus 5. JS `deactivateInternal` detaches every attachment and
/// sets each document to `Detached`; JS `deactivate` stops the sync loop and
/// `detachInternal` cancels each watch stream.
#[tokio::test]
async fn deactivate_detaches_documents_and_stops_tasks() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let keys = [
        unique_key("deactivate-detaches-1"),
        unique_key("deactivate-detaches-2"),
    ];
    let docs = vec![
        client
            .attach(shared(&keys[0]), manual_attach())
            .await
            .unwrap(),
        client
            .attach(shared(&keys[1]), AttachOptions::default())
            .await
            .unwrap(),
    ];
    // The sync loop and the realtime document's watch loop.
    assert_eq!(2, client.running_task_count());

    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();

    assert!(!client.is_active());
    for (key, doc) in keys.iter().zip(&docs) {
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert!(!client.has(key));
    }
    eventually("background loops to stop", || {
        client.running_task_count() == 0
    })
    .await;
}
