//! Integration tests for the maximum document size against a live server.
//!
//! JS reference: `packages/sdk/test/integration/document_limit_test.ts`.
//! JS sets the limit on a project through the admin API, and the client
//! takes it from the attach response. The admin API is out of scope here:
//! these set the same limit with `Document::set_max_size_per_document`
//! after attaching, so `should successfully assign size limit to document`
//! is not ported.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::tree::TwoClients;
use yorkie::{DataSize, JsonText, SharedDocument, YorkieError};

const SIZE_LIMIT: usize = 100;

fn set_limit(doc: &SharedDocument) {
    doc.lock().unwrap().set_max_size_per_document(SIZE_LIMIT);
}

fn edit_text(doc: &SharedDocument, content: &str) -> yorkie::Result<()> {
    doc.lock().unwrap().update(|root| {
        root.get_text_mut("text")?
            .edit(0, 0, content, None)
            .map(|_| ())
    })
}

fn live(doc: &SharedDocument) -> DataSize {
    doc.lock().unwrap().doc_size().live
}

fn total(doc: &SharedDocument) -> usize {
    doc.lock().unwrap().doc_size().total()
}

#[tokio::test]
async fn rejects_local_update_that_exceeds_document_size_limit() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "size-limit").await;
    set_limit(&t.d1);

    t.d1.lock()
        .unwrap()
        .update(|root| root.set("text", JsonText::new()).map(|_| ()))
        .unwrap();
    assert_eq!(DataSize { data: 0, meta: 72 }, live(&t.d1));

    let err = edit_text(&t.d1, "helloworld").unwrap_err();
    assert!(
        matches!(err, YorkieError::DocumentSizeExceedsLimit { .. }),
        "{err:?}"
    );
    assert!(
        err.to_string().starts_with("document size exceeded"),
        "{err}"
    );
    assert_eq!(72, total(&t.d1));

    t.deactivate().await;
}

#[tokio::test]
async fn allows_remote_updates_even_if_they_exceed_document_size_limit() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "size-remote").await;
    set_limit(&t.d1);
    set_limit(&t.d2);

    t.d1.lock()
        .unwrap()
        .update(|root| root.set("text", JsonText::new()).map(|_| ()))
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(72, total(&t.d1));

    edit_text(&t.d1, "aa").unwrap();
    assert_eq!(DataSize { data: 4, meta: 96 }, live(&t.d1));
    t.c1.sync(None).await.unwrap();

    edit_text(&t.d2, "a").unwrap();
    assert_eq!(DataSize { data: 2, meta: 96 }, live(&t.d2));

    // Pulling the remote edit takes d2 past the limit, which is allowed.
    t.c2.sync(None).await.unwrap();
    assert_eq!(DataSize { data: 6, meta: 120 }, live(&t.d2));

    t.c1.sync(None).await.unwrap();
    assert_eq!(DataSize { data: 6, meta: 120 }, live(&t.d1));

    // A local update is still refused.
    let err = edit_text(&t.d1, "a").unwrap_err();
    assert!(
        matches!(err, YorkieError::DocumentSizeExceedsLimit { .. }),
        "{err:?}"
    );

    t.deactivate().await;
}
