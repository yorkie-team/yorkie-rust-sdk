//! Integration tests for Tree undo and redo with two clients against a live
//! server.
//!
//! JS references, all in `packages/sdk/test/integration/` (`8629993e`):
//! - `history_tree_test.ts`: "multi client basic", "reconcile cases", "multi
//!   client edge cases", "multi client style undo convergence", "multi
//!   client style vs edit/split convergence", "undo past initial tree via
//!   initialRoot" and "GC symmetry and anchor fallback";
//! - `history_tree_concurrent_test.ts`;
//! - `history_tree_opinfo_test.ts`.
//!
//! The single-client cases of `history_tree_test.ts` need no server and are
//! in `yorkie-core` (`document/tree_undo_tests.rs`); the multi-client split
//! cases are in `integration_history_tree_split.rs`.
//!
//! Each case syncs two clients manually, like JS `withTwoClientsAndDocuments`.
//! A JS loop becomes one test that runs every row and names the row on
//! failure. Run with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::history::{assert_converged, redo, sorted_json, undo};
use integration_support::tree::{
    attributes, doc_of, edit_tree, element, element_with, p, set_tree, text, xml,
};
use integration_support::two_clients::{active_client, shared, TwoClients};
use integration_support::{max_vector_of, rpc_addr, unique_key};
use serde_json::json;
use std::sync::{Arc, Mutex};
use yorkie::{
    AttachOptions, DeactivateOptions, DocEvent, Document, JsonObject, JsonTree, JsonValue,
    OpSource, OperationInfo, SharedDocument, TreeNodeInput,
};

fn assert_xml_converged(t: &TwoClients, what: &str) {
    assert_eq!(xml(&t.d1), xml(&t.d2), "{what}");
}

