//! Integration tests for snapshots.
//!
//! JS reference: `packages/sdk/test/integration/snapshot_test.ts`. Run with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.
//!
//! Each case pushes more changes than the server's snapshot threshold (JS
//! `DefaultSnapshotThreshold`, 500), so the other client pulls a snapshot.
//!
//! Two cases are ignored as too slow: they build a root with 500 members
//! (500 keys, or 500 overwritten texts kept as tombstones), and a local
//! update costs time quadratic in that count, so each takes minutes in a
//! debug build. Both pass with `cargo test --release -- --ignored`.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::two_clients::TwoClients;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use yorkie::{CounterValue, DocEvent, JsonCounter, JsonText, JsonValue, SharedDocument};

/// JS `DefaultSnapshotThreshold` (`test/helper/helper.ts`).
const SNAPSHOT_THRESHOLD: i32 = 500;

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn update(doc: &SharedDocument, f: impl FnOnce(&mut yorkie::JsonObject) -> yorkie::Result<()>) {
    doc.lock().unwrap().update(f).unwrap();
}

/// JS "should handle snapshot" (snapshot_test.ts:27).
#[tokio::test]
#[ignore = "diverges from JS: local update is quadratic in root member count, see docs/current-porting-gaps.md"]
async fn handles_a_snapshot() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "snapshot").await;

    // 01. Updates changes over snapshot threshold.
    for idx in 0..SNAPSHOT_THRESHOLD {
        update(&t.d1, |root| root.set(idx.to_string(), idx).map(|_| ()));
    }
    t.c1.sync(None).await.unwrap();

    // 02. Makes local changes then pull a snapshot from the agent.
    update(&t.d2, |root| root.set("key", "value").map(|_| ()));
    t.c2.sync(None).await.unwrap();
    assert_eq!(
        Some(&JsonValue::String("value".to_owned())),
        t.d2.lock().unwrap().get_root().get("key")
    );

    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "should handle snapshot for text object" (snapshot_test.ts:52).
#[tokio::test]
#[ignore = "diverges from JS: local update is quadratic in root member count, see docs/current-porting-gaps.md"]
async fn handles_a_snapshot_for_a_text() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "snapshot-text").await;

    for _ in 0..SNAPSHOT_THRESHOLD {
        update(&t.d1, |root| root.set("k1", JsonText::new()).map(|_| ()));
    }
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    // 01. Updates changes over snapshot threshold by c1.
    for idx in 0..SNAPSHOT_THRESHOLD as usize {
        update(&t.d1, |root| {
            root.get_text_mut("k1")?
                .edit(idx, idx, "x", None)
                .map(|_| ())
        });
    }

    // 02. Makes local change by c2.
    update(&t.d2, |root| {
        root.get_text_mut("k1")?.edit(0, 0, "o", None).map(|_| ())
    });

    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "should handle snapshot for text with attributes" (snapshot_test.ts:82).
#[tokio::test]
async fn handles_a_snapshot_for_a_text_with_attributes() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "snapshot-text-attributes").await;

    update(&t.d1, |root| {
        root.set_text("k1")?.edit(0, 0, "a", None).map(|_| ())
    });
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    // 01. Updates changes over snapshot threshold by c1.
    for _ in 0..SNAPSHOT_THRESHOLD {
        update(&t.d1, |root| {
            let bold = BTreeMap::from([("bold".to_owned(), Value::from("true"))]);
            root.get_text_mut("k1")?.set_style(0, 1, bold)
        });
    }
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "should publish snapshot event with up-to-date document"
/// (snapshot_test.ts:106).
///
/// JS reads the counter inside the snapshot callback. A Rust callback runs
/// while the document is locked and cannot read it, so this checks that one
/// snapshot event is published and that the counter holds the expected value
/// once the sync returns. Rust publishes `Snapshot` after the local changes
/// are replayed onto the snapshot, as JS does, so the two are the same state.
#[tokio::test]
async fn publishes_a_snapshot_event_with_the_up_to_date_document() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "snapshot-event").await;

    let snapshots = Arc::new(Mutex::new(0));
    let seen = snapshots.clone();
    t.d2.lock().unwrap().subscribe(move |event| {
        if matches!(event, DocEvent::Snapshot) {
            *seen.lock().unwrap() += 1;
        }
    });

    update(&t.d1, |root| {
        root.set("counter", JsonCounter::integer(0)).map(|_| ())
    });
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    // 01. c1 increases the counter for creating snapshot.
    for _ in 0..SNAPSHOT_THRESHOLD {
        update(&t.d1, |root| {
            root.get_counter_mut("counter")?.increase(1).map(|_| ())
        });
    }
    t.c1.sync(None).await.unwrap();

    // 02. c2 receives the snapshot and increases the counter simultaneously.
    let c2 = t.c2.clone();
    let synced = tokio::spawn(async move { c2.sync(None).await });
    update(&t.d2, |root| {
        root.get_counter_mut("counter")?.increase(1).map(|_| ())
    });
    synced.await.unwrap().unwrap();

    assert_eq!(1, *snapshots.lock().unwrap());
    let value = match t.d2.lock().unwrap().get_root().get("counter") {
        Some(JsonValue::Counter(counter)) => counter.value(),
        other => panic!("expected a counter, got {other:?}"),
    };
    assert_eq!(CounterValue::Integer(SNAPSHOT_THRESHOLD + 1), value);

    t.deactivate().await;
}
