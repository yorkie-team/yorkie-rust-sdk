//! Integration tests for tree split and merge against a live server.
//!
//! JS references, all in `packages/sdk/test/integration/`:
//! - `tree_split_opinfo_test.ts`;
//! - `tree_concurrent_split_merge_test.ts`, with `removeStyleByPath`
//!   replaced by `removeStyle` over the same range;
//! - `tree_merge_anchor_test.ts`; the snapshot round trip at the end of the
//!   chained merge case is a `yorkie-core` wire test, as Rust clients have no
//!   snapshot encoder of their own;
//! - the split and merge cases of `tree_test.ts`: the two syncing
//!   `editByPath` cases, the overlapping merges, the unwrap, the contained
//!   and side-by-side split and merge cases with the merge, split and delete
//!   cases that follow them, and the split edge cases. Each runs the JS
//!   steps in order, assertion for assertion.
//!
//! The matrices of `tree_concurrency_test.ts` are in `integration_tree.rs`.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::tree::{
    active_client, collect_ops, edit_tree, element, element_with, manual_attach, set_tree, shared,
    text, xml, TwoClients,
};
use integration_support::{rpc_addr, unique_key};
use serde_json::json;
use yorkie::{
    Client, DeactivateOptions, DetachOptions, JsonTree, OperationInfo, SharedDocument,
    TreeNodeInput,
};

use Doc::{D1, D2};
use Step::{Converge, Edit, EditByPath, Expect, Sync};
use SyncClient::{C1, C2};

#[derive(Clone, Copy)]
enum Doc {
    D1,
    D2,
}

#[derive(Clone, Copy)]
enum SyncClient {
    C1,
    C2,
}

/// One step of a JS two-client case.
enum Step {
    /// `tree.edit(from, to, contents, split_level)` on one document.
    Edit(Doc, usize, usize, Vec<TreeNodeInput>, usize),
    /// `tree.edit_by_path(from, to, contents, split_level)`.
    EditByPath(Doc, Vec<usize>, Vec<usize>, Vec<TreeNodeInput>, usize),
    /// Asserts the XML of one document.
    Expect(Doc, &'static str),
    /// Asserts that both documents show the same tree.
    Converge,
    Sync(SyncClient),
}

/// Sets `root` from d1 and runs `steps` on two clients (JS
/// `withTwoClientsAndDocuments`).
async fn run_case(name: &str, root: TreeNodeInput, steps: Vec<Step>) {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, name).await;
    set_tree(&t.d1, root);
    for (index, step) in steps.into_iter().enumerate() {
        let doc = |doc: Doc| match doc {
            D1 => &t.d1,
            D2 => &t.d2,
        };
        match step {
            Edit(d, from, to, contents, split_level) => {
                edit_tree(doc(d), |tree| tree.edit(from, to, contents, split_level));
            }
            EditByPath(d, from, to, contents, split_level) => {
                edit_tree(doc(d), |tree| {
                    tree.edit_by_path(&from, &to, contents, split_level)
                });
            }
            Expect(d, expected) => assert_eq!(expected, xml(doc(d)), "{name}: step {index}"),
            Converge => assert_eq!(xml(&t.d1), xml(&t.d2), "{name}: step {index}"),
            Sync(client) => {
                let client = match client {
                    C1 => &t.c1,
                    C2 => &t.c2,
                };
                client.sync(None).await.unwrap();
            }
        }
    }
    t.deactivate().await;
}

