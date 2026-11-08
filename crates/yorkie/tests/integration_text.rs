//! Integration tests for concurrent text editing against a live server.
//!
//! JS reference: `packages/sdk/test/integration/text_test.ts`. Like the JS
//! `withTwoClientsAndDocuments` cases, each test syncs two clients manually.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::two_clients::TwoClients;
use serde_json::Value;
use std::collections::BTreeMap;
use yorkie::{JsonText, SharedDocument};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

/// Runs `edit` on the text at `k1` inside one update.
fn edit_text(doc: &SharedDocument, edit: impl FnOnce(&mut JsonText) -> yorkie::Result<()>) {
    doc.lock()
        .unwrap()
        .update(|root| edit(root.get_text_mut("k1")?))
        .unwrap();
}

fn create_text(doc: &SharedDocument, content: &str) {
    doc.lock()
        .unwrap()
        .update(|root| {
            let text = root.set_text("k1")?;
            if !content.is_empty() {
                text.edit(0, 0, content, None)?;
            }
            Ok(())
        })
        .unwrap();
}

fn bold() -> BTreeMap<String, Value> {
    BTreeMap::from([("bold".to_owned(), Value::Bool(true))])
}

/// JS "should handle concurrent edit operations": both clients insert at the
/// same position, then edit overlapping ranges.
#[tokio::test]
async fn concurrent_edits_at_the_same_position_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "text-concurrent-edit").await;

    create_text(&t.d1, "");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":[]}"#, sorted_json(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.edit(0, 0, "ABCD", None).map(|_| ()));
    assert_eq!(r#"{"k1":[{"val":"ABCD"}]}"#, sorted_json(&t.d1));
    edit_text(&t.d2, |text| text.edit(0, 0, "1234", None).map(|_| ()));
    assert_eq!(r#"{"k1":[{"val":"1234"}]}"#, sorted_json(&t.d2));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.edit(2, 3, "XX", None).map(|_| ()));
    edit_text(&t.d2, |text| text.edit(2, 3, "YY", None).map(|_| ()));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.edit(4, 5, "ZZ", None).map(|_| ()));
    edit_text(&t.d2, |text| text.edit(2, 3, "TT", None).map(|_| ()));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "should handle concurrent insertion and deletion": an insertion the
/// deletion did not see survives it.
#[tokio::test]
async fn concurrent_insertion_and_deletion_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "text-insert-delete").await;

    create_text(&t.d1, "AB");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":[{"val":"AB"}]}"#, sorted_json(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.edit(0, 2, "", None).map(|_| ()));
    assert_eq!(r#"{"k1":[]}"#, sorted_json(&t.d1));
    edit_text(&t.d2, |text| text.edit(1, 1, "C", None).map(|_| ()));
    assert_eq!(
        r#"{"k1":[{"val":"A"},{"val":"C"},{"val":"B"}]}"#,
        sorted_json(&t.d2)
    );

    t.sync_all().await;
    assert_eq!(r#"{"k1":[{"val":"C"}]}"#, sorted_json(&t.d1));
    assert_eq!(r#"{"k1":[{"val":"C"}]}"#, sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "concurrent deletion test for LWW behavior - partial overlap". JS also
/// checks which deletion's ticket each tombstone keeps, which reads CRDT
/// internals the Rust public API does not expose.
#[tokio::test]
async fn concurrent_overlapping_deletions_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "text-delete-overlap").await;

    create_text(&t.d1, "abcd");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":[{"val":"abcd"}]}"#, sorted_json(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.edit(0, 3, "", None).map(|_| ()));
    edit_text(&t.d2, |text| text.edit(1, 4, "", None).map(|_| ()));
    t.sync_all().await;

    assert_eq!(r#"{"k1":[]}"#, sorted_json(&t.d1));
    assert_eq!(r#"{"k1":[]}"#, sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "ex2. concurrent formatting and insertion": the insertion the style
/// did not see stays unstyled.
#[tokio::test]
async fn concurrent_style_and_insertion_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "text-style-insert").await;

    create_text(&t.d1, "The fox jumped.");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.set_style(0, 15, bold()));
    assert_eq!(
        r#"{"k1":[{"attrs":{"bold":true},"val":"The fox jumped."}]}"#,
        sorted_json(&t.d1)
    );
    edit_text(&t.d2, |text| text.edit(4, 4, "brown ", None).map(|_| ()));
    assert_eq!(
        r#"{"k1":[{"val":"The "},{"val":"brown "},{"val":"fox jumped."}]}"#,
        sorted_json(&t.d2)
    );

    t.sync_all().await;
    assert_eq!(
        r#"{"k1":[{"attrs":{"bold":true},"val":"The "},{"val":"brown "},{"attrs":{"bold":true},"val":"fox jumped."}]}"#,
        sorted_json(&t.d1)
    );
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "ex3. overlapping formatting(bold)": two concurrent styles over
/// overlapping ranges.
#[tokio::test]
async fn concurrent_overlapping_styles_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "text-style-overlap").await;

    create_text(&t.d1, "The fox jumped.");
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_text(&t.d1, |text| text.set_style(0, 7, bold()));
    assert_eq!(
        r#"{"k1":[{"attrs":{"bold":true},"val":"The fox"},{"val":" jumped."}]}"#,
        sorted_json(&t.d1)
    );
    edit_text(&t.d2, |text| text.set_style(4, 15, bold()));
    assert_eq!(
        r#"{"k1":[{"val":"The "},{"attrs":{"bold":true},"val":"fox jumped."}]}"#,
        sorted_json(&t.d2)
    );

    t.sync_all().await;
    assert_eq!(
        r#"{"k1":[{"attrs":{"bold":true},"val":"The "},{"attrs":{"bold":true},"val":"fox"},{"attrs":{"bold":true},"val":" jumped."}]}"#,
        sorted_json(&t.d1)
    );
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}