/// A document key for `case`: keys are slugs, so every other character
/// becomes a dash.
fn slug(case: &str) -> String {
    case.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn actors(t: &TwoClients) -> yorkie::VersionVector {
    let a1 = t.c1.actor_id().unwrap();
    let a2 = t.c2.actor_id().unwrap();
    max_vector_of(&[&a1, &a2])
}

// --- history_tree_test.ts ---

/// JS `TreeOp`, the rows of "multi client basic".
#[derive(Debug, Clone, Copy)]
enum TreeOp {
    InsertText,
    DeleteText,
    InsertElement,
}

const MULTI_OPS: [TreeOp; 3] = [
    TreeOp::InsertText,
    TreeOp::DeleteText,
    TreeOp::InsertElement,
];

/// JS `initTree`: `<doc><p>The fox jumped.</p></doc>`.
const FOX: &str = "<doc><p>The fox jumped.</p></doc>";

fn fox() -> TreeNodeInput {
    doc_of(vec![p("The fox jumped.")])
}

/// JS `applyTreeOp1` (client 1 edits the middle and the end).
fn apply_tree_op1(doc: &SharedDocument, op: TreeOp) {
    edit_tree(doc, |t| match op {
        TreeOp::InsertText => t.edit(16, 16, vec![text("X")], 0),
        TreeOp::DeleteText => t.edit(6, 7, vec![], 0),
        TreeOp::InsertElement => t.edit(17, 17, vec![p("New")], 0),
    });
}

/// JS `applyTreeOp2` (client 2 edits the start).
fn apply_tree_op2(doc: &SharedDocument, op: TreeOp) {
    edit_tree(doc, |t| match op {
        TreeOp::InsertText => t.edit(1, 1, vec![text("Q")], 0),
        TreeOp::DeleteText => t.edit(15, 16, vec![], 0),
        TreeOp::InsertElement => t.edit(0, 0, vec![p("Front")], 0),
    });
}

/// JS "Tree History - multi client basic", `should converge after undo:
/// ${op1}-${op2}` and `should converge after redo: ${op1}-${op2}`, for all 9
/// pairs. The redo case repeats the undo case's steps, so one run asserts
/// both.
#[tokio::test]
async fn converges_after_both_clients_undo_and_redo_tree_operations() {
    let Some(addr) = rpc_addr() else { return };
    for op1 in MULTI_OPS {
        for op2 in MULTI_OPS {
            let case = format!("{op1:?}-{op2:?}");
            let t = TwoClients::with_tree(&addr, &format!("history-tree-{case}"), fox(), FOX).await;

            apply_tree_op1(&t.d1, op1);
            apply_tree_op2(&t.d2, op2);
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

/// A remote edit in JS "Tree History - reconcile cases": a deletion of
/// `from..to`, or an insertion of `content` at `from`.
fn edit_range(doc: &SharedDocument, (from, to, content): (usize, usize, &str)) {
    edit_tree(doc, |t| {
        let contents = if content.is_empty() {
            vec![]
        } else {
            vec![text(content)]
        };
        t.edit(from, to, contents, 0)
    });
}

/// One case of JS "Tree History - reconcile cases": on
/// `<doc><p>0123456789</p></doc>`, d1 and d2 edit concurrently, then both
/// undo and both redo, converging after each step.
async fn reconcile_case(name: &str, d1_edit: (usize, usize, &str), d2_edit: (usize, usize, &str)) {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        &format!("history-tree-{name}"),
        doc_of(vec![p("0123456789")]),
        "<doc><p>0123456789</p></doc>",
    )
    .await;

    edit_range(&t.d1, d1_edit);
    edit_range(&t.d2, d2_edit);
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

/// JS "Case 1 (left): remote edit LEFT of undo should shift position".
#[tokio::test]
async fn reconciles_a_remote_tree_edit_left_of_the_undo() {
    reconcile_case("reconcile-left", (7, 9, ""), (3, 3, "XX")).await;
}

/// JS "Case 2 (right): remote edit RIGHT of undo should not affect".
#[tokio::test]
async fn reconciles_a_remote_tree_edit_right_of_the_undo() {
    reconcile_case("reconcile-right", (3, 5, ""), (9, 9, "YY")).await;
}

/// JS "Case 3 (contained_by): undo range contained by remote should
/// collapse".
#[tokio::test]
async fn reconciles_a_tree_undo_contained_by_the_remote_edit() {
    reconcile_case("reconcile-contained-by", (5, 7, ""), (3, 9, "")).await;
}

/// JS "Case 4 (contains): remote range contained by undo should adjust".
#[tokio::test]
async fn reconciles_a_remote_tree_edit_contained_by_the_undo() {
    reconcile_case("reconcile-contains", (3, 9, ""), (6, 6, "ZZ")).await;
}

/// JS "Case 5 (overlap_start): remote overlaps start of undo range".
#[tokio::test]
async fn reconciles_a_remote_tree_edit_over_the_undo_start() {
    reconcile_case("reconcile-overlap-start", (5, 9, ""), (3, 7, "")).await;
}

/// JS "Case 6 (overlap_end): remote overlaps end of undo range".
#[tokio::test]
async fn reconciles_a_remote_tree_edit_over_the_undo_end() {
    reconcile_case("reconcile-overlap-end", (3, 7, ""), (5, 9, "")).await;
}

/// JS "Case 7 (adjacent): adjacent edits at boundary".
#[tokio::test]
async fn reconciles_a_remote_tree_edit_adjacent_to_the_undo() {
    reconcile_case("reconcile-adjacent", (5, 7, ""), (7, 7, "AA")).await;
}

/// JS "Tree History - multi client edge cases", "should converge with
/// concurrent element + text edits".
#[tokio::test]
async fn converges_after_undo_of_concurrent_element_and_text_edits() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-element-text",
        doc_of(vec![p("ABCD")]),
        "<doc><p>ABCD</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit(5, 5, vec![p("New")], 0));
    edit_tree(&t.d2, |t| t.edit(1, 1, vec![text("X")], 0));
    t.sync_all().await;
    assert_converged(&t, "after edits");

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after undo");
    t.deactivate().await;
}

/// JS "should converge with concurrent text edits in same paragraph": d1
/// undoes and then redoes while d2 only undoes.
#[tokio::test]
async fn converges_when_one_client_redoes_and_the_other_undoes_tree_edits() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-same-paragraph",
        doc_of(vec![p("ABCDEFGH")]),
        "<doc><p>ABCDEFGH</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit(3, 5, vec![text("XX")], 0));
    edit_tree(&t.d2, |t| t.edit(7, 9, vec![text("YY")], 0));
    t.sync_all().await;
    assert_converged(&t, "after edits");

    undo(&t.d1);
    t.sync_all().await;

    redo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after redo and undo");
    t.deactivate().await;
}

/// JS "should converge with nested structure concurrent edits".
#[tokio::test]
async fn converges_after_undo_and_redo_of_edits_in_two_paragraphs() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-nested",
        doc_of(vec![p("AB"), p("CD")]),
        "<doc><p>AB</p><p>CD</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit(1, 1, vec![text("X")], 0));
    edit_tree(&t.d2, |t| t.edit(5, 5, vec![text("Y")], 0));
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

/// A style op of JS "Tree History - multi client style undo convergence" and
/// "multi client style vs edit/split convergence".
#[derive(Debug, Clone, Copy)]
enum StyleOp {
    SetBold,
    SetItalic,
    SetColor,
    RemoveBold,
}

/// Applies `op` over `from..to`.
fn apply_style(doc: &SharedDocument, op: StyleOp, from: usize, to: usize) {
    edit_tree(doc, |t| match op {
        StyleOp::SetBold => t.style(from, to, attributes(json!({"bold": "true"}))),
        StyleOp::SetItalic => t.style(from, to, attributes(json!({"italic": "true"}))),
        StyleOp::SetColor => t.style(from, to, attributes(json!({"color": "red"}))),
        StyleOp::RemoveBold => t.remove_style(from, to, ["bold"]),
    });
}

const LOCAL_STYLE_OPS: [StyleOp; 3] = [StyleOp::SetBold, StyleOp::SetItalic, StyleOp::RemoveBold];

/// JS "Tree History - multi client style undo convergence", `should
/// converge: local ${localOp} + remote ${remoteOp} on ${target}`, for all 18
/// rows: d1 styles the first `<p>`, d2 styles the same or the second one,
/// then d1 undoes.
#[tokio::test]
async fn converges_after_undo_of_a_style_against_a_concurrent_style() {
    let Some(addr) = rpc_addr() else { return };
    let remote_ops = [StyleOp::SetColor, StyleOp::SetBold, StyleOp::RemoveBold];
    for local in LOCAL_STYLE_OPS {
        for remote in remote_ops {
            for (target, range) in [("same-element", (0, 1)), ("different-element", (3, 4))] {
                let case = format!("{local:?}+{remote:?}-on-{target}");
                let t = TwoClients::with_tree(
                    &addr,
                    &slug(&format!("history-tree-style-{case}")),
                    doc_of(vec![
                        element_with("p", json!({"bold": "true"}), vec![text("AB")]),
                        p("CD"),
                    ]),
                    r#"<doc><p bold="true">AB</p><p>CD</p></doc>"#,
                )
                .await;

                apply_style(&t.d1, local, 0, 1);
                apply_style(&t.d2, remote, range.0, range.1);
                t.sync_all().await;

                undo(&t.d1);
                t.sync_all().await;
                assert_xml_converged(&t, &format!("divergence: {case}"));

                t.deactivate().await;
            }
        }
    }
}

/// A remote edit of JS "multi client style vs edit/split convergence".
#[derive(Debug, Clone, Copy)]
enum EditOp {
    InsertText,
    DeleteText,
    InsertElement,
    SplitL1,
}

fn apply_edit(doc: &SharedDocument, op: EditOp) {
    edit_tree(doc, |t| match op {
        EditOp::InsertText => t.edit(1, 1, vec![text("X")], 0),
        EditOp::DeleteText => t.edit(1, 2, vec![], 0),
        EditOp::InsertElement => t.edit(6, 6, vec![p("NEW")], 0),
        EditOp::SplitL1 => t.edit(3, 3, vec![], 1),
    });
}

/// JS "Tree History - multi client style vs edit/split convergence", for
/// all 12 pairs in both directions: `should converge: local style(...) +
/// remote edit(...)` (d1 undoes its style) and `should converge: local
/// edit(...) + remote style(...)` (d1 undoes its edit).
#[tokio::test]
async fn converges_after_undo_of_a_style_or_edit_against_the_other() {
    let Some(addr) = rpc_addr() else { return };
    let edit_ops = [
        EditOp::InsertText,
        EditOp::DeleteText,
        EditOp::InsertElement,
        EditOp::SplitL1,
    ];
    for style in LOCAL_STYLE_OPS {
        for edit in edit_ops {
            for style_is_local in [true, false] {
                let case = if style_is_local {
                    format!("style({style:?})+remote-edit({edit:?})")
                } else {
                    format!("edit({edit:?})+remote-style({style:?})")
                };
                let t = TwoClients::with_tree(
                    &addr,
                    &slug(&format!("history-tree-style-edit-{case}")),
                    doc_of(vec![
                        element_with("p", json!({"bold": "true"}), vec![text("ABCD")]),
                        p("EFGH"),
                    ]),
                    r#"<doc><p bold="true">ABCD</p><p>EFGH</p></doc>"#,
                )
                .await;

                if style_is_local {
                    apply_style(&t.d1, style, 0, 1);
                    apply_edit(&t.d2, edit);
                } else {
                    apply_edit(&t.d1, edit);
                    apply_style(&t.d2, style, 0, 1);
                }
                t.sync_all().await;

                undo(&t.d1);
                t.sync_all().await;
                assert_xml_converged(&t, &format!("divergence: {case}"));

                t.deactivate().await;
            }
        }
    }
}

/// JS "Tree History - undo past initial tree via initialRoot", "should not
/// allow undoing past initialRoot after attach": typing four characters
/// into a Tree from `initialRoot` and undoing them leaves the tree, and a
/// fifth undo is not offered.
#[tokio::test]
async fn does_not_undo_past_a_tree_from_the_initial_root() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let mut initial_root = JsonObject::new();
    initial_root
        .set(
            "content",
            JsonValue::Tree(JsonTree::new(doc_of(vec![element_with(
                "block",
                json!({"id": "block-1"}),
                vec![element("inline", vec![])],
            )]))),
        )
        .unwrap();
    let doc = client
        .attach(
            shared(&unique_key("history-tree-initial-root")),
            AttachOptions {
                initial_root: Some(initial_root),
                ..AttachOptions::default()
            },
        )
        .await
        .unwrap();
    let content_xml = || match doc.lock().unwrap().get_root().get("content") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().unwrap(),
        other => panic!("expected a tree, got {other:?}"),
    };
    let initial_xml = content_xml();
    let can_undo = || doc.lock().unwrap().history().can_undo();
    assert!(!can_undo(), "undo stack should be empty after attach");

    for ch in ["a", "s", "d", "f"] {
        doc.lock()
            .unwrap()
            .update(|root| {
                root.get_tree_mut("content")?.edit_by_path(
                    &[0, 0, 0],
                    &[0, 0, 0],
                    vec![text(ch)],
                    0,
                )
            })
            .unwrap();
    }

    for _ in 0..4 {
        assert!(can_undo());
        undo(&doc);
    }
    assert_eq!(initial_xml, content_xml());

    assert!(!can_undo(), "should not be able to undo past initialRoot");
    assert_eq!(initial_xml, content_xml(), "tree should remain intact");

    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "Tree History - GC symmetry and anchor fallback", "converges and
/// purges symmetrically after undo/redo of concurrent deletes": after both
/// undo and redo overlapping deletions, a full-vector collection purges the
/// same count on both replicas and drains both.
#[tokio::test]
async fn purges_symmetrically_after_undo_and_redo_of_concurrent_deletes() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-gc-symmetry",
        doc_of(vec![p("0123456789")]),
        "<doc><p>0123456789</p></doc>",
    )
    .await;

    // d2's deleted range sits strictly inside d1's (contained_by).
    edit_tree(&t.d1, |t| t.edit(2, 8, vec![], 0));
    edit_tree(&t.d2, |t| t.edit(4, 6, vec![], 0));
    t.sync_all().await;
    assert_converged(&t, "after ops");

    undo(&t.d1);
    undo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after undo");

    redo(&t.d1);
    redo(&t.d2);
    t.sync_all().await;
    assert_converged(&t, "after redo");

    let vector = actors(&t);
    let purged1 = t.d1.lock().unwrap().garbage_collect(&vector);
    let purged2 = t.d2.lock().unwrap().garbage_collect(&vector);
    assert_eq!(purged1, purged2, "both replicas purge the same node count");
    assert_eq!(0, t.d1.lock().unwrap().garbage_len(), "d1 garbage drained");
    assert_eq!(0, t.d2.lock().unwrap().garbage_len(), "d2 garbage drained");
    assert_converged(&t, "converged after GC");
    t.deactivate().await;
}