/// JS `tree_split_opinfo_test.ts` "reports the split after text the receiver
/// typed at the boundary": alice types at the end of an inline element while
/// bob presses Enter there, a split of the inline element and then of the
/// paragraph. Alice's text stays left of the split, and the split she is
/// told about is after it.
#[tokio::test]
async fn reports_the_split_after_text_the_receiver_typed_at_the_boundary() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "split-opinfo",
        element(
            "doc",
            vec![element("p", vec![element("t", vec![text("가나다0123")])])],
        ),
        "<doc><p><t>가나다0123</t></p></doc>",
    )
    .await;

    edit_tree(&t.d1, |tree| {
        tree.edit_by_path(&[0, 0, 7], &[0, 0, 7], vec![text("4")], 0)
    });
    edit_tree(&t.d1, |tree| {
        tree.edit_by_path(&[0, 0, 8], &[0, 0, 8], vec![text("5")], 0)
    });
    edit_tree(&t.d2, |tree| {
        tree.edit_by_path(&[0, 0, 7], &[0, 0, 7], vec![], 1)?;
        tree.split_by_path(&[0, 1])
    });
    let ops = collect_ops(&t.d1);
    t.sync_all().await;

    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(
        "<doc><p><t>가나다012345</t></p><p><t></t></p></doc>",
        xml(&t.d1)
    );
    let paths = ops
        .lock()
        .unwrap()
        .iter()
        .map(|op| match op {
            OperationInfo::TreeEdit {
                from_path, to_path, ..
            } => (from_path.clone(), to_path.clone()),
            other => panic!("expected a tree edit, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vec![(vec![0, 0, 9], vec![0, 0, 9]), (vec![0, 1], vec![0, 1]),],
        paths
    );
    t.deactivate().await;
}

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

/// `<doc><p>` holding one `<span>` per text, the trees of JS
/// `tree_concurrent_split_merge_test.ts`.
fn spans(values: &[&str]) -> TreeNodeInput {
    let spans = values
        .iter()
        .map(|value| element("span", vec![text(value)]))
        .collect();
    element("doc", vec![element("p", spans)])
}

/// Runs `op1` on one replica and `op2` on the other without either seeing
/// the other, syncs them, and checks that they agree on `expected` (JS
/// `concurrently` of `tree_concurrent_split_merge_test.ts`).
async fn concurrently(
    name: &str,
    root: TreeNodeInput,
    op1: impl FnOnce(&mut JsonTree) -> yorkie::Result<()>,
    op2: impl FnOnce(&mut JsonTree) -> yorkie::Result<()>,
    expected: &str,
) {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, name).await;
    set_tree(&t.d1, root);
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    edit_tree(&t.d1, op1);
    edit_tree(&t.d2, op2);
    t.sync_all().await;

    assert_eq!(expected, xml(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    t.deactivate().await;
}

/// JS "does not duplicate content when two replicas split the same
/// position". As in JS, each replica contributes its own boundary, so an
/// empty span sits between the halves.
#[tokio::test]
async fn does_not_duplicate_content_when_two_replicas_split_the_same_position() {
    concurrently(
        "split-same-position",
        spans(&["abcde"]),
        |tree| tree.split_by_path(&[0, 0, 3]),
        |tree| tree.split_by_path(&[0, 0, 3]),
        "<doc><p><span>abc</span><span></span><span>de</span></p></doc>",
    )
    .await;
}

/// JS "does not duplicate content when two replicas merge the same
/// boundary".
#[tokio::test]
async fn does_not_duplicate_content_when_two_replicas_merge_the_same_boundary() {
    concurrently(
        "merge-same-boundary",
        spans(&["abc", "de"]),
        |tree| tree.merge_by_path(&[0, 1]),
        |tree| tree.merge_by_path(&[0, 1]),
        "<doc><p><span>abcde</span></p></doc>",
    )
    .await;
}

/// JS "keeps every span when two replicas merge neighbouring boundaries".
#[tokio::test]
async fn keeps_every_span_when_two_replicas_merge_neighbouring_boundaries() {
    concurrently(
        "merge-neighbouring-boundaries",
        spans(&["ab", "cd", "ef"]),
        |tree| tree.merge_by_path(&[0, 1]),
        |tree| tree.merge_by_path(&[0, 2]),
        "<doc><p><span>abcdef</span></p></doc>",
    )
    .await;
}

/// JS "keeps the text in order when two replicas split at different
/// positions".
#[tokio::test]
async fn keeps_the_text_in_order_when_two_replicas_split_at_different_positions() {
    concurrently(
        "split-different-positions",
        spans(&["abcde"]),
        |tree| tree.split_by_path(&[0, 0, 1]),
        |tree| tree.split_by_path(&[0, 0, 4]),
        "<doc><p><span>a</span><span>bcd</span><span>e</span></p></doc>",
    )
    .await;
}

/// JS "applies a concurrent style change to both halves of a split", with
/// `removeStyleByPath([0, 0], [0, 1])` as `remove_style` over the indexes of
/// those paths.
#[tokio::test]
async fn applies_a_concurrent_style_change_to_both_halves_of_a_split() {
    let root = element(
        "doc",
        vec![element(
            "p",
            vec![element_with(
                "span",
                json!({"bold": "true"}),
                vec![text("abcde")],
            )],
        )],
    );
    concurrently(
        "split-and-remove-style",
        root,
        |tree| tree.split_by_path(&[0, 0, 2]),
        |tree| {
            let (from, to) = (tree.path_to_index(&[0, 0])?, tree.path_to_index(&[0, 1])?);
            tree.remove_style(from, to, ["bold"])
        },
        "<doc><p><span>ab</span><span>cde</span></p></doc>",
    )
    .await;
}

/// JS "keeps the text in order when a split meets a merge".
#[tokio::test]
async fn keeps_the_text_in_order_when_a_split_meets_a_merge() {
    concurrently(
        "split-meets-merge",
        spans(&["abc", "de"]),
        |tree| tree.merge_by_path(&[0, 1]),
        |tree| tree.split_by_path(&[0, 0, 1]),
        "<doc><p><span>a</span><span>bcde</span></p></doc>",
    )
    .await;
}

/// `<r><p>ab</p><p>cd</p>...</r>` with one paragraph per value.
fn paragraphs(values: &[&str]) -> TreeNodeInput {
    let paragraphs = values
        .iter()
        .map(|value| element("p", vec![text(value)]))
        .collect();
    element("r", paragraphs)
}

/// JS `tree_merge_anchor_test.ts` "converges when inserting into a
/// concurrently removed range".
#[tokio::test]
async fn converges_when_inserting_into_a_concurrently_removed_range() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "insert-into-removed-range").await;
    set_tree(&t.d1, paragraphs(&["ab", "cd"]));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    edit_tree(&t.d1, |tree| tree.edit(6, 6, vec![element("p", vec![])], 0));
    edit_tree(&t.d2, |tree| tree.edit(0, 6, vec![], 0));
    t.sync_all().await;

    assert_eq!("<r><p></p>d</r>", xml(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    t.deactivate().await;
}

/// Three clients on one document in manual sync mode.
async fn attach_three(addr: &str, name: &str) -> Vec<(Client, SharedDocument)> {
    let key = unique_key(name);
    let mut replicas = Vec::new();
    for _ in 0..3 {
        let client = active_client(addr).await;
        let doc = client.attach(shared(&key), manual_attach()).await.unwrap();
        replicas.push((client, doc));
    }
    replicas
}

async fn sync_three_times(replicas: &[(Client, SharedDocument)]) {
    for _ in 0..3 {
        for (client, _) in replicas {
            client.sync(None).await.unwrap();
        }
    }
}

async fn deactivate_all(replicas: Vec<(Client, SharedDocument)>) {
    for (client, doc) in replicas {
        client.detach(&doc, DetachOptions).await.unwrap();
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "converges with two inserts at the same anchor in a removed range":
/// c1 and c2 insert at the same index while c3 removes the range the
/// inserts anchor into.
#[tokio::test]
async fn converges_with_two_inserts_at_the_same_anchor_in_a_removed_range() {
    let Some(addr) = rpc_addr() else { return };
    let replicas = attach_three(&addr, "two-inserts-removed-range").await;
    set_tree(&replicas[0].1, paragraphs(&["ab", "cd"]));
    for (client, _) in &replicas {
        client.sync(None).await.unwrap();
    }

    edit_tree(&replicas[0].1, |tree| {
        tree.edit(6, 6, vec![element("i", vec![])], 0)
    });
    edit_tree(&replicas[1].1, |tree| {
        tree.edit(6, 6, vec![element("b", vec![])], 0)
    });
    edit_tree(&replicas[2].1, |tree| tree.edit(0, 6, vec![], 0));
    sync_three_times(&replicas).await;

    assert_eq!(sorted_json(&replicas[0].1), sorted_json(&replicas[1].1));
    assert_eq!(sorted_json(&replicas[1].1), sorted_json(&replicas[2].1));
    deactivate_all(replicas).await;
}

/// JS "converges when insert is anchored in a chained merge": c1 merges P2
/// into P1 while c2 merges P3 into P2 and c3 inserts at the start of P3. The
/// insert survives between the remaining a and f.
#[tokio::test]
async fn converges_when_insert_is_anchored_in_a_chained_merge() {
    let Some(addr) = rpc_addr() else { return };
    let replicas = attach_three(&addr, "chained-merge-anchor").await;
    set_tree(&replicas[0].1, paragraphs(&["ab", "cd", "ef"]));
    for (client, _) in &replicas {
        client.sync(None).await.unwrap();
    }
    assert_eq!("<r><p>ab</p><p>cd</p><p>ef</p></r>", xml(&replicas[0].1));
    assert_eq!("<r><p>ab</p><p>cd</p><p>ef</p></r>", xml(&replicas[2].1));

    edit_tree(&replicas[0].1, |tree| tree.edit(2, 6, vec![], 0));
    edit_tree(&replicas[1].1, |tree| tree.edit(6, 10, vec![], 0));
    edit_tree(&replicas[2].1, |tree| {
        tree.edit(9, 9, vec![element("x", vec![])], 0)
    });
    assert_eq!("<r><p>ad</p><p>ef</p></r>", xml(&replicas[0].1));
    assert_eq!("<r><p>ab</p><p>cf</p></r>", xml(&replicas[1].1));
    assert_eq!(
        "<r><p>ab</p><p>cd</p><p><x></x>ef</p></r>",
        xml(&replicas[2].1)
    );
    sync_three_times(&replicas).await;

    assert_eq!("<r><p>a<x></x>f</p></r>", xml(&replicas[0].1));
    assert_eq!(sorted_json(&replicas[0].1), sorted_json(&replicas[1].1));
    assert_eq!(sorted_json(&replicas[0].1), sorted_json(&replicas[2].1));
    deactivate_all(replicas).await;
}

// The split and merge cases of JS `tree_test.ts`, step for step.

/// JS "overlapping-merge-and-merge" (`tree_test.ts:2692`).
#[tokio::test]
async fn overlapping_merge_and_merge() {
    run_case(
        "overlapping-merge-and-merge",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("b")]),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 5, 7, vec![], 0),
            Expect(D1, "<r><p>ab</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>bc</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>abc</p></r>"),
            Expect(D2, "<r><p>abc</p></r>"),
        ],
    )
    .await;
}

