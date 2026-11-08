//! Integration tests for the duplicate attach guard against a live server.
//!
//! JS reference: `packages/sdk/test/integration/duplicate_attach_test.ts`.
//! A duplicate attach of the same key on one client must be rejected locally
//! with `ErrAlreadyAttached`, and must not deactivate the client: reaching
//! the server would produce a misleading `ErrClientNotFound`, which the SDK
//! escalates to a full client deactivation.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::two_clients::{active_client, shared};
use integration_support::{rpc_addr, unique_key};
use yorkie::{
    AttachOptions, ClientResult, DeactivateOptions, DetachOptions, ErrorCode, SharedDocument,
};

/// Awaits an attach the way a caller does in try/catch and checks it is
/// rejected with the document-scoped `ErrAlreadyAttached` (JS
/// `attachExpectingAlreadyAttached`).
async fn attach_expecting_already_attached(
    attach: impl std::future::Future<Output = ClientResult<SharedDocument>>,
) {
    match attach.await {
        Ok(_) => panic!("expected duplicate attach to throw ErrAlreadyAttached"),
        Err(err) => assert_eq!(Some(ErrorCode::ErrAlreadyAttached), err.code(), "{err:?}"),
    }
}

/// JS "rejects re-attaching an already attached key without killing the
/// client" (duplicate_attach_test.ts:49).
#[tokio::test]
async fn rejects_reattaching_an_attached_key_without_killing_the_client() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let doc_key = unique_key("duplicate-attach");

    let doc1 = client
        .attach(shared(&doc_key), AttachOptions::default())
        .await
        .unwrap();

    attach_expecting_already_attached(client.attach(shared(&doc_key), AttachOptions::default()))
        .await;

    assert!(client.is_active(), "client must stay active");
    assert!(client.has(&doc_key), "original attachment is intact");

    client.detach(&doc1, DetachOptions).await.unwrap();
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "rejects a concurrent attach of the same key while the first is in
/// flight" (duplicate_attach_test.ts:69).
#[tokio::test(flavor = "current_thread")]
async fn rejects_a_concurrent_attach_of_the_same_key() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let doc_key = unique_key("duplicate-attach-in-flight");

    // Fire the first attach without awaiting it, and let it run up to its
    // request: the key is not attached yet, so the guard must also track the
    // attach in flight.
    // Under the current_thread runtime, one `yield_now` runs the spawned
    // task up to its first pending await before this task goes on. The
    // ordering depends on that: do not switch this test to multi_thread.
    let attach1 = tokio::spawn({
        let (client, doc1) = (client.clone(), shared(&doc_key));
        async move { client.attach(doc1, AttachOptions::default()).await }
    });
    tokio::task::yield_now().await;

    attach_expecting_already_attached(client.attach(shared(&doc_key), AttachOptions::default()))
        .await;

    // The first attach still completes successfully.
    let doc1 = attach1.await.unwrap().unwrap();
    assert!(client.is_active(), "client must stay active");
    assert!(client.has(&doc_key), "first attach succeeded");

    client.detach(&doc1, DetachOptions).await.unwrap();
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}