/// JS "recreates a purged subtree on redo and converges": an undo after the
/// deleted `<p>` was purged rebuilds it from the spans its reverse carries.
#[tokio::test]
async fn recreates_a_purged_subtree_and_converges() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-recreate",
        doc_of(vec![p("hello")]),
        "<doc><p>hello</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit(0, 7, vec![], 0));
    t.sync_all().await;
    assert_converged(&t, "after delete");

    undo(&t.d1);
    t.sync_all().await;
    let vector = actors(&t);
    t.d1.lock().unwrap().garbage_collect(&vector);
    t.d2.lock().unwrap().garbage_collect(&vector);
    assert_converged(&t, "after undo + GC");

    redo(&t.d1);
    t.sync_all().await;
    t.d1.lock().unwrap().garbage_collect(&vector);
    t.d2.lock().unwrap().garbage_collect(&vector);

    undo(&t.d1);
    t.sync_all().await;
    assert_converged(&t, "converged after recreate");
    assert_eq!(
        "<doc><p>hello</p></doc>",
        xml(&t.d1),
        "purged subtree recreated under original identity"
    );
    t.deactivate().await;
}

// --- history_tree_concurrent_test.ts ---

/// JS `conv`: converged, with both trees in the message.
fn conv(t: &TwoClients, label: &str) {
    assert_eq!(
        sorted_json(&t.d1),
        sorted_json(&t.d2),
        "{label} DIVERGED\n  d1={}\n  d2={}",
        xml(&t.d1),
        xml(&t.d2)
    );
}

