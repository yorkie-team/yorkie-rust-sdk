//! Integration tests for documents in `Polling` sync mode against a live
//! server.
//!
//! JS reference: `packages/sdk/test/integration/document_polling_test.ts`.
//! JS checks the value after a fixed wait; these wait for it up to the same
//! bound instead, so a change that arrives later than JS allows still fails.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::two_clients::{active_client, shared};
use integration_support::{eventually_within, rpc_addr, unique_key};
use std::time::Duration;
use yorkie::{
    AttachOptions, DeactivateOptions, DetachOptions, ErrorCode, JsonValue, SharedDocument, SyncMode,
};

fn polling(interval: Duration) -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Polling,
        document_poll_interval: Some(interval),
        ..AttachOptions::default()
    }
}

fn set_k(doc: &SharedDocument, value: &str) {
    doc.lock()
        .unwrap()
        .update(|root| root.set("k", value).map(|_| ()))
        .unwrap();
}

fn has_k(doc: &SharedDocument, value: &str) -> bool {
    matches!(
        doc.lock().unwrap().get_root().get("k"),
        Some(JsonValue::String(k)) if k == value
    )
}

/// JS "Polling document receives remote changes within poll interval"
/// (document_polling_test.ts:27).
#[tokio::test]
async fn polling_document_receives_remote_changes_within_the_interval() {
    let Some(addr) = rpc_addr() else { return };
    let doc_key = unique_key("polling-receives");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    // c1 attaches in Polling mode with 200ms interval.
    let d1 = c1
        .attach(shared(&doc_key), polling(Duration::from_millis(200)))
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&doc_key), AttachOptions::default())
        .await
        .unwrap();

    // c2 makes a change.
    set_k(&d2, "v");
    c2.sync(None).await.unwrap();

    // Within at least 2 polling ticks (~400ms); JS waits 600ms.
    eventually_within("c1 to poll the change", Duration::from_millis(600), || {
        has_k(&d1, "v")
    })
    .await;

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "changeSyncMode transitions Polling → Realtime for documents"
/// (document_polling_test.ts:64).
#[tokio::test]
async fn change_sync_mode_from_polling_to_realtime() {
    let Some(addr) = rpc_addr() else { return };
    let doc_key = unique_key("polling-to-realtime");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    // c1 starts in Polling mode with a slow interval (proves the Realtime
    // path is what delivers the change, not the polling tick).
    let d1 = c1
        .attach(shared(&doc_key), polling(Duration::from_millis(5000)))
        .await
        .unwrap();
    let d2 = c2
        .attach(shared(&doc_key), AttachOptions::default())
        .await
        .unwrap();

    // Switch c1 to Realtime, which opens a watch stream.
    c1.change_sync_mode(&d1, SyncMode::Realtime).await.unwrap();

    // c2 makes a change.
    set_k(&d2, "rt");
    c2.sync(None).await.unwrap();

    // Realtime should deliver well within 1s (much faster than the 5s
    // polling interval that's still configured); JS waits 800ms.
    eventually_within(
        "c1 to receive the change",
        Duration::from_millis(800),
        || has_k(&d1, "rt"),
    )
    .await;

    c1.detach(&d1, DetachOptions).await.unwrap();
    c2.detach(&d2, DetachOptions).await.unwrap();
    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "attach rejects documentPollInterval: 0 with ErrInvalidArgument"
/// (document_polling_test.ts:106).
#[tokio::test]
async fn attach_rejects_a_zero_document_poll_interval() {
    let Some(addr) = rpc_addr() else { return };
    let c = active_client(&addr).await;

    let err = c
        .attach(
            shared(&unique_key("polling-zero-interval")),
            AttachOptions {
                document_poll_interval: Some(Duration::ZERO),
                ..AttachOptions::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(Some(ErrorCode::ErrInvalidArgument), err.code(), "{err:?}");
    assert!(
        err.to_string()
            .contains("document_poll_interval must be greater than 0"),
        "{err}"
    );

    c.deactivate(DeactivateOptions::default()).await.unwrap();
}
