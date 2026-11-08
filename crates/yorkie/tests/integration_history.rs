//! Integration tests for undo and redo with two clients against a live
//! server.
//!
//! JS references:
//! - `packages/sdk/test/integration/history_array_test.ts` (`Array Undo in
//!   Multi-Client`).
//! - `packages/sdk/test/integration/history_text_test.ts` (`Text History -
//!   multi client basic`, `reconcile cases`, `multi client edge cases`).
//! - `packages/sdk/test/integration/counter_test.ts` (`Can undo/redo for
//!   concurrent users`).
//!
//! Each case syncs two clients manually, like JS `withTwoClientsAndDocuments`.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::tree::TwoClients;
use serde_json::Value;
use std::collections::BTreeMap;
use yorkie::{JsonArray, JsonText, JsonValue, SharedDocument};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn undo(doc: &SharedDocument) {
    doc.lock().unwrap().history_mut().undo().unwrap();
}

fn redo(doc: &SharedDocument) {
    doc.lock().unwrap().history_mut().redo().unwrap();
}

fn assert_converged(t: &TwoClients, what: &str) {
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2), "{what}");
}

// --- Array (history_array_test.ts) ---

#[derive(Debug, Clone, Copy)]
enum ArrayOp {
    Add,
    Remove,
    Move,
    Set,
}

const ARRAY_OPS: [ArrayOp; 4] = [ArrayOp::Add, ArrayOp::Remove, ArrayOp::Move, ArrayOp::Set];

fn update_list(doc: &SharedDocument, edit: impl FnOnce(&mut JsonArray) -> yorkie::Result<()>) {
    doc.lock()
        .unwrap()
        .update(|root| edit(root.get_array_mut("list")?))
        .unwrap();
}

/// JS `applyOp1`.
fn apply_op1(doc: &SharedDocument, op: ArrayOp) {
    update_list(doc, |list| {
        match op {
            ArrayOp::Add => {
                if let Some(prev) = list.element_id(0).cloned() {
                    list.insert_after(&prev, "insV")?;
                }
            }
            ArrayOp::Move => {
                if list.len() >= 3 {
                    let from = list.element_id(0).cloned().unwrap();
                    let to = list.element_id(2).cloned().unwrap();
                    list.move_after(&to, &from)?;
                }
            }
            ArrayOp::Remove => {
                if !list.is_empty() {
                    list.delete(0);
                }
            }
            ArrayOp::Set => {
                if list.len() > 1 {
                    list.set_value(1, "s")?;
                }
            }
        }
        Ok(())
    });
}

/// JS `applyOp2`.
fn apply_op2(doc: &SharedDocument, op: ArrayOp) {
    update_list(doc, |list| {
        match op {
            ArrayOp::Add => {
                if let Some(prev) = list.element_id(2).cloned() {
                    list.insert_after(&prev, "insV")?;
                }
            }
            ArrayOp::Move => {
                if list.len() >= 4 {
                    let from = list.element_id(1).cloned().unwrap();
                    let to = list.element_id(3).cloned().unwrap();
                    list.move_after(&to, &from)?;
                }
            }
            ArrayOp::Remove => {
                if list.len() > 1 {
                    list.delete(1);
                }
            }
            ArrayOp::Set => {
                if list.len() > 2 {
                    list.set_value(2, "2")?;
                }
            }
        }
        Ok(())
    });
}

/// JS `should handle multi user undo: ${op1}-${op2}`, for all 16 pairs.
#[tokio::test]
async fn converges_after_both_clients_undo_array_operations() {
    let Some(addr) = rpc_addr() else { return };
    for op1 in ARRAY_OPS {
        for op2 in ARRAY_OPS {
            let case = format!("{op1:?}-{op2:?}");
            let t = TwoClients::attach(&addr, &format!("history-array-{case}")).await;
            t.d1.lock()
                .unwrap()
                .update(|root| {
                    let mut list = JsonArray::new();
                    for letter in ["a", "b", "c", "d", "e"] {
                        list.push(letter)?;
                    }
                    root.set("list", list).map(|_| ())
                })
                .unwrap();
            t.c1.sync(None).await.unwrap();
            t.c2.sync(None).await.unwrap();
            assert_converged(&t, &format!("{case}: after init"));

            apply_op1(&t.d1, op1);
            apply_op2(&t.d2, op2);
            t.sync_all().await;
            assert_converged(&t, &format!("{case}: after both ops"));

            undo(&t.d1);
            undo(&t.d2);
            t.sync_all().await;
            assert_converged(&t, &format!("{case}: after both undos"));

            t.deactivate().await;
        }
    }
}

// --- Text (history_text_test.ts) ---

#[derive(Debug, Clone, Copy)]
enum TextOp {
    Insert,
    Delete,
    Replace,
}

const TEXT_OPS: [TextOp; 3] = [TextOp::Insert, TextOp::Delete, TextOp::Replace];