/// JS `initFlat`: `<doc><p>0123456789</p></doc>`.
async fn flat(addr: &str, name: &str) -> TwoClients {
    TwoClients::with_tree(
        addr,
        &slug(&format!("history-tree-concurrent-{name}")),
        doc_of(vec![p("0123456789")]),
        "<doc><p>0123456789</p></doc>",
    )
    .await
}

/// A deleted range `from..to`.
type Range = (usize, usize);

/// The undo ranges of d1 and d2 in JS "Tree History - concurrent overlapping
/// undo after GC", covering every overlap relationship.
const OVERLAPS: [(&str, Range, Range); 6] = [
    ("contained_by", (5, 7), (3, 9)),
    ("contains", (3, 9), (5, 7)),
    ("overlap_start", (5, 9), (3, 7)),
    ("overlap_end", (3, 7), (5, 9)),
    ("identical", (3, 7), (3, 7)),
    ("adjacent", (3, 5), (5, 7)),
];

/// JS `converges on undo of overlapping deletes: ${name}` and `converges on
/// undo+redo of overlapping deletes: ${name}`, for all 6 overlaps: once GC
/// has purged both deletions, both undos restore the initial content and
/// both redos the post-delete content. The undo case repeats the first
/// steps of the redo case, so one run asserts both.
#[tokio::test]
async fn converges_on_undo_and_redo_of_overlapping_deletes_after_gc() {
    let Some(addr) = rpc_addr() else { return };
    for (name, r1, r2) in OVERLAPS {
        let t = flat(&addr, name).await;
        let initial = xml(&t.d1);

        edit_tree(&t.d1, |t| t.edit(r1.0, r1.1, vec![], 0));
        edit_tree(&t.d2, |t| t.edit(r2.0, r2.1, vec![], 0));
        t.settle().await;
        t.settle().await; // let GC purge the tombstones
        conv(&t, &format!("{name}: after deletes"));
        let after_deletes = xml(&t.d1);

        undo(&t.d1);
        undo(&t.d2);
        t.settle().await;
        conv(&t, &format!("{name}: after undo"));
        assert_eq!(
            initial,
            xml(&t.d1),
            "{name}: undo restores the initial visible content"
        );

        redo(&t.d1);
        redo(&t.d2);
        t.settle().await;
        conv(&t, &format!("{name}: after redo"));
        assert_eq!(
            after_deletes,
            xml(&t.d1),
            "{name}: redo restores the post-delete visible content"
        );

        t.deactivate().await;
    }
}