/// JS "overlapping-merge-and-delete-element-node" (`tree_test.ts:2729`).
#[tokio::test]
async fn overlapping_merge_and_delete_element_node() {
    run_case(
        "overlapping-merge-and-delete-element-node",
        element(
            "r",
            vec![element("p", vec![text("a")]), element("p", vec![text("b")])],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 3, 6, vec![], 0),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>a</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p></r>"),
            Expect(D2, "<r><p>a</p></r>"),
        ],
    )
    .await;
}

/// JS "overlapping-merge-and-delete-text-nodes" (`tree_test.ts:2758`).
#[tokio::test]
async fn overlapping_merge_and_delete_text_nodes() {
    run_case(
        "overlapping-merge-and-delete-text-nodes",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("bcde")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>bcde</p></r>"),
            Expect(D2, "<r><p>a</p><p>bcde</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 4, 6, vec![], 0),
            Expect(D1, "<r><p>abcde</p></r>"),
            Expect(D2, "<r><p>a</p><p>de</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ade</p></r>"),
            Expect(D2, "<r><p>ade</p></r>"),
        ],
    )
    .await;
}

/// JS "Can converge an unwrap with a concurrent delete of the same paragraph" (`tree_test.ts:2632`).
#[tokio::test]
async fn can_converge_an_unwrap_with_a_concurrent_delete_of_the_same_paragraph() {
    run_case(
        "can-converge-an-unwrap-with-a-concurrent-delete-of-the-same-paragraph",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Edit(D1, 0, 1, vec![], 0),
            Edit(D2, 0, 5, vec![], 0),
            Expect(D1, "<r><p>cd</p>ab</r>"),
            Expect(D2, "<r>cd</r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r>cd</r>"),
            Expect(D2, "<r>cd</r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-split-at-the-same-position" (`tree_test.ts:3084`).
#[tokio::test]
async fn contained_split_and_split_at_the_same_position() {
    run_case(
        "contained-split-and-split-at-the-same-position",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 2, 2, vec![], 1),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p></p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p></p><p>b</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-split-at-diffrent-positions-on-the-same-node" (`tree_test.ts:3118`).
#[tokio::test]
async fn contained_split_and_split_at_diffrent_positions_on_the_same_node() {
    run_case(
        "contained-split-and-split-at-diffrent-positions-on-the-same-node",
        element("r", vec![element("p", vec![text("abc")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>abc</p></r>"),
            Expect(D2, "<r><p>abc</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 3, 3, vec![], 1),
            Expect(D1, "<r><p>a</p><p>bc</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-split-at-different-levels" (`tree_test.ts:3152`).
#[tokio::test]
async fn contained_split_and_split_at_different_levels() {
    run_case(
        "contained-split-and-split-at-different-levels",
        element(
            "r",
            vec![element(
                "p",
                vec![
                    element("p", vec![text("ab")]),
                    element("p", vec![text("c")]),
                ],
            )],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p><p>ab</p><p>c</p></p></r>"),
            Expect(D2, "<r><p><p>ab</p><p>c</p></p></r>"),
            Edit(D1, 3, 3, vec![], 1),
            Edit(D2, 5, 5, vec![], 1),
            Expect(D1, "<r><p><p>a</p><p>b</p><p>c</p></p></r>"),
            Expect(D2, "<r><p><p>ab</p></p><p><p>c</p></p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p><p>a</p><p>b</p></p><p><p>c</p></p></r>"),
            Expect(D2, "<r><p><p>a</p><p>b</p></p><p><p>c</p></p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-insert-into-the-split-position" (`tree_test.ts:3206`).
#[tokio::test]
async fn contained_split_and_insert_into_the_split_position() {
    run_case(
        "contained-split-and-insert-into-the-split-position",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 2, 2, vec![text("c")], 0),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>acb</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ac</p><p>b</p></r>"),
            Expect(D2, "<r><p>ac</p><p>b</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-insert-into-original-node" (`tree_test.ts:3234`).
#[tokio::test]
async fn contained_split_and_insert_into_original_node() {
    run_case(
        "contained-split-and-insert-into-original-node",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 1, 1, vec![text("c")], 0),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>cab</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ca</p><p>b</p></r>"),
            Expect(D2, "<r><p>ca</p><p>b</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-insert-into-split-node" (`tree_test.ts:3262`).
#[tokio::test]
async fn contained_split_and_insert_into_split_node() {
    run_case(
        "contained-split-and-insert-into-split-node",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 3, 3, vec![text("c")], 0),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>abc</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p>bc</p></r>"),
            Expect(D2, "<r><p>a</p><p>bc</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-delete-contents-in-split-node" (`tree_test.ts:3290`).
#[tokio::test]
async fn contained_split_and_delete_contents_in_split_node() {
    run_case(
        "contained-split-and-delete-contents-in-split-node",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 2, 3, vec![], 0),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p></p></r>"),
            Expect(D2, "<r><p>a</p><p></p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-delete-the-whole-original-and-split-nodes" (`tree_test.ts:3318`).
#[tokio::test]
async fn contained_split_and_delete_the_whole_original_and_split_nodes() {
    run_case(
        "contained-split-and-delete-the-whole-original-and-split-nodes",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 0, 4, vec![], 0),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r></r>"),
            Expect(D2, "<r></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-merge-at-different-levels" (`tree_test.ts:3346`).
#[tokio::test]
async fn contained_merge_and_merge_at_different_levels() {
    run_case(
        "contained-merge-and-merge-at-different-levels",
        element(
            "r",
            vec![
                element(
                    "p",
                    vec![element("p", vec![text("a")]), element("p", vec![text("b")])],
                ),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p><p>a</p><p>b</p></p><p>c</p></r>"),
            Expect(D2, "<r><p><p>a</p><p>b</p></p><p>c</p></r>"),
            Edit(D1, 3, 5, vec![], 0),
            Edit(D2, 7, 9, vec![], 0),
            Expect(D1, "<r><p><p>ab</p></p><p>c</p></r>"),
            Expect(D2, "<r><p><p>a</p><p>b</p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p><p>ab</p>c</p></r>"),
            Expect(D2, "<r><p><p>ab</p>c</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-merge-at-the-same-level" (`tree_test.ts:3395`).
#[tokio::test]
async fn contained_merge_and_merge_at_the_same_level() {
    run_case(
        "contained-merge-and-merge-at-the-same-level",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("b")]),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Edit(D1, 2, 7, vec![], 0),
            Edit(D2, 5, 7, vec![], 0),
            Expect(D1, "<r><p>ac</p></r>"),
            Expect(D2, "<r><p>a</p><p>bc</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ac</p></r>"),
            Expect(D2, "<r><p>ac</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-insert" (`tree_test.ts:3431`).
#[tokio::test]
async fn contained_merge_and_insert() {
    run_case(
        "contained-merge-and-insert",
        element(
            "r",
            vec![element("p", vec![text("a")]), element("p", vec![text("b")])],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 4, 4, vec![text("c")], 0),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>a</p><p>cb</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>acb</p></r>"),
            Expect(D2, "<r><p>acb</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-delete-the-whole" (`tree_test.ts:3460`).
#[tokio::test]
async fn contained_merge_and_delete_the_whole() {
    run_case(
        "contained-merge-and-delete-the-whole",
        element(
            "r",
            vec![element("p", vec![text("a")]), element("p", vec![text("b")])],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 0, 6, vec![], 0),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r></r>"),
            Expect(D2, "<r></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-delete-contents-in-merged-node" (`tree_test.ts:3489`).
#[tokio::test]
async fn contained_merge_and_delete_contents_in_merged_node() {
    run_case(
        "contained-merge-and-delete-contents-in-merged-node",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("bc")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>bc</p></r>"),
            Expect(D2, "<r><p>a</p><p>bc</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 4, 5, vec![], 0),
            Expect(D1, "<r><p>abc</p></r>"),
            Expect(D2, "<r><p>a</p><p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ac</p></r>"),
            Expect(D2, "<r><p>ac</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-delete-sub-range-in-merged-range" (`tree_test.ts:3520`).
#[tokio::test]
async fn contained_merge_and_delete_sub_range_in_merged_range() {
    run_case(
        "contained-merge-and-delete-sub-range-in-merged-range",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("b")]),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Edit(D1, 2, 7, vec![], 0),
            Edit(D2, 3, 6, vec![], 0),
            Expect(D1, "<r><p>ac</p></r>"),
            Expect(D2, "<r><p>a</p><p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ac</p></r>"),
            Expect(D2, "<r><p>ac</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-split-merged-node" (`tree_test.ts:3558`).
#[tokio::test]
async fn contained_merge_and_split_merged_node() {
    run_case(
        "contained-merge-and-split-merged-node",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("bc")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>bc</p></r>"),
            Expect(D2, "<r><p>a</p><p>bc</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 5, 5, vec![], 1),
            Expect(D1, "<r><p>abc</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ab</p><p>c</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-merge-and-split-at-multi-levels" (`tree_test.ts:3590`).
#[tokio::test]
async fn contained_merge_and_split_at_multi_levels() {
    run_case(
        "contained-merge-and-split-at-multi-levels",
        element(
            "r",
            vec![element(
                "p",
                vec![element("p", vec![text("a")]), element("p", vec![text("b")])],
            )],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p><p>a</p><p>b</p></p></r>"),
            Expect(D2, "<r><p><p>a</p><p>b</p></p></r>"),
            Edit(D1, 3, 5, vec![], 0),
            Edit(D2, 4, 4, vec![], 1),
            Expect(D1, "<r><p><p>ab</p></p></r>"),
            Expect(D2, "<r><p><p>a</p></p><p><p>b</p></p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p><p>ab</p></p><p></p></r>"),
            Expect(D2, "<r><p><p>ab</p></p><p></p></r>"),
        ],
    )
    .await;
}

/// JS "side-by-side-split-and-split" (`tree_test.ts:4130`).
#[tokio::test]
async fn side_by_side_split_and_split() {
    run_case(
        "side-by-side-split-and-split",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p><p>cd</p></r>"),
            Expect(D2, "<r><p>ab</p><p>cd</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 6, 6, vec![], 1),
            Expect(D1, "<r><p>a</p><p>b</p><p>cd</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p><p>d</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p><p>d</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p><p>d</p></r>"),
        ],
    )
    .await;
}

/// JS "side-by-side-split-and-insert" (`tree_test.ts:4177`).
#[tokio::test]
async fn side_by_side_split_and_insert() {
    run_case(
        "side-by-side-split-and-insert",
        element("r", vec![element("p", vec![text("ab")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 4, 4, vec![element("p", vec![text("c")])], 0),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
        ],
    )
    .await;
}

/// JS "side-by-side-split-and-delete" (`tree_test.ts:4211`).
#[tokio::test]
async fn side_by_side_split_and_delete() {
    run_case(
        "side-by-side-split-and-delete",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p><p>c</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 4, 7, vec![], 0),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
        ],
    )
    .await;
}

/// JS "cascade-delete-across-parent-after-multi-level-split" (`tree_test.ts:4243`).
#[tokio::test]
async fn cascade_delete_across_parent_after_multi_level_split() {
    run_case(
        "cascade-delete-across-parent-after-multi-level-split",
        element(
            "r",
            vec![element(
                "p",
                vec![
                    element("p", vec![text("ab")]),
                    element("p", vec![text("cd")]),
                ],
            )],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Edit(D1, 3, 3, vec![], 2),
            Edit(D2, 1, 6, vec![], 0),
            Expect(D1, "<r><p><p>a</p></p><p><p>b</p><p>cd</p></p></r>"),
            Expect(D2, "<r><p>cd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
            Expect(D1, "<r><p>cd</p><p></p></r>"),
        ],
    )
    .await;
}

/// JS "sequential-merge-then-split" (`tree_test.ts:4280`).
#[tokio::test]
async fn sequential_merge_then_split() {
    run_case(
        "sequential-merge-then-split",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Edit(D1, 3, 5, vec![], 0),
            Expect(D1, "<r><p>abcd</p></r>"),
            Sync(C1),
            Sync(C2),
            Expect(D2, "<r><p>abcd</p></r>"),
            Edit(D2, 3, 3, vec![], 1),
            Expect(D2, "<r><p>ab</p><p>cd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
            Expect(D1, "<r><p>ab</p><p>cd</p></r>"),
        ],
    )
    .await;
}

/// JS "multi-level-split-with-concurrent-merge-and-text-split" (`tree_test.ts:4320`).
#[tokio::test]
async fn multi_level_split_with_concurrent_merge_and_text_split() {
    run_case(
        "multi-level-split-with-concurrent-merge-and-text-split",
        element(
            "r",
            vec![element(
                "p",
                vec![
                    element("p", vec![text("ab")]),
                    element("p", vec![text("cd")]),
                ],
            )],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Edit(D1, 3, 3, vec![], 2),
            Edit(D2, 1, 6, vec![], 0),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
        ],
    )
    .await;
}

/// JS "split-with-concurrent-delete-overlapping-content" (`tree_test.ts:4361`).
#[tokio::test]
async fn split_with_concurrent_delete_overlapping_content() {
    run_case(
        "split-with-concurrent-delete-overlapping-content",
        element("r", vec![element("p", vec![text("abcd")])]),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>abcd</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 3, 3, vec![], 1),
            Expect(D1, "<r><p>ad</p></r>"),
            Expect(D2, "<r><p>ab</p><p>cd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
            Expect(D1, "<r><p>a</p><p>d</p></r>"),
        ],
    )
    .await;
}

/// JS "merge-with-concurrent-content-delete" (`tree_test.ts:4396`).
#[tokio::test]
async fn merge_with_concurrent_content_delete() {
    run_case(
        "merge-with-concurrent-content-delete",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Edit(D1, 2, 3, vec![], 0),
            Edit(D2, 3, 5, vec![], 0),
            Expect(D1, "<r><p>a</p><p>cd</p></r>"),
            Expect(D2, "<r><p>abcd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
            Expect(D1, "<r><p>acd</p></r>"),
        ],
    )
    .await;
}

/// JS "merge-with-concurrent-full-content-delete-in-source" (`tree_test.ts:4423`).
#[tokio::test]
async fn merge_with_concurrent_full_content_delete_in_source() {
    run_case(
        "merge-with-concurrent-full-content-delete-in-source",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Edit(D1, 5, 7, vec![], 0),
            Edit(D2, 3, 5, vec![], 0),
            Expect(D1, "<r><p>ab</p><p></p></r>"),
            Expect(D2, "<r><p>abcd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
            Expect(D1, "<r><p>ab</p></r>"),
        ],
    )
    .await;
}

/// JS "side-by-side-merge-and-merge" (`tree_test.ts:4452`).
#[tokio::test]
async fn side_by_side_merge_and_merge() {
    run_case(
        "side-by-side-merge-and-merge",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("b")]),
                element("p", vec![text("c")]),
                element("p", vec![text("d")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p><p>d</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p><p>d</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 8, 10, vec![], 0),
            Expect(D1, "<r><p>ab</p><p>c</p><p>d</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>cd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ab</p><p>cd</p></r>"),
            Expect(D2, "<r><p>ab</p><p>cd</p></r>"),
        ],
    )
    .await;
}

/// JS "contained-split-and-merge-same-block" (`tree_test.ts:4504`).
#[tokio::test]
async fn contained_split_and_merge_same_block() {
    run_case(
        "contained-split-and-merge-same-block",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p><p>cd</p></r>"),
            Edit(D1, 2, 2, vec![], 1),
            Edit(D2, 3, 5, vec![], 0),
            Expect(D1, "<r><p>a</p><p>b</p><p>cd</p></r>"),
            Expect(D2, "<r><p>abcd</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Converge,
        ],
    )
    .await;
}

/// JS "side-by-side-merge-and-insert" (`tree_test.ts:4539`).
#[tokio::test]
async fn side_by_side_merge_and_insert() {
    run_case(
        "side-by-side-merge-and-insert",
        element(
            "r",
            vec![element("p", vec![text("a")]), element("p", vec![text("b")])],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 6, 6, vec![element("p", vec![text("c")])], 0),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ab</p><p>c</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p></r>"),
        ],
    )
    .await;
}

/// JS "side-by-side-merge-and-delete" (`tree_test.ts:4573`).
#[tokio::test]
async fn side_by_side_merge_and_delete() {
    run_case(
        "side-by-side-merge-and-delete",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("b")]),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 6, 9, vec![], 0),
            Expect(D1, "<r><p>ab</p><p>c</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
        ],
    )
    .await;
}

/// JS "side-by-side-merge-and-split" (`tree_test.ts:4609`).
#[tokio::test]
async fn side_by_side_merge_and_split() {
    run_case(
        "side-by-side-merge-and-split",
        element(
            "r",
            vec![
                element("p", vec![text("a")]),
                element("p", vec![text("b")]),
                element("p", vec![text("cd")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>a</p><p>b</p><p>cd</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>cd</p></r>"),
            Edit(D1, 2, 4, vec![], 0),
            Edit(D2, 8, 8, vec![], 1),
            Expect(D1, "<r><p>ab</p><p>cd</p></r>"),
            Expect(D2, "<r><p>a</p><p>b</p><p>c</p><p>d</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ab</p><p>c</p><p>d</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p><p>d</p></r>"),
        ],
    )
    .await;
}

/// JS "concurrent-delete-after-merge-with-nested-content" (`tree_test.ts:4706`).
#[tokio::test]
async fn concurrent_delete_after_merge_with_nested_content() {
    run_case(
        "concurrent-delete-after-merge-with-nested-content",
        element(
            "r",
            vec![
                element("p", vec![element("b", vec![text("a")])]),
                element("p", vec![element("b", vec![text("b")])]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p><b>a</b></p><p><b>b</b></p></r>"),
            Expect(D2, "<r><p><b>a</b></p><p><b>b</b></p></r>"),
            Edit(D1, 4, 6, vec![], 0),
            Edit(D2, 0, 10, vec![], 0),
            Expect(D1, "<r><p><b>a</b><b>b</b></p></r>"),
            Expect(D2, "<r></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r></r>"),
            Expect(D2, "<r></r>"),
        ],
    )
    .await;
}

/// JS "delete-starting-inside-merge-target" (`tree_test.ts:4755`).
#[tokio::test]
async fn delete_starting_inside_merge_target() {
    run_case(
        "delete-starting-inside-merge-target",
        element(
            "r",
            vec![
                element("p", vec![text("ab")]),
                element("p", vec![text("c")]),
            ],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(D1, "<r><p>ab</p><p>c</p></r>"),
            Expect(D2, "<r><p>ab</p><p>c</p></r>"),
            Edit(D1, 3, 5, vec![], 0),
            Edit(D2, 3, 7, vec![], 0),
            Expect(D1, "<r><p>abc</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
            Sync(C1),
            Sync(C2),
            Sync(C1),
            Expect(D1, "<r><p>ab</p></r>"),
            Expect(D2, "<r><p>ab</p></r>"),
        ],
    )
    .await;
}

/// JS "Can split link can transmitted through rpc" (`tree_test.ts:5257`).
#[tokio::test]
async fn can_split_link_can_transmitted_through_rpc() {
    run_case(
        "can-split-link-can-transmitted-through-rpc",
        element("doc", vec![element("p", vec![text("ab")])]),
        vec![
            Edit(D1, 2, 2, vec![text("1")], 0),
            Sync(C1),
            Sync(C2),
            Expect(D1, "<doc><p>a1b</p></doc>"),
            Expect(D2, "<doc><p>a1b</p></doc>"),
            Edit(D2, 3, 3, vec![text("1")], 0),
            Expect(D2, "<doc><p>a11b</p></doc>"),
            Edit(D2, 2, 3, vec![text("12")], 0),
            Edit(D2, 4, 5, vec![text("21")], 0),
            Expect(D2, "<doc><p>a1221b</p></doc>"),
            Edit(D2, 2, 4, vec![text("123")], 0),
            Expect(D2, "<doc><p>a12321b</p></doc>"),
        ],
    )
    .await;
}

/// JS "Can split and merge with empty paragraph: left" (`tree_test.ts:5428`).
#[tokio::test]
async fn can_split_and_merge_with_empty_paragraph_left() {
    run_case(
        "can-split-and-merge-with-empty-paragraph-left",
        element("doc", vec![element("p", vec![text("a"), text("b")])]),
        vec![
            Expect(D1, "<doc><p>ab</p></doc>"),
            Edit(D1, 1, 1, vec![], 1),
            Expect(D1, "<doc><p></p><p>ab</p></doc>"),
            Edit(D1, 1, 3, vec![], 0),
            Expect(D1, "<doc><p>ab</p></doc>"),
            Sync(C1),
            Sync(C2),
            Converge,
        ],
    )
    .await;
}

/// JS "Can split and merge with empty paragraph: right" (`tree_test.ts:5462`).
#[tokio::test]
async fn can_split_and_merge_with_empty_paragraph_right() {
    run_case(
        "can-split-and-merge-with-empty-paragraph-right",
        element("doc", vec![element("p", vec![text("a"), text("b")])]),
        vec![
            Expect(D1, "<doc><p>ab</p></doc>"),
            Edit(D1, 3, 3, vec![], 1),
            Expect(D1, "<doc><p>ab</p><p></p></doc>"),
            Edit(D1, 3, 5, vec![], 0),
            Expect(D1, "<doc><p>ab</p></doc>"),
            Sync(C1),
            Sync(C2),
            Converge,
        ],
    )
    .await;
}

/// JS "Can split and merge with empty paragraph and multiple split level: left" (`tree_test.ts:5496`).
#[tokio::test]
async fn can_split_and_merge_with_empty_paragraph_and_multiple_split_level_left() {
    run_case(
        "can-split-and-merge-with-empty-paragraph-and-multiple-split-level-left",
        element(
            "doc",
            vec![element("p", vec![element("p", vec![text("a"), text("b")])])],
        ),
        vec![
            Expect(D1, "<doc><p><p>ab</p></p></doc>"),
            Edit(D1, 2, 2, vec![], 2),
            Expect(D1, "<doc><p><p></p></p><p><p>ab</p></p></doc>"),
            Edit(D1, 2, 6, vec![], 0),
            Expect(D1, "<doc><p><p>ab</p></p></doc>"),
            Sync(C1),
            Sync(C2),
            Converge,
        ],
    )
    .await;
}

/// JS "Can split at the same offset multiple times" (`tree_test.ts:5541`).
#[tokio::test]
async fn can_split_at_the_same_offset_multiple_times() {
    run_case(
        "can-split-at-the-same-offset-multiple-times",
        element("doc", vec![element("p", vec![text("a"), text("b")])]),
        vec![
            Expect(D1, "<doc><p>ab</p></doc>"),
            Edit(D1, 2, 2, vec![], 1),
            Expect(D1, "<doc><p>a</p><p>b</p></doc>"),
            Edit(D1, 2, 2, vec![text("c")], 0),
            Expect(D1, "<doc><p>ac</p><p>b</p></doc>"),
            Edit(D1, 2, 2, vec![], 1),
            Expect(D1, "<doc><p>a</p><p>c</p><p>b</p></doc>"),
            Edit(D1, 2, 7, vec![], 0),
            Expect(D1, "<doc><p>ab</p></doc>"),
            Sync(C1),
            Sync(C2),
            Converge,
        ],
    )
    .await;
}

/// JS "Can sync editByPath split with other clients" (`tree_test.ts:738`).
#[tokio::test]
async fn can_sync_editbypath_split_with_other_clients() {
    run_case(
        "can-sync-editbypath-split-with-other-clients",
        element(
            "doc",
            vec![element(
                "tc",
                vec![
                    element("p", vec![element("tn", vec![text("1234")])]),
                    element("p", vec![element("tn", vec![text("5678")])]),
                ],
            )],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(
                D1,
                "<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            Expect(
                D2,
                "<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            EditByPath(D1, vec![0, 0, 0, 2], vec![0, 0, 0, 2], vec![], 1),
            Sync(C1),
            Sync(C2),
            Expect(
                D1,
                "<doc><tc><p><tn>12</tn><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            Expect(
                D2,
                "<doc><tc><p><tn>12</tn><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            EditByPath(D1, vec![0, 0, 1], vec![0, 0, 1], vec![], 1),
            Sync(C1),
            Sync(C2),
            Expect(
                D1,
                "<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            Expect(
                D2,
                "<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
        ],
    )
    .await;
}

/// JS "Can sync editByPath merge with other clients" (`tree_test.ts:815`).
#[tokio::test]
async fn can_sync_editbypath_merge_with_other_clients() {
    run_case(
        "can-sync-editbypath-merge-with-other-clients",
        element(
            "doc",
            vec![element(
                "tc",
                vec![
                    element("p", vec![element("tn", vec![text("1234")])]),
                    element("p", vec![element("tn", vec![text("5678")])]),
                ],
            )],
        ),
        vec![
            Sync(C1),
            Sync(C2),
            Expect(
                D1,
                "<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            Expect(
                D2,
                "<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            EditByPath(D1, vec![0, 0, 1], vec![0, 1, 0], vec![], 0),
            Sync(C1),
            Sync(C2),
            Expect(D1, "<doc><tc><p><tn>1234</tn><tn>5678</tn></p></tc></doc>"),
            Expect(D2, "<doc><tc><p><tn>1234</tn><tn>5678</tn></p></tc></doc>"),
            EditByPath(D1, vec![0, 0, 0, 4], vec![0, 0, 1, 0], vec![], 0),
            Sync(C1),
            Sync(C2),
            Expect(D1, "<doc><tc><p><tn>12345678</tn></p></tc></doc>"),
            Expect(D2, "<doc><tc><p><tn>12345678</tn></p></tc></doc>"),
        ],
    )
    .await;
}