fn edit_text(doc: &SharedDocument, edit: impl FnOnce(&mut JsonText) -> yorkie::Result<()>) {
    doc.lock()
        .unwrap()
        .update(|root| edit(root.get_text_mut("t")?))
        .unwrap();
}

fn edit(doc: &SharedDocument, from: usize, to: usize, content: &str) {
    edit_text(doc, |t| t.edit(from, to, content, None).map(|_| ()));
}

/// JS `applyTextOp1`.
fn apply_text_op1(doc: &SharedDocument, op: TextOp) {
    edit_text(doc, |t| {
        let len = t.len();
        match op {
            TextOp::Insert => t.edit(len, len, "X", None).map(|_| ()),
            TextOp::Delete if len >= 3 => t.edit(1, 2, "", None).map(|_| ()),
            TextOp::Delete if len > 0 => t.edit(0, 1, "", None).map(|_| ()),
            TextOp::Delete => Ok(()),
            TextOp::Replace if len >= 3 => t.edit(1, 3, "12", None).map(|_| ()),
            TextOp::Replace => t.edit(0, len.min(1), "R", None).map(|_| ()),
        }
    });
}

/// JS `applyTextOp2`.
fn apply_text_op2(doc: &SharedDocument, op: TextOp) {
    edit_text(doc, |t| {
        let len = t.len();
        match op {
            TextOp::Insert => t.edit(0, 0, "Q", None).map(|_| ()),
            TextOp::Delete if len > 0 => t.edit(len - 1, len, "", None).map(|_| ()),
            TextOp::Delete => Ok(()),
            TextOp::Replace if len > 0 => t.edit(0, 1, "Z", None).map(|_| ()),
            TextOp::Replace => t.edit(0, 0, "Z", None).map(|_| ()),
        }
    });
}

/// Attaches two clients and syncs a text at `t` holding `content`.
async fn with_text(addr: &str, name: &str, content: &str) -> TwoClients {
    let t = TwoClients::attach(addr, name).await;
    t.d1.lock()
        .unwrap()
        .update(|root| root.set_text("t")?.edit(0, 0, content, None).map(|_| ()))
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    t
}

/// JS `should converge after undo: ${op1}-${op2}` and `should converge after
/// redo: ${op1}-${op2}`, for all 9 pairs.
#[tokio::test]
async fn converges_after_both_clients_undo_and_redo_text_operations() {
    let Some(addr) = rpc_addr() else { return };
    for op1 in TEXT_OPS {
        for op2 in TEXT_OPS {
            let case = format!("{op1:?}-{op2:?}");
            let t = with_text(&addr, &format!("history-text-{case}"), "The fox jumped.").await;

            apply_text_op1(&t.d1, op1);
            apply_text_op2(&t.d2, op2);
            t.sync_all().await;
            assert_converged(&t, &format!("{case}: after ops"));

            undo(&t.d1);
            undo(&t.d2);
            t.sync_all().await;
            assert_converged(&t, &format!("{case}: after undo"));

            redo(&t.d1);
            redo(&t.d2);
            t.sync_all().await;
            assert_converged(&t, &format!("{case}: after redo"));

            t.deactivate().await;
        }
    }
}

/// Runs one reconcile case: d1 and d2 edit `0123456789` concurrently, then
/// both undo and both redo, converging after each step.
async fn reconcile_case(name: &str, d1_edit: (usize, usize, &str), d2_edit: (usize, usize, &str)) {
    let Some(addr) = rpc_addr() else { return };
    let t = with_text(&addr, &format!("history-text-{name}"), "0123456789").await;

    edit(&t.d1, d1_edit.0, d1_edit.1, d1_edit.2);
    edit(&t.d2, d2_edit.0, d2_edit.1, d2_edit.2);
    t.sync_all().await;
    assert_converged(&t, "after edits");

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after undo");

    redo(&t.d1);
    redo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after redo");

    t.deactivate().await;
}

#[tokio::test]
async fn reconciles_a_remote_edit_left_of_the_undo() {
    reconcile_case("reconcile-left", (6, 8, ""), (2, 2, "XX")).await;
}

#[tokio::test]
async fn reconciles_a_remote_edit_right_of_the_undo() {
    reconcile_case("reconcile-right", (2, 4, ""), (8, 8, "YY")).await;
}

#[tokio::test]
async fn reconciles_an_undo_contained_by_the_remote_edit() {
    reconcile_case("reconcile-contained-by", (4, 6, ""), (2, 8, "")).await;
}

#[tokio::test]
async fn reconciles_a_remote_edit_contained_by_the_undo() {
    reconcile_case("reconcile-contains", (2, 8, ""), (5, 5, "ZZ")).await;
}

#[tokio::test]
async fn reconciles_a_remote_edit_over_the_undo_start() {
    reconcile_case("reconcile-overlap-start", (4, 8, ""), (2, 6, "")).await;
}

#[tokio::test]
async fn reconciles_a_remote_edit_over_the_undo_end() {
    reconcile_case("reconcile-overlap-end", (2, 6, ""), (4, 8, "")).await;
}