/// JS "KNOWN: delete a whole <p> vs edit text inside it, both undo"
/// (`it.skip` there).
#[tokio::test]
#[ignore = "known limitation shared with JS and Go: after GC, the internal text-node \
            segmentation diverges when a whole element is deleted concurrently with \
            an edit inside it and both undo; a fix needs undo-stack-aware GC"]
async fn converges_after_undo_of_a_paragraph_delete_vs_an_edit_inside_it() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-concurrent-known-1",
        doc_of(vec![p("hello"), p("world")]),
        "<doc><p>hello</p><p>world</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit(0, 7, vec![], 0));
    edit_tree(&t.d2, |t| t.edit(3, 5, vec![text("XY")], 0));
    t.settle().await;
    conv(&t, "after ops");

    undo(&t.d1);
    undo(&t.d2);
    t.settle().await;
    conv(&t, "after undo");
    t.deactivate().await;
}

/// JS "KNOWN: delete two <p> vs edit inside first, both undo
/// (segmentation)" (`it.skip` there).
#[tokio::test]
#[ignore = "known limitation shared with JS and Go: after GC, the internal text-node \
            segmentation diverges when two elements are deleted concurrently with an \
            edit inside one and both undo; a fix needs undo-stack-aware GC"]
async fn converges_after_undo_of_a_two_paragraph_delete_vs_an_edit_inside() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-concurrent-known-2",
        doc_of(vec![p("aaaa"), p("bbbb"), p("cccc")]),
        "<doc><p>aaaa</p><p>bbbb</p><p>cccc</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit(0, 12, vec![], 0));
    edit_tree(&t.d2, |t| t.edit(2, 4, vec![text("XY")], 0));
    t.settle().await;
    t.settle().await;

    undo(&t.d1);
    undo(&t.d2);
    t.settle().await;
    conv(&t, "after undo");
    t.deactivate().await;
}

