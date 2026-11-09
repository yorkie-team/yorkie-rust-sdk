//! Integration tests for undo and redo of Tree splits with two clients
//! against a live server.
//!
//! JS reference: `packages/sdk/test/integration/history_tree_split_test.ts`
//! (`8629993e`), "multi client split L1 convergence", "split L1 edge cases"
//! ("should handle undo after concurrent parent deletion (L1)"), "multi
//! client split L2 convergence" and "multi client split L2 edge cases". Its
//! single-client cases need no server and are in `yorkie-core`
//! (`document/tree_undo_tests.rs`), with the chained `split-l2 → split-l2`
//! case ignored there as in JS and Go.
//!
//! Each case syncs two clients manually, like JS `withTwoClientsAndDocuments`.
//! A JS loop becomes one test that runs every row and names the row on
//! failure. Run with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::history::{redo, undo};
use integration_support::rpc_addr;
use integration_support::tree::{doc_of, edit_tree, element, p, text, xml};
use integration_support::two_clients::TwoClients;
use yorkie::{SharedDocument, TreeNodeInput};

fn assert_xml_converged(t: &TwoClients, what: &str) {
    assert_eq!(xml(&t.d1), xml(&t.d2), "{what}");
}

/// The remote ops of JS "multi client split L1/L2 convergence".
#[derive(Debug, Clone, Copy)]
enum RemoteOp {
    InsertText,
    DeleteText,
    InsertElement,
}

/// Where JS puts the remote op, relative to d1's split.
#[derive(Debug, Clone, Copy)]
enum RemotePos {
    BeforeSplit,
    AfterSplit,
    DifferentElement,
}

const REMOTE_OPS: [RemoteOp; 3] = [
    RemoteOp::InsertText,
    RemoteOp::DeleteText,
    RemoteOp::InsertElement,
];

const REMOTE_POSITIONS: [RemotePos; 3] = [
    RemotePos::BeforeSplit,
    RemotePos::AfterSplit,
    RemotePos::DifferentElement,
];

// --- split L1 ---

/// `<doc><p>ABCD</p><p>EFGH</p></doc>`.
async fn two_paragraphs(addr: &str, name: &str) -> TwoClients {
    TwoClients::with_tree(
        addr,
        name,
        doc_of(vec![p("ABCD"), p("EFGH")]),
        "<doc><p>ABCD</p><p>EFGH</p></doc>",
    )
    .await
}

/// JS `applyRemoteOp` of "multi client split L1 convergence".
fn apply_remote_op_l1(doc: &SharedDocument, op: RemoteOp, pos: RemotePos) {
    let index = |before, after, different| match pos {
        RemotePos::BeforeSplit => before,
        RemotePos::AfterSplit => after,
        RemotePos::DifferentElement => different,
    };
    edit_tree(doc, |t| match op {
        RemoteOp::InsertText => {
            let at = index(1, 5, 7);
            t.edit(at, at, vec![text("X")], 0)
        }
        RemoteOp::DeleteText => {
            let at = index(1, 4, 7);
            t.edit(at, at + 1, vec![], 0)
        }
        RemoteOp::InsertElement => {
            let at = index(0, 6, 12);
            t.edit(at, at, vec![p("NEW")], 0)
        }
    });
}

/// JS "Tree History - multi client split L1 convergence", `should converge:
/// split + remote ${remoteOp} at ${remotePos}`, for all 9 rows: d1 splits
/// the first `<p>` between B and C, d2 edits, then d1 undoes the split.
#[tokio::test]
async fn converges_after_undo_of_a_level_one_split_against_a_remote_edit() {
    let Some(addr) = rpc_addr() else { return };
    for op in REMOTE_OPS {
        for pos in REMOTE_POSITIONS {
            let case = format!("{op:?}-at-{pos:?}");
            let t = two_paragraphs(&addr, &format!("history-tree-split-l1-{case}")).await;

            edit_tree(&t.d1, |t| t.edit(3, 3, vec![], 1));
            apply_remote_op_l1(&t.d2, op, pos);
            t.sync_all().await;

            undo(&t.d1);
            t.sync_all().await;
            assert_xml_converged(&t, &format!("divergence: split + {case}"));

            t.deactivate().await;
        }
    }
}

/// JS "Tree History - split L1 edge cases", "should handle undo after
/// concurrent parent deletion (L1)": d2 deletes the `<p>` d1 split, and
/// d1's undo of the split converges.
#[tokio::test]
async fn converges_after_undo_of_a_level_one_split_whose_parent_was_deleted() {
    let Some(addr) = rpc_addr() else { return };
    let t = two_paragraphs(&addr, "history-tree-split-l1-parent-deleted").await;

    edit_tree(&t.d1, |t| t.edit(3, 3, vec![], 1));
    edit_tree(&t.d2, |t| t.edit(0, 6, vec![], 0));
    t.sync_all().await;

    undo(&t.d1);
    t.sync_all().await;
    assert_xml_converged(
        &t,
        "divergence after undo with concurrent parent deletion (L1)",
    );
    t.deactivate().await;
}

// --- split L2 ---

/// `<div><p>value</p></div>`.
fn div(value: &str) -> TreeNodeInput {
    element("div", vec![p(value)])
}