#[tokio::test]
async fn reconciles_a_remote_edit_adjacent_to_the_undo() {
    reconcile_case("reconcile-adjacent", (4, 6, ""), (6, 6, "AA")).await;
}

fn content_of(doc: &SharedDocument) -> String {
    match doc.lock().unwrap().get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected text at t, got {other:?}"),
    }
}

/// Both clients undo overlapping deletions: the content should be the
/// original text.
async fn overlapping_undo_restores_original(
    name: &str,
    d1_edit: (usize, usize),
    d2_edit: (usize, usize),
) {
    let Some(addr) = rpc_addr() else { return };
    let t = with_text(&addr, &format!("history-text-{name}"), "0123456789").await;

    edit(&t.d1, d1_edit.0, d1_edit.1, "");
    edit(&t.d2, d2_edit.0, d2_edit.1, "");
    t.sync_all().await;

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;

    assert_eq!(content_of(&t.d1), content_of(&t.d2), "convergence");
    assert_eq!("0123456789", content_of(&t.d1), "content after undo");
    t.deactivate().await;
}

#[tokio::test]
#[ignore = "skipped upstream; needs identity-preserving undo restore, see the \
            Undo and Redo section of docs/current-porting-gaps.md"]
async fn restores_the_original_after_both_undo_contained_deletions() {
    overlapping_undo_restores_original("correctness-contained", (4, 6), (2, 8)).await;
}

#[tokio::test]
#[ignore = "skipped upstream; needs identity-preserving undo restore, see the \
            Undo and Redo section of docs/current-porting-gaps.md"]
async fn restores_the_original_after_both_undo_overlapping_deletions() {
    overlapping_undo_restores_original("correctness-overlap", (4, 8), (2, 6)).await;
}

#[tokio::test]
async fn converges_after_undo_of_inserts_at_the_same_position() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_text(&addr, "history-text-same-position", "ABCD").await;

    edit(&t.d1, 2, 2, "X");
    edit(&t.d2, 2, 2, "Y");
    t.sync_all().await;
    assert_converged(&t, "after edits");

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after undo");
    t.deactivate().await;
}

#[tokio::test]
async fn converges_after_undo_of_full_deletion_and_insertion() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_text(&addr, "history-text-full-deletion", "ABCD").await;

    edit(&t.d1, 0, 4, "");
    edit(&t.d2, 0, 0, "XY");
    t.sync_all().await;
    assert_converged(&t, "after edits");

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after undo");
    t.deactivate().await;
}

#[tokio::test]
async fn converges_when_one_client_undoes_and_the_other_redoes() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_text(&addr, "history-text-undo-redo", "ABCDEFGH").await;

    edit(&t.d1, 2, 4, "XX");
    edit(&t.d2, 6, 8, "YY");
    t.sync_all().await;

    undo(&t.d1);
    t.sync_all().await;

    redo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after redo and undo");
    t.deactivate().await;
}

#[tokio::test]
async fn converges_after_undo_and_redo_of_concurrent_styles() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_text(&addr, "history-text-styles", "The fox jumped.").await;

    let style = |key: &str| {
        let mut attributes = BTreeMap::new();
        attributes.insert(key.to_owned(), Value::Bool(true));
        attributes
    };
    edit_text(&t.d1, |text| text.set_style(0, 15, style("bold")));
    edit_text(&t.d2, |text| text.set_style(4, 15, style("italic")));
    t.sync_all().await;

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after undo");

    redo(&t.d1);
    redo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after redo");
    t.deactivate().await;
}

// --- Counter (counter_test.ts) ---

#[tokio::test]
async fn undoes_an_increase_while_another_client_increases() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "history-counter").await;
    t.d1.lock()
        .unwrap()
        .update(|root| root.set_counter("counter", 100).map(|_| ()))
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":100}"#, sorted_json(&t.d2));

    let increase = |doc: &SharedDocument, by: i32| {
        doc.lock()
            .unwrap()
            .update(|root| root.get_counter_mut("counter")?.increase(by).map(|_| ()))
            .unwrap();
    };
    increase(&t.d1, 1);
    increase(&t.d2, 2);
    t.sync_all().await;
    assert_eq!(r#"{"counter":103}"#, sorted_json(&t.d1));
    assert_eq!(r#"{"counter":103}"#, sorted_json(&t.d2));

    undo(&t.d1);
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":102}"#, sorted_json(&t.d1));
    assert_eq!(r#"{"counter":102}"#, sorted_json(&t.d2));

    // Only the client that undid can redo.
    assert!(t.d1.lock().unwrap().history().can_redo());
    assert!(!t.d2.lock().unwrap().history().can_redo());

    redo(&t.d1);
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":103}"#, sorted_json(&t.d1));
    assert_eq!(r#"{"counter":103}"#, sorted_json(&t.d2));
    t.deactivate().await;
}