/// JS "undo one of two concurrent splits of the same node" (yorkie#1999):
/// `abcde` split at 1 and at 4 settles as `a|bcd|e`, and undoing the split
/// at 4 gives `a|bcde`.
#[tokio::test]
async fn undoes_one_of_two_concurrent_splits_of_the_same_node() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-concurrent-two-splits",
        doc_of(vec![element(
            "p",
            vec![element("span", vec![text("abcde")])],
        )]),
        "<doc><p><span>abcde</span></p></doc>",
    )
    .await;

    edit_tree(&t.d1, |t| t.edit_by_path(&[0, 0, 1], &[0, 0, 1], vec![], 1));
    edit_tree(&t.d2, |t| t.edit_by_path(&[0, 0, 4], &[0, 0, 4], vec![], 1));
    t.settle().await;
    assert_eq!(
        "<doc><p><span>a</span><span>bcd</span><span>e</span></p></doc>",
        xml(&t.d1)
    );

    undo(&t.d2);
    t.settle().await;
    assert_eq!(
        "<doc><p><span>a</span><span>bcde</span></p></doc>",
        xml(&t.d1)
    );
    assert_converged(&t, "after undo");
    t.deactivate().await;
}

// --- history_tree_opinfo_test.ts ---

/// What JS `follow` keeps: a detached mirror of a document's tree, built
/// only from the tree-edit op infos the document publishes, and the op infos
/// of its undos and redos.
struct Follower {
    mirror: Arc<Mutex<Document>>,
    undo_redo_infos: Arc<Mutex<Vec<OperationInfo>>>,
}

impl Follower {
    fn xml(&self) -> String {
        match self.mirror.lock().unwrap().get_root().get("t") {
            Some(JsonValue::Tree(tree)) => tree.to_xml().unwrap(),
            other => panic!("expected a tree, got {other:?}"),
        }
    }

    /// The `(from_path, to_path)` of each undo or redo op info.
    fn undo_redo_paths(&self) -> Vec<(Vec<usize>, Vec<usize>)> {
        self.undo_redo_infos
            .lock()
            .unwrap()
            .iter()
            .map(|info| match info {
                OperationInfo::TreeEdit {
                    from_path, to_path, ..
                } => (from_path.clone(), to_path.clone()),
                other => panic!("expected a tree edit, got {other:?}"),
            })
            .collect()
    }
}