/// `<doc><div><p>ABCD</p></div><div><p>EFGH</p></div></doc>`.
///
/// ```text
/// <doc> <div> <p> A B C D </p> </div> <div> <p> E  F  G  H  </p> </div>
///   0     1    2  3 4 5 6   7     8      9   10 11 12 13 14  15     16
/// ```
async fn two_divs(addr: &str, name: &str) -> TwoClients {
    TwoClients::with_tree(
        addr,
        name,
        doc_of(vec![div("ABCD"), div("EFGH")]),
        "<doc><div><p>ABCD</p></div><div><p>EFGH</p></div></doc>",
    )
    .await
}

/// JS `applyRemoteOp` of "multi client split L2 convergence".
fn apply_remote_op_l2(doc: &SharedDocument, op: RemoteOp, pos: RemotePos) {
    let index = |before, after, different| match pos {
        RemotePos::BeforeSplit => before,
        RemotePos::AfterSplit => after,
        RemotePos::DifferentElement => different,
    };
    edit_tree(doc, |t| match op {
        RemoteOp::InsertText => {
            let at = index(3, 6, 11);
            t.edit(at, at, vec![text("X")], 0)
        }
        RemoteOp::DeleteText => {
            let at = index(2, 5, 10);
            t.edit(at, at + 1, vec![], 0)
        }
        RemoteOp::InsertElement => {
            let at = index(0, 8, 16);
            t.edit(at, at, vec![div("NEW")], 0)
        }
    });
}

/// JS "Tree History - multi client split L2 convergence", `should converge:
/// split L2 + remote ${remoteOp} at ${remotePos}` and `should converge after
/// redo: split L2 + remote ${remoteOp} at ${remotePos}`, for all 9 rows: d1
/// splits the first `<div><p>` between B and C with split level 2, d2
/// edits, then d1 undoes and redoes the split. The redo case repeats the
/// undo case's steps, so one run asserts both.
#[tokio::test]
async fn converges_after_undo_and_redo_of_a_level_two_split_against_a_remote_edit() {
    let Some(addr) = rpc_addr() else { return };
    for op in REMOTE_OPS {
        for pos in REMOTE_POSITIONS {
            let case = format!("{op:?}-at-{pos:?}");
            let t = two_divs(&addr, &format!("history-tree-split-l2-{case}")).await;

            edit_tree(&t.d1, |t| t.edit(4, 4, vec![], 2));
            apply_remote_op_l2(&t.d2, op, pos);
            t.sync_all().await;

            undo(&t.d1);
            t.sync_all().await;
            assert_xml_converged(&t, &format!("divergence: split L2 + {case}"));

            redo(&t.d1);
            t.sync_all().await;
            assert_xml_converged(&t, &format!("redo divergence: split L2 + {case}"));

            t.deactivate().await;
        }
    }
}

/// `<doc><div><p>AB</p></div></doc>`.
async fn one_div(addr: &str, name: &str) -> TwoClients {
    TwoClients::with_tree(
        addr,
        name,
        doc_of(vec![div("AB")]),
        "<doc><div><p>AB</p></div></doc>",
    )
    .await
}

/// JS "Tree History - multi client split L2 edge cases", "should converge:
/// undo L2 front split with remote insert".
#[tokio::test]
async fn converges_after_undo_of_a_level_two_front_split_against_an_insert() {
    let Some(addr) = rpc_addr() else { return };
    let t = one_div(&addr, "history-tree-split-l2-front").await;

    // d1: <doc><div><p></p></div><div><p>AB</p></div></doc>
    edit_tree(&t.d1, |t| t.edit(2, 2, vec![], 2));
    edit_tree(&t.d2, |t| t.edit(3, 3, vec![text("X")], 0));
    t.sync_all().await;

    undo(&t.d1);
    t.sync_all().await;
    assert_xml_converged(&t, "divergence: undo front L2 split with remote insert");
    t.deactivate().await;
}

/// JS "should converge: undo L2 back split with remote insert".
#[tokio::test]
async fn converges_after_undo_of_a_level_two_back_split_against_an_insert() {
    let Some(addr) = rpc_addr() else { return };
    let t = one_div(&addr, "history-tree-split-l2-back").await;

    // d1: <doc><div><p>AB</p></div><div><p></p></div></doc>
    edit_tree(&t.d1, |t| t.edit(4, 4, vec![], 2));
    edit_tree(&t.d2, |t| t.edit(2, 2, vec![text("X")], 0));
    t.sync_all().await;

    undo(&t.d1);
    t.sync_all().await;
    assert_xml_converged(&t, "divergence: undo back L2 split with remote insert");
    t.deactivate().await;
}

/// JS "should handle undo after concurrent parent deletion (L2)": d2
/// deletes the `<div>` d1 split, and d1's undo of the split converges.
#[tokio::test]
async fn converges_after_undo_of_a_level_two_split_whose_parent_was_deleted() {
    let Some(addr) = rpc_addr() else { return };
    let t = two_divs(&addr, "history-tree-split-l2-parent-deleted").await;

    edit_tree(&t.d1, |t| t.edit(4, 4, vec![], 2));
    edit_tree(&t.d2, |t| t.edit(0, 8, vec![], 0));
    t.sync_all().await;

    undo(&t.d1);
    t.sync_all().await;
    assert_xml_converged(
        &t,
        "divergence after undo with concurrent parent deletion (L2)",
    );
    t.deactivate().await;
}
