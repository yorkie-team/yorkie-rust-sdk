//! Integration tests for the `disable_gc` attach option.
//!
//! JS reference: `packages/sdk/test/integration/disable_gc_test.ts` (31-260),
//! `describe('disableGC attach option')`. As in JS, the server side (no
//! minimum version vector kept for the client, no vector in its replies) is
//! the server's to test; these check that the option is wired through and
//! that Counter values converge when opted-in and opted-out clients mix.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::two_clients::{active_client, shared};
use integration_support::{deactivate, rpc_addr, unique_key};
use yorkie::{
    AttachOptions, CounterValue, DetachOptions, JsonCounter, JsonValue, SharedDocument, SyncMode,
};

fn attach_options(disable_gc: bool) -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        disable_gc,
        ..AttachOptions::default()
    }
}

fn update(doc: &SharedDocument, f: impl FnOnce(&mut yorkie::JsonObject) -> yorkie::Result<()>) {
    doc.lock().unwrap().update(f).unwrap();
}

fn create_counter(doc: &SharedDocument) {
    update(doc, |root| {
        root.set("counter", JsonCounter::integer(0)).map(|_| ())
    });
}

fn increase(doc: &SharedDocument) {
    update(doc, |root| {
        root.get_counter_mut("counter")?.increase(1).map(|_| ())
    });
}

fn counter(doc: &SharedDocument) -> CounterValue {
    match doc.lock().unwrap().get_root().get("counter") {
        Some(JsonValue::Counter(counter)) => counter.value(),
        other => panic!("expected a counter, got {other:?}"),
    }
}

fn vector_size(doc: &SharedDocument) -> usize {
    doc.lock().unwrap().version_vector().size()
}

/// JS "Can attach a Counter document with disableGC and sync without error"
/// (disable_gc_test.ts:32).
#[tokio::test]
async fn attaches_a_counter_document_with_disable_gc() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("disable-gc-attach");
    let c = active_client(&addr).await;

    let doc = c.attach(shared(&key), attach_options(true)).await.unwrap();
    update(&doc, |root| {
        root.set("counter", JsonCounter::integer(0))?;
        root.get_counter_mut("counter")?.increase(1)?;
        Ok(())
    });
    c.sync(None).await.unwrap();
    assert_eq!(CounterValue::Integer(1), counter(&doc));

    deactivate(&[&c]).await;
}

/// JS "Mixed opt-in and opt-out clients converge on the Counter value"
/// (disable_gc_test.ts:57).
#[tokio::test]
async fn mixed_opt_in_and_opt_out_clients_converge() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("disable-gc-mixed");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    let d1 = c1
        .attach(shared(&key), attach_options(false))
        .await
        .unwrap();
    create_counter(&d1);
    c1.sync(None).await.unwrap();

    let d2 = c2.attach(shared(&key), attach_options(true)).await.unwrap();

    increase(&d1);
    increase(&d2);

    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();

    assert_eq!(CounterValue::Integer(2), counter(&d1));
    assert_eq!(CounterValue::Integer(2), counter(&d2));

    deactivate(&[&c1, &c2]).await;
}

/// JS "Opt-out clients keep per-Change VV at size 1 under multi-actor
/// fanout" (disable_gc_test.ts:95): applying a remote change advances only
/// the lamport, so each opted-out document's version vector keeps its own
/// entry alone.
#[tokio::test]
async fn opt_out_clients_keep_the_version_vector_at_one_entry() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("disable-gc-fanout");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;

    let d1 = c1.attach(shared(&key), attach_options(true)).await.unwrap();
    let d2 = c2.attach(shared(&key), attach_options(true)).await.unwrap();
    let d3 = c3.attach(shared(&key), attach_options(true)).await.unwrap();

    create_counter(&d1);
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    c3.sync(None).await.unwrap();

    for _ in 0..3 {
        increase(&d1);
        increase(&d2);
        increase(&d3);
        c1.sync(None).await.unwrap();
        c2.sync(None).await.unwrap();
        c3.sync(None).await.unwrap();
    }
    // Drain the remaining pushed changes so every client sees them.
    c1.sync(None).await.unwrap();
    c2.sync(None).await.unwrap();
    c3.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();

    for (i, doc) in [&d1, &d2, &d3].into_iter().enumerate() {
        assert_eq!(CounterValue::Integer(9), counter(doc), "doc[{i}]");
        assert_eq!(
            1,
            vector_size(doc),
            "opt-out doc[{i}].versionVector must stay at size 1"
        );
    }

    deactivate(&[&c1, &c2, &c3]).await;
}

/// JS "Opt-out client picks up server lamport when attach returns a
/// snapshot" (disable_gc_test.ts:159). As in JS, 30 updates are pushed. The
/// server's snapshot threshold is 500 (Go `DefaultSnapshotThreshold`), so
/// despite its title this case takes the change path, in JS and here: the
/// attach returns the 30 changes and the lamport catches up one change at a
/// time. The snapshot path is covered by the `yorkie-core` unit tests
/// `an_opted_out_snapshot_attach_keeps_one_vector_entry` and
/// `an_opted_in_snapshot_attach_merges_later_remote_vectors`
/// (`document.rs`).
#[tokio::test]
async fn opt_out_client_picks_up_the_server_lamport_on_attach() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("disable-gc-snapshot");
    let c_a = active_client(&addr).await;

    let doc_a = c_a
        .attach(shared(&key), attach_options(false))
        .await
        .unwrap();
    for i in 0..30 {
        update(&doc_a, |root| root.set(format!("k{i}"), i).map(|_| ()));
    }
    c_a.sync(None).await.unwrap();

    let c_b = active_client(&addr).await;
    let doc_b = c_b
        .attach(shared(&key), attach_options(true))
        .await
        .unwrap();
    {
        let doc = doc_b.lock().unwrap();
        let lamport = doc
            .version_vector()
            .get(doc.actor_id().as_str())
            .unwrap_or_default();
        assert!(
            lamport >= 30,
            "opt-out client must catch up to server lamport, got {lamport}"
        );
        assert_eq!(
            1,
            doc.version_vector().size(),
            "opt-out doc.VV must stay size 1 after attach"
        );
    }

    deactivate(&[&c_b, &c_a]).await;
}

/// JS "Re-attach without disableGC restores normal sync behavior"
/// (disable_gc_test.ts:217).
#[tokio::test]
async fn reattach_without_disable_gc_restores_normal_sync() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("disable-gc-reattach");
    let c = active_client(&addr).await;

    // First attach: opt-out.
    let d1 = c.attach(shared(&key), attach_options(true)).await.unwrap();
    update(&d1, |root| {
        root.set("counter", JsonCounter::integer(0))?;
        root.get_counter_mut("counter")?.increase(1)?;
        Ok(())
    });
    c.sync(None).await.unwrap();
    c.detach(&d1, DetachOptions).await.unwrap();

    // Re-attach without the option; the client reads the flag from the
    // attachment, so this push-pull sends it off.
    let d2 = c.attach(shared(&key), attach_options(false)).await.unwrap();
    increase(&d2);
    c.sync(None).await.unwrap();
    assert_eq!(CounterValue::Integer(2), counter(&d2));

    deactivate(&[&c]).await;
}