/// JS `follow`: mirrors `doc` into a detached document by applying only the
/// tree-edit op infos `doc` publishes, the way an editor binding would. If
/// the op infos describe the change, the mirror stays equal to `doc`.
fn follow(doc: &SharedDocument) -> Follower {
    let root = match doc.lock().unwrap().get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.root_tree_node().unwrap(),
        other => panic!("expected a tree, got {other:?}"),
    };
    let mut mirror = Document::new("mirror");
    mirror
        .update(|r| r.set_tree("t", root).map(|_| ()))
        .unwrap();
    let mirror = Arc::new(Mutex::new(mirror));
    let undo_redo_infos = Arc::new(Mutex::new(Vec::new()));

    let (sink, infos) = (Arc::clone(&mirror), Arc::clone(&undo_redo_infos));
    doc.lock().unwrap().subscribe(move |event| {
        let info = match event {
            DocEvent::LocalChange(info) | DocEvent::RemoteChange(info) => info,
            _ => return,
        };
        for op in &info.operations {
            let OperationInfo::TreeEdit {
                from_path,
                to_path,
                value,
                ..
            } = op
            else {
                continue;
            };
            if info.source == OpSource::UndoRedo {
                infos.lock().unwrap().push(op.clone());
            }
            let contents = value.clone().unwrap_or_default();
            sink.lock()
                .unwrap()
                .update(|r| {
                    r.get_tree_mut("t")?
                        .edit_by_path(from_path, to_path, contents, 0)
                })
                .unwrap();
        }
    });
    Follower {
        mirror,
        undo_redo_infos,
    }
}

/// JS "Tree History - undo/redo OpInfo positions", `reports where undo and
/// redo of "${name}" landed`, for all 5 rows: the op infos of d1's edit, its
/// undo and its redo keep a mirror of each replica equal to it.
#[tokio::test]
async fn reports_where_undo_and_redo_of_a_tree_edit_landed() {
    let Some(addr) = rpc_addr() else { return };
    type Edit = fn(&mut JsonTree) -> yorkie::Result<()>;
    let cases: [(&str, Vec<TreeNodeInput>, Edit); 5] = [
        ("insert text", vec![p("abcd")], |t| {
            t.edit_by_path(&[0, 2], &[0, 2], vec![text("X")], 0)
        }),
        ("delete text", vec![p("abcd")], |t| {
            t.edit_by_path(&[0, 1], &[0, 3], vec![], 0)
        }),
        ("insert element", vec![p("ab")], |t| {
            t.edit_by_path(&[1], &[1], vec![p("cd")], 0)
        }),
        ("delete element", vec![p("ab"), p("cd")], |t| {
            t.edit_by_path(&[0], &[1], vec![], 0)
        }),
        (
            "delete two elements",
            vec![p("ab"), p("cd"), p("ef")],
            |t| t.edit_by_path(&[0], &[2], vec![], 0),
        ),
    ];
    for (name, initial, edit) in cases {
        let t = TwoClients::attach(&addr, &slug(&format!("history-tree-opinfo-{name}"))).await;
        set_tree(&t.d1, doc_of(initial));
        t.c1.sync(None).await.unwrap();
        t.c2.sync(None).await.unwrap();
        let m1 = follow(&t.d1);
        let m2 = follow(&t.d2);
        let check = |label: &str| {
            assert_eq!(m1.xml(), xml(&t.d1), "{name}: d1 {label}");
            assert_eq!(m2.xml(), xml(&t.d2), "{name}: d2 {label}");
        };

        edit_tree(&t.d1, edit);
        t.sync_all().await;
        check("after edit");

        undo(&t.d1);
        t.sync_all().await;
        check("after undo");

        redo(&t.d1);
        t.sync_all().await;
        check("after redo");

        for (from_path, _) in m1.undo_redo_paths() {
            assert!(!from_path.is_empty(), "{name}: undo/redo reports a path");
        }
        t.deactivate().await;
    }
}

