//! Integration test that a push-only client does not collect garbage.
//!
//! JS reference: `packages/sdk/test/integration/pushonly_gc_test.ts`. Run
//! with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::tree::{edit_tree, element, set_tree, text, xml};
use integration_support::two_clients::{active_client, manual_attach, shared};
use integration_support::{eventually, rpc_addr, unique_key};
use yorkie::{AttachOptions, DeactivateOptions, DetachOptions, SharedDocument, SyncMode};

/// Waits until the sync loop has pushed every local change of `doc`, i.e.
/// until a push-only round trip for it has completed (JS `pushed`).
async fn pushed(doc: &SharedDocument) {
    eventually("the push to land", || {
        !doc.lock().unwrap().has_local_changes()
    })
    .await;
}

/// JS "must not collect a tombstone a deferred remote change anchors on"
/// (pushonly_gc_test.ts:58).
///
/// A push-only client pushes but does not pull, and every response still
/// carries the server's minimum version vector. Collecting with it would
/// purge tombstones while the remote changes anchored on them are exactly
/// what the client has not pulled yet, leaving it unable to apply them once
/// it resumes.
#[tokio::test]
async fn does_not_collect_a_tombstone_a_deferred_remote_change_anchors_on() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("pushonly-gc");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    // c1 syncs by hand so each of its pushes and pulls lands in a known order.
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    set_tree(&d1, element("doc", vec![element("p", vec![text("ab")])]));
    c1.sync(None).await.unwrap();
    eventually("c2 to see the tree", || {
        d2.lock().unwrap().get_root().get("t").is_some()
    })
    .await;

    // c2 composes: it pushes while pulling nothing.
    c2.change_sync_mode(&d2, SyncMode::RealtimePushOnly)
        .await
        .unwrap();
    edit_tree(&d2, |t| t.edit(2, 2, vec![text("X")], 0));
    pushed(&d2).await;
    c1.sync(None).await.unwrap();
    assert_eq!("<doc><p>aXb</p></doc>", xml(&d1));

    // c2 replaces its "X": the node becomes a tombstone on both sides.
    edit_tree(&d2, |t| t.edit(2, 3, vec![text("x")], 0));
    pushed(&d2).await;

    // c1 still sees "X" live, and inserts right after it: an edit anchored
    // on the node c2 just removed. Then two syncs: the first pushes the
    // insert and pulls the removal, the second reports a version vector that
    // covers the removal, so the server's minimum vector now does too.
    edit_tree(&d1, |t| t.edit(3, 3, vec![text("Y")], 0));
    c1.sync(None).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!("<doc><p>axYb</p></doc>", xml(&d1));

    // c2 keeps composing. The reply to this push carries a minimum vector
    // under which "X" is collectable, while the insert anchored on it is
    // still waiting on the server for c2 to pull.
    edit_tree(&d2, |t| t.edit(1, 1, vec![text("z")], 0));
    pushed(&d2).await;

    // The composition ends: c2 resumes and pulls the deferred insert.
    c2.change_sync_mode(&d2, SyncMode::Realtime).await.unwrap();
    c2.sync(Some(&d2)).await.unwrap();
    c1.sync(None).await.unwrap();
    assert_eq!("<doc><p>zaxYb</p></doc>", xml(&d2));
    assert_eq!(xml(&d1), xml(&d2));

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}