/// JS "reports shifted positions when a peer edited before the undone text".
#[tokio::test]
async fn reports_shifted_positions_after_a_peer_edit_before_the_undone_text() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-opinfo-shifted",
        doc_of(vec![p("ab")]),
        "<doc><p>ab</p></doc>",
    )
    .await;
    let m1 = follow(&t.d1);
    let m2 = follow(&t.d2);

    edit_tree(&t.d1, |t| {
        t.edit_by_path(&[0, 2], &[0, 2], vec![text("cd")], 0)
    });
    t.sync_all().await;
    edit_tree(&t.d2, |t| {
        t.edit_by_path(&[0, 0], &[0, 0], vec![text("XYZ")], 0)
    });
    t.sync_all().await;

    undo(&t.d1);
    t.sync_all().await;
    assert_eq!("<doc><p>XYZab</p></doc>", xml(&t.d1));
    assert_eq!(m1.xml(), xml(&t.d1), "d1 after undo");
    assert_eq!(m2.xml(), xml(&t.d2), "d2 after undo");
    assert_eq!(vec![(vec![0, 5], vec![0, 7])], m1.undo_redo_paths());
    t.deactivate().await;
}

/// JS "reports nothing when the undone nodes stay under a removed ancestor":
/// the undo publishes no op info but still reaches the peer, and undoing
/// the peer's removal converges with or without the text, as GC decides.
#[tokio::test]
async fn reports_nothing_for_an_undo_under_a_removed_ancestor() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-opinfo-removed-ancestor",
        doc_of(vec![p("ab"), p("cd")]),
        "<doc><p>ab</p><p>cd</p></doc>",
    )
    .await;
    let m1 = follow(&t.d1);
    let m2 = follow(&t.d2);

    // d1 deletes the text in the first paragraph, then d2 removes that
    // paragraph, so d1's undo brings the text back under a removed ancestor.
    edit_tree(&t.d1, |t| t.edit_by_path(&[0, 0], &[0, 2], vec![], 0));
    t.sync_all().await;
    edit_tree(&t.d2, |t| t.edit_by_path(&[0], &[1], vec![], 0));
    t.sync_all().await;
    assert_eq!("<doc><p>cd</p></doc>", xml(&t.d1));

    let d1_actor = t.d1.lock().unwrap().actor_id().clone();
    let seen_by_d2 = || {
        t.d2.lock()
            .unwrap()
            .version_vector()
            .get(d1_actor.as_str())
            .unwrap_or(0)
    };
    let seen_before = seen_by_d2();
    undo(&t.d1);
    t.sync_all().await;

    assert!(
        m1.undo_redo_infos.lock().unwrap().is_empty(),
        "undo reports no position"
    );
    assert_eq!("<doc><p>cd</p></doc>", xml(&t.d1));
    assert_eq!(m1.xml(), xml(&t.d1), "d1 after undo");
    assert_eq!(m2.xml(), xml(&t.d2), "d2 after undo");
    assert!(seen_by_d2() > seen_before, "the undo reached d2");

    undo(&t.d2);
    t.sync_all().await;
    let restored = xml(&t.d2);
    assert!(
        restored == "<doc><p>ab</p><p>cd</p></doc>" || restored == "<doc><p></p><p>cd</p></doc>",
        "unexpected tree after d2's undo: {restored}"
    );
    assert_eq!(xml(&t.d1), restored);
    t.deactivate().await;
}

/// JS "reports a restored subtree parent first, then its children".
#[tokio::test]
async fn reports_a_restored_subtree_parent_first_then_its_children() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "history-tree-opinfo-parent-first",
        doc_of(vec![p("ab"), p("cd")]),
        "<doc><p>ab</p><p>cd</p></doc>",
    )
    .await;
    let m1 = follow(&t.d1);
    let m2 = follow(&t.d2);

    edit_tree(&t.d1, |t| t.edit_by_path(&[1], &[2], vec![], 0));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    undo(&t.d1);
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    assert_eq!("<doc><p>ab</p><p>cd</p></doc>", xml(&t.d2));
    assert_eq!(m1.xml(), xml(&t.d1));
    assert_eq!(m2.xml(), xml(&t.d2));
    let reported = m1
        .undo_redo_infos
        .lock()
        .unwrap()
        .iter()
        .map(|info| match info {
            OperationInfo::TreeEdit {
                from_path, value, ..
            } => (from_path.clone(), value.clone()),
            other => panic!("expected a tree edit, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vec![
            (vec![1], Some(vec![element("p", vec![])])),
            (vec![1, 0], Some(vec![text("cd")])),
        ],
        reported
    );
    t.deactivate().await;
}
