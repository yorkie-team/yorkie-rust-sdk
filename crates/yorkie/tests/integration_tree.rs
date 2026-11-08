//! Integration tests for concurrent tree editing against a live server.
//!
//! JS references: `packages/sdk/test/integration/tree_test.ts` (the
//! two-client cases of "Tree", "Tree.style", "Tree.edit(concurrent ...)" and
//! "TreeChange" that do not split or merge; `integration_tree_split.rs` has
//! those) and `packages/sdk/test/integration/tree_concurrency_test.ts` (all
//! its matrices, merge and split operations included). Like the JS
//! `withTwoClientsAndDocuments` cases, each test syncs two clients manually.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::tree::{
    attributes, collect_ops, edit_tree, element, element_with, set_tree, text, xml,
};
use integration_support::two_clients::{active_client, attach_pair, TwoClients};
use integration_support::{rpc_addr, unique_key};
use serde_json::json;
use std::sync::Mutex;
use yorkie::{
    DeactivateOptions, DetachOptions, OperationInfo, SharedDocument, TreeNodeInput,
    TreeStyleOpValue,
};

/// A tree edit op info as JS tests compare it: `(from, to, value)`.
type EditSummary = (usize, usize, Option<Vec<TreeNodeInput>>);

fn edit_summaries(ops: &Mutex<Vec<OperationInfo>>) -> Vec<EditSummary> {
    ops.lock()
        .unwrap()
        .iter()
        .map(|op| match op {
            OperationInfo::TreeEdit {
                from, to, value, ..
            } => (*from, *to, value.clone()),
            other => panic!("expected a tree edit, got {other:?}"),
        })
        .collect()
}

fn doc_p(value: &str) -> TreeNodeInput {
    element("doc", vec![element("p", vec![text(value)])])
}

/// JS "Can sync its content with other clients".
#[tokio::test]
async fn syncs_its_content_with_other_clients() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-sync",
        doc_p("hello"),
        "<doc><p>hello</p></doc>",
    )
    .await;

    edit_tree(&t.d1, |tree| {
        tree.edit(7, 7, vec![element("p", vec![text("yorkie")])], 0)
    });
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!("<doc><p>hello</p><p>yorkie</p></doc>", xml(&t.d1));
    assert_eq!("<doc><p>hello</p><p>yorkie</p></doc>", xml(&t.d2));

    t.deactivate().await;
}

fn doc_p_italic() -> TreeNodeInput {
    element(
        "doc",
        vec![element_with(
            "p",
            json!({"italic": "true"}),
            vec![text("hello")],
        )],
    )
}

/// JS "Can sync its content containing attributes with other replicas".
#[tokio::test]
async fn syncs_its_content_containing_attributes() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-sync-attributes",
        doc_p_italic(),
        r#"<doc><p italic="true">hello</p></doc>"#,
    )
    .await;

    edit_tree(&t.d1, |tree| {
        tree.style(0, 1, attributes(json!({"bold": "true"})))
    });
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    let expected = r#"<doc><p bold="true" italic="true">hello</p></doc>"#;
    assert_eq!(expected, xml(&t.d1));
    assert_eq!(expected, xml(&t.d2));

    t.deactivate().await;
}

/// JS "Can sync its content with remove style".
#[tokio::test]
async fn syncs_its_content_with_remove_style() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-sync-remove-style",
        doc_p_italic(),
        r#"<doc><p italic="true">hello</p></doc>"#,
    )
    .await;

    edit_tree(&t.d1, |tree| tree.remove_style(0, 1, ["italic"]));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!("<doc><p>hello</p></doc>", xml(&t.d1));
    assert_eq!("<doc><p>hello</p></doc>", xml(&t.d2));

    t.deactivate().await;
}

/// JS "Should return correct range path within doc.subscribe": edits by path
/// from both clients, then checks the paths of a local deletion's op info.
#[tokio::test]
async fn reports_the_path_range_of_an_edit() {
    let Some(addr) = rpc_addr() else { return };
    let p_n = || element("p", vec![element("n", vec![])]);
    let t = TwoClients::with_tree(
        &addr,
        "tree-path-range",
        element(
            "r",
            vec![
                element("c", vec![element("u", vec![p_n()])]),
                element("c", vec![p_n()]),
            ],
        ),
        "<r><c><u><p><n></n></p></u></c><c><p><n></n></p></c></r>",
    )
    .await;

    for (offset, value) in [(0, "1"), (1, "2"), (2, "3")] {
        edit_tree(&t.d2, |tree| {
            tree.edit_by_path(&[1, 0, 0, offset], &[1, 0, 0, offset], vec![text(value)], 0)
        });
    }
    t.c2.sync(None).await.unwrap();
    t.c1.sync(None).await.unwrap();
    let expected = "<r><c><u><p><n></n></p></u></c><c><p><n>123</n></p></c></r>";
    assert_eq!(expected, xml(&t.d1));
    assert_eq!(expected, xml(&t.d2));

    edit_tree(&t.d1, |tree| {
        tree.edit_by_path(&[1, 0, 0, 1], &[1, 0, 0, 1], vec![text("abcdefgh")], 0)
    });
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    let expected = "<r><c><u><p><n></n></p></u></c><c><p><n>1abcdefgh23</n></p></c></r>";
    assert_eq!(expected, xml(&t.d1));
    assert_eq!(expected, xml(&t.d2));

    edit_tree(&t.d2, |tree| {
        tree.edit_by_path(&[1, 0, 0, 5], &[1, 0, 0, 5], vec![text("4")], 0)
    });
    edit_tree(&t.d2, |tree| {
        tree.edit_by_path(&[1, 0, 0, 6], &[1, 0, 0, 7], vec![], 0)
    });
    edit_tree(&t.d2, |tree| {
        tree.edit_by_path(&[1, 0, 0, 6], &[1, 0, 0, 6], vec![text("5")], 0)
    });
    t.c2.sync(None).await.unwrap();
    t.c1.sync(None).await.unwrap();

    let ops = collect_ops(&t.d2);
    edit_tree(&t.d2, |tree| {
        tree.edit_by_path(&[1, 0, 0, 7], &[1, 0, 0, 8], vec![], 0)
    });
    t.c2.sync(None).await.unwrap();
    t.c1.sync(None).await.unwrap();
    let expected = "<r><c><u><p><n></n></p></u></c><c><p><n>1abcd45gh23</n></p></c></r>";
    assert_eq!(expected, xml(&t.d1));
    assert_eq!(expected, xml(&t.d2));

    let ops = ops.lock().unwrap().clone();
    assert_eq!(1, ops.len());
    let OperationInfo::TreeEdit {
        from_path, to_path, ..
    } = &ops[0]
    else {
        panic!("expected a tree edit, got {:?}", ops[0]);
    };
    assert_eq!(&vec![1, 0, 0, 7], from_path);
    assert_eq!(&vec![1, 0, 0, 8], to_path);

    t.deactivate().await;
}

/// JS "Can concurrently delete overlapping elements".
#[tokio::test]
async fn concurrent_overlapping_element_deletions_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-delete-overlapping-elements",
        element(
            "r",
            vec![
                element("p", vec![]),
                element("i", vec![]),
                element("b", vec![]),
            ],
        ),
        "<r><p></p><i></i><b></b></r>",
    )
    .await;

    edit_tree(&t.d1, |tree| tree.edit(0, 4, vec![], 0));
    edit_tree(&t.d2, |tree| tree.edit(2, 6, vec![], 0));
    assert_eq!("<r><b></b></r>", xml(&t.d1));
    assert_eq!("<r><p></p></r>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!("<r></r>", xml(&t.d1));
    assert_eq!("<r></r>", xml(&t.d2));

    t.deactivate().await;
}

/// JS "Can concurrently delete overlapping text".
#[tokio::test]
async fn concurrent_overlapping_text_deletions_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-delete-overlapping-text",
        element("r", vec![element("p", vec![text("abcd")])]),
        "<r><p>abcd</p></r>",
    )
    .await;

    edit_tree(&t.d1, |tree| tree.edit(1, 4, vec![], 0));
    edit_tree(&t.d2, |tree| tree.edit(2, 5, vec![], 0));
    assert_eq!("<r><p>d</p></r>", xml(&t.d1));
    assert_eq!("<r><p>a</p></r>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!("<r><p></p></r>", xml(&t.d1));
    assert_eq!("<r><p></p></r>", xml(&t.d2));

    t.deactivate().await;
}

/// JS "Can concurrently insert and delete contained elements of the same
/// depth": an element inserted inside a range deleted concurrently survives.
#[tokio::test]
async fn concurrent_insert_inside_a_deleted_range_converges() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-insert-delete-contained",
        element(
            "r",
            vec![
                element("p", vec![text("1234")]),
                element("p", vec![text("abcd")]),
            ],
        ),
        "<r><p>1234</p><p>abcd</p></r>",
    )
    .await;

    edit_tree(&t.d1, |tree| tree.edit(6, 6, vec![element("p", vec![])], 0));
    edit_tree(&t.d2, |tree| tree.edit(0, 12, vec![], 0));
    assert_eq!("<r><p>1234</p><p></p><p>abcd</p></r>", xml(&t.d1));
    assert_eq!("<r></r>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!("<r><p></p></r>", xml(&t.d1));
    assert_eq!("<r><p></p></r>", xml(&t.d2));

    t.deactivate().await;
}

/// JS "Can concurrently insert side by side elements (left)".
#[tokio::test]
async fn concurrent_inserts_at_the_same_element_position_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-insert-side-by-side",
        element("r", vec![element("p", vec![])]),
        "<r><p></p></r>",
    )
    .await;

    edit_tree(&t.d1, |tree| tree.edit(0, 0, vec![element("b", vec![])], 0));
    edit_tree(&t.d2, |tree| tree.edit(0, 0, vec![element("i", vec![])], 0));
    assert_eq!("<r><b></b><p></p></r>", xml(&t.d1));
    assert_eq!("<r><i></i><p></p></r>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!("<r><i></i><b></b><p></p></r>", xml(&t.d1));
    assert_eq!("<r><i></i><b></b><p></p></r>", xml(&t.d2));

    t.deactivate().await;
}

/// JS "Can insert text to the same position(left) concurrently".
#[tokio::test]
async fn concurrent_inserts_at_the_same_text_position_converge() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-insert-text-same-position",
        element("r", vec![element("p", vec![text("12")])]),
        "<r><p>12</p></r>",
    )
    .await;

    edit_tree(&t.d1, |tree| tree.edit(1, 1, vec![text("A")], 0));
    edit_tree(&t.d2, |tree| tree.edit(1, 1, vec![text("B")], 0));
    assert_eq!("<r><p>A12</p></r>", xml(&t.d1));
    assert_eq!("<r><p>B12</p></r>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!("<r><p>BA12</p></r>", xml(&t.d1));
    assert_eq!("<r><p>BA12</p></r>", xml(&t.d2));

    t.deactivate().await;
}

/// JS "TreeChange: Concurrent delete and delete".
#[tokio::test]
async fn reports_concurrent_deletions() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-change-delete-delete",
        doc_p("ab"),
        "<doc><p>ab</p></doc>",
    )
    .await;
    let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

    edit_tree(&t.d1, |tree| tree.edit(0, 4, vec![], 0));
    assert_eq!("<doc></doc>", xml(&t.d1));
    edit_tree(&t.d2, |tree| tree.edit(1, 2, vec![], 0));
    assert_eq!("<doc><p>b</p></doc>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(vec![(0, 4, None)], edit_summaries(&ops1));
    assert_eq!(vec![(1, 2, None), (0, 3, None)], edit_summaries(&ops2));

    t.deactivate().await;
}

/// JS "TreeChange: Concurrent delete and insert": the remote deletion skips
/// the text it did not see, so it reports two ranges.
#[tokio::test]
async fn reports_a_deletion_split_by_a_concurrent_insert() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-change-delete-insert",
        doc_p("ab"),
        "<doc><p>ab</p></doc>",
    )
    .await;
    let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

    edit_tree(&t.d1, |tree| tree.edit(1, 3, vec![], 0));
    assert_eq!("<doc><p></p></doc>", xml(&t.d1));
    edit_tree(&t.d2, |tree| tree.edit(2, 2, vec![text("c")], 0));
    assert_eq!("<doc><p>acb</p></doc>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(
        vec![(1, 3, None), (1, 1, Some(vec![text("c")]))],
        edit_summaries(&ops1)
    );
    assert_eq!(
        vec![(2, 2, Some(vec![text("c")])), (3, 4, None), (1, 2, None)],
        edit_summaries(&ops2)
    );

    t.deactivate().await;
}

/// JS "TreeChange: Concurrent delete and insert when parent removed": an
/// insertion into a removed parent is not visible, so it reports nothing.
#[tokio::test]
async fn reports_nothing_for_an_insert_into_a_removed_parent() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-change-parent-removed",
        doc_p("ab"),
        "<doc><p>ab</p></doc>",
    )
    .await;
    let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

    edit_tree(&t.d1, |tree| tree.edit(0, 4, vec![], 0));
    assert_eq!("<doc></doc>", xml(&t.d1));
    edit_tree(&t.d2, |tree| tree.edit(2, 2, vec![text("c")], 0));
    assert_eq!("<doc><p>acb</p></doc>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(vec![(0, 4, None)], edit_summaries(&ops1));
    assert_eq!(
        vec![(2, 2, Some(vec![text("c")])), (0, 5, None)],
        edit_summaries(&ops2)
    );

    t.deactivate().await;
}

/// JS "TreeChange: Concurrent delete with contents and insert".
#[tokio::test]
async fn reports_a_replacement_and_a_concurrent_insert() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-change-replace-insert",
        doc_p("a"),
        "<doc><p>a</p></doc>",
    )
    .await;
    let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

    edit_tree(&t.d1, |tree| tree.edit(1, 2, vec![text("b")], 0));
    assert_eq!("<doc><p>b</p></doc>", xml(&t.d1));
    edit_tree(&t.d2, |tree| tree.edit(2, 2, vec![text("c")], 0));
    assert_eq!("<doc><p>ac</p></doc>", xml(&t.d2));

    t.sync_all().await;
    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(
        vec![(1, 2, Some(vec![text("b")])), (2, 2, Some(vec![text("c")]))],
        edit_summaries(&ops1)
    );
    assert_eq!(
        vec![(2, 2, Some(vec![text("c")])), (1, 2, Some(vec![text("b")]))],
        edit_summaries(&ops2)
    );

    t.deactivate().await;
}

fn tree_edit_op(from: usize, to: usize, path: Vec<usize>, value: TreeNodeInput) -> OperationInfo {
    OperationInfo::TreeEdit {
        path: "$.t".to_owned(),
        from,
        to,
        value: Some(vec![value]),
        split_level: None,
        from_path: path.clone(),
        to_path: path,
    }
}

fn tree_style_op(
    from: usize,
    to: usize,
    from_path: Vec<usize>,
    value: TreeStyleOpValue,
) -> OperationInfo {
    let mut to_path = from_path.clone();
    to_path.push(0);
    OperationInfo::TreeStyle {
        path: "$.t".to_owned(),
        from,
        to,
        from_path,
        to_path,
        value,
    }
}

/// JS "TreeChange: Concurrent insert and style" and "Concurrent insert and
/// removeStyle": a remote style reports where its element is now.
#[tokio::test]
async fn reports_styles_concurrent_with_an_insert() {
    let Some(addr) = rpc_addr() else { return };
    let set_key = TreeStyleOpValue {
        attributes: Some(attributes(json!({"key": "a"}))),
        attributes_to_remove: None,
    };
    let remove_key = TreeStyleOpValue {
        attributes: None,
        attributes_to_remove: Some(vec!["key".to_owned()]),
    };

    for (name, initial_p, remove, expected) in [
        (
            "tree-change-insert-style",
            element("p", vec![]),
            false,
            r#"<doc><p></p><p key="a"></p></doc>"#,
        ),
        (
            "tree-change-insert-remove-style",
            element_with("p", json!({"key": "a"}), vec![]),
            true,
            "<doc><p></p><p></p></doc>",
        ),
    ] {
        let t = TwoClients::attach(&addr, name).await;
        set_tree(&t.d1, element("doc", vec![initial_p]));
        t.c1.sync(None).await.unwrap();
        t.c2.sync(None).await.unwrap();
        assert_eq!(xml(&t.d1), xml(&t.d2));
        let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

        for _ in 0..2 {
            edit_tree(&t.d1, |tree| {
                if remove {
                    tree.remove_style(0, 1, ["key"])
                } else {
                    tree.style(0, 1, attributes(json!({"key": "a"})))
                }
            });
        }
        edit_tree(&t.d2, |tree| tree.edit(0, 0, vec![element("p", vec![])], 0));
        t.sync_all().await;
        assert_eq!(xml(&t.d1), xml(&t.d2));
        assert_eq!(expected, xml(&t.d1));

        let value = if remove {
            remove_key.clone()
        } else {
            set_key.clone()
        };
        let edit = tree_edit_op(0, 0, vec![0], element("p", vec![]));
        let style = tree_style_op(0, 1, vec![0], value.clone());
        let style2 = tree_style_op(2, 3, vec![1], value);
        assert_eq!(
            vec![style.clone(), style, edit.clone()],
            *ops1.lock().unwrap()
        );
        assert_eq!(vec![edit, style2.clone(), style2], *ops2.lock().unwrap());

        t.deactivate().await;
    }
}

/// JS "TreeChange: Concurrent delete and style". JS styles by path `[0]`,
/// which is the index range `0..1`.
#[tokio::test]
async fn reports_a_style_concurrent_with_a_deletion() {
    let Some(addr) = rpc_addr() else { return };
    let t_node = |id: &str| element_with("t", json!({"id": id, "value": "init"}), vec![]);
    let t = TwoClients::with_tree(
        &addr,
        "tree-change-delete-style",
        element("root", vec![t_node("1"), t_node("2")]),
        r#"<root><t id="1" value="init"></t><t id="2" value="init"></t></root>"#,
    )
    .await;
    let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

    edit_tree(&t.d1, |tree| {
        tree.style(0, 1, attributes(json!({"value": "changed"})))
    });
    edit_tree(&t.d2, |tree| tree.edit_by_path(&[0], &[1], vec![], 0));
    t.sync_all().await;
    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(r#"<root><t id="2" value="init"></t></root>"#, xml(&t.d1));

    let ops1 = ops1.lock().unwrap().clone();
    assert_eq!(2, ops1.len());
    assert!(matches!(
        &ops1[0],
        OperationInfo::TreeStyle { from: 0, to: 1, value, .. }
            if value.attributes == Some(attributes(json!({"value": "changed"})))
    ));
    assert!(matches!(
        &ops1[1],
        OperationInfo::TreeEdit {
            from: 0,
            to: 2,
            value: None,
            ..
        }
    ));
    assert_eq!(vec![(0, 2, None)], edit_summaries(&ops2));

    t.deactivate().await;
}

/// JS "TreeChange: Concurrent style and style": the later style wins, and
/// the replica that lost reports the winning value.
#[tokio::test]
async fn reports_concurrent_styles() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::with_tree(
        &addr,
        "tree-change-style-style",
        doc_p("hello"),
        "<doc><p>hello</p></doc>",
    )
    .await;
    let (ops1, ops2) = (collect_ops(&t.d1), collect_ops(&t.d2));

    edit_tree(&t.d1, |tree| {
        tree.style(0, 1, attributes(json!({"bold": "true"})))
    });
    edit_tree(&t.d2, |tree| {
        tree.style(0, 1, attributes(json!({"bold": "false"})))
    });
    t.sync_all().await;
    assert_eq!(xml(&t.d1), xml(&t.d2));
    assert_eq!(r#"<doc><p bold="false">hello</p></doc>"#, xml(&t.d1));

    let styles = |ops: &Mutex<Vec<OperationInfo>>| {
        ops.lock()
            .unwrap()
            .iter()
            .map(|op| match op {
                OperationInfo::TreeStyle {
                    from, to, value, ..
                } => (*from, *to, value.attributes.clone()),
                other => panic!("expected a tree style, got {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    let bold = |value: &str| Some(attributes(json!({ "bold": value })));
    assert_eq!(
        vec![(0, 1, bold("true")), (0, 1, bold("false"))],
        styles(&ops1)
    );
    assert_eq!(vec![(0, 1, bold("false"))], styles(&ops2));

    t.deactivate().await;
}

// The generated matrices of JS `tree_concurrency_test.ts`. JS runs each
// combination as its own test; here one test per matrix runs them in turn on
// one pair of clients and reports every combination that does not converge.

/// Two ranges, one per client, as `(from, mid, to)` (JS `makeTwoRanges`).
struct TwoRanges {
    ranges: [(usize, usize, usize); 2],
    desc: &'static str,
}

const fn ranges(
    first: (usize, usize, usize),
    second: (usize, usize, usize),
    desc: &'static str,
) -> TwoRanges {
    TwoRanges {
        ranges: [first, second],
        desc,
    }
}

/// The part of a range an operation acts on (JS `RangeSelector`).
#[derive(Clone, Copy)]
enum Selector {
    Front,
    Middle,
    Back,
    All,
    OneQuarter,
    ThreeQuarter,
}

impl Selector {
    fn range(self, (from, mid, to): (usize, usize, usize)) -> (usize, usize) {
        let one_quarter = (from + mid + 1) >> 1;
        let three_quarter = (mid + to) >> 1;
        match self {
            Self::Front => (from, from),
            Self::Middle => (mid, mid),
            Self::Back => (to, to),
            Self::All => (from, to),
            Self::OneQuarter => (one_quarter, one_quarter),
            Self::ThreeQuarter => (three_quarter, three_quarter),
        }
    }
}

#[derive(Clone)]
enum TreeOp {
    Edit {
        selector: Selector,
        content: Option<TreeNodeInput>,
        desc: &'static str,
    },
    /// Deletes the element boundaries inside the range, merging the elements
    /// around them (JS `EditOpCode.MergeUpdate`).
    Merge { desc: &'static str },
    /// Splits `level` elements at the selected position (JS
    /// `EditOpCode.SplitUpdate`).
    Split {
        selector: Selector,
        level: usize,
        desc: &'static str,
    },
    Style {
        key: &'static str,
        /// `None` removes the attribute.
        value: Option<&'static str>,
        desc: &'static str,
    },
}

impl TreeOp {
    fn edit(selector: Selector, content: Option<TreeNodeInput>, desc: &'static str) -> Self {
        Self::Edit {
            selector,
            content,
            desc,
        }
    }

    fn split(selector: Selector, level: usize, desc: &'static str) -> Self {
        Self::Split {
            selector,
            level,
            desc,
        }
    }

    fn style(key: &'static str, value: Option<&'static str>, desc: &'static str) -> Self {
        Self::Style { key, value, desc }
    }

    fn desc(&self) -> &'static str {
        match self {
            Self::Edit { desc, .. }
            | Self::Merge { desc }
            | Self::Split { desc, .. }
            | Self::Style { desc, .. } => desc,
        }
    }

    fn run(&self, doc: &SharedDocument, range: (usize, usize, usize)) {
        match self {
            Self::Edit {
                selector, content, ..
            } => {
                let (from, to) = selector.range(range);
                let contents = content.iter().cloned().collect();
                edit_tree(doc, |tree| tree.edit(from, to, contents, 0));
            }
            Self::Merge { .. } => {
                let (from, to) = Selector::All.range(range);
                if let Some((from, to)) = merge_range(&xml(doc), from, to) {
                    edit_tree(doc, |tree| tree.edit(from, to, vec![], 0));
                }
            }
            Self::Split {
                selector, level, ..
            } => {
                let (from, to) = selector.range(range);
                assert_eq!(from, to, "a split is at one position");
                edit_tree(doc, |tree| tree.edit(from, to, vec![], *level));
            }
            Self::Style { key, value, .. } => {
                let (from, _, to) = range;
                edit_tree(doc, |tree| match value {
                    Some(value) => tree.style(from, to, attributes(json!({ *key: value }))),
                    None => tree.remove_style(from, to, [*key]),
                });
            }
        }
    }
}

/// Returns the range from before the first end tag to the last start tag in
/// `from..to` of `xml`, the boundaries a merge deletes (JS `getMergeRange`
/// with `parseSimpleXML`).
fn merge_range(xml: &str, from: usize, to: usize) -> Option<(usize, usize)> {
    let mut tokens = Vec::new();
    let mut chars = xml.chars().peekable();
    while let Some(ch) = chars.next() {
        let mut token = ch.to_string();
        if ch == '<' {
            for next in chars.by_ref() {
                token.push(next);
                if next == '>' {
                    break;
                }
            }
        }
        tokens.push(token);
    }

    let (mut start, mut end) = (None, None);
    for (index, token) in tokens.iter().enumerate().take(to + 1).skip(from + 1) {
        if start.is_none() && token.starts_with("</") {
            start = Some(index - 1);
        }
        if token.starts_with('<') && !token.starts_with("</") {
            end = Some(index);
        }
    }
    match (start, end) {
        (Some(start), Some(end)) if start < end => Some((start, end)),
        _ => None,
    }
}

/// Runs every combination of the matrix and fails with the ones that do not
/// converge (JS `runTestConcurrency`).
async fn run_matrix(
    name: &str,
    root: TreeNodeInput,
    initial_xml: &str,
    ranges: &[TwoRanges],
    ops1: &[TreeOp],
    ops2: &[TreeOp],
) {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    let mut diverged = Vec::new();
    for (case, (two_ranges, (op1, op2))) in ranges
        .iter()
        .flat_map(|r| {
            ops1.iter()
                .flat_map(move |a| ops2.iter().map(move |b| (r, (a, b))))
        })
        .enumerate()
    {
        let desc = format!("{name}-{}({},{})", two_ranges.desc, op1.desc(), op2.desc());
        let (d1, d2) = attach_pair(&c1, &c2, &unique_key(&format!("{name}-{case}"))).await;
        set_tree(&d1, root.clone());
        c1.sync(Some(&d1)).await.unwrap();
        c2.sync(Some(&d2)).await.unwrap();
        assert_eq!(initial_xml, xml(&d1), "{desc}");
        assert_eq!(initial_xml, xml(&d2), "{desc}");

        op1.run(&d1, two_ranges.ranges[0]);
        op2.run(&d2, two_ranges.ranges[1]);
        let mut synced = Ok(());
        for _ in 0..2 {
            for (client, doc) in [(&c1, &d1), (&c2, &d2)] {
                if synced.is_ok() {
                    synced = client.sync(Some(doc)).await;
                }
            }
        }

        let (after1, after2) = (xml(&d1), xml(&d2));
        match synced {
            Err(err) => diverged.push(format!("{desc}: sync failed: {err:?}")),
            Ok(()) if after1 != after2 => {
                diverged.push(format!("{desc}: d1 {after1} != d2 {after2}"));
            }
            Ok(()) => {}
        }
        // A failed sync can leave a document unable to detach cleanly.
        let _ = c1.detach(&d1, DetachOptions).await;
        let _ = c2.detach(&d2, DetachOptions).await;
    }

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
    assert!(diverged.is_empty(), "diverged:\n{}", diverged.join("\n"));
}

/// JS "concurrently-edit-edit-test".
#[tokio::test]
async fn concurrently_edit_edit_matrix_converges() {
    let root = element(
        "r",
        vec![
            element("p", vec![text("abc")]),
            element("p", vec![text("def")]),
            element("p", vec![text("ghi")]),
        ],
    );
    let two_ranges = [
        ranges((0, 5, 10), (5, 10, 15), "intersect-element"),
        ranges((1, 2, 3), (2, 3, 4), "intersect-text"),
        ranges((0, 5, 15), (5, 5, 10), "contain-element"),
        ranges((1, 2, 4), (2, 2, 3), "contain-text"),
        ranges((0, 5, 15), (6, 7, 9), "contain-mixed-type"),
        ranges((0, 5, 5), (5, 5, 10), "side-by-side-element"),
        ranges((1, 1, 2), (2, 3, 4), "side-by-side-text"),
        ranges((0, 5, 10), (0, 5, 10), "equal-element"),
        ranges((1, 2, 4), (1, 2, 4), "equal-text"),
    ];
    let edit_ops = |text_value: &'static str, element_type: &'static str| {
        let text_node = Some(text(text_value));
        let element_node = Some(element(element_type, vec![]));
        vec![
            TreeOp::edit(Selector::Front, text_node.clone(), "insertTextFront"),
            TreeOp::edit(Selector::Middle, text_node.clone(), "insertTextMiddle"),
            TreeOp::edit(Selector::Back, text_node.clone(), "insertTextBack"),
            TreeOp::edit(Selector::All, text_node, "replaceText"),
            TreeOp::edit(Selector::Front, element_node.clone(), "insertElementFront"),
            TreeOp::edit(
                Selector::Middle,
                element_node.clone(),
                "insertElementMiddle",
            ),
            TreeOp::edit(Selector::Back, element_node.clone(), "insertElementBack"),
            TreeOp::edit(Selector::All, element_node, "replaceElement"),
            TreeOp::edit(Selector::All, None, "delete"),
            TreeOp::Merge { desc: "merge" },
        ]
    };
    run_matrix(
        "concurrently-edit-edit-test",
        root,
        "<r><p>abc</p><p>def</p><p>ghi</p></r>",
        &two_ranges,
        &edit_ops("A", "b"),
        &edit_ops("B", "i"),
    )
    .await;
}

/// JS "concurrently-style-style-test".
#[tokio::test]
async fn concurrently_style_style_matrix_converges() {
    let root = element(
        "r",
        vec![
            element("p", vec![text("a")]),
            element("p", vec![text("b")]),
            element("p", vec![text("c")]),
        ],
    );
    let two_ranges = [
        ranges((3, 0, 6), (3, 0, 6), "equal"),
        ranges((0, 0, 9), (3, 0, 6), "contain"),
        ranges((0, 0, 6), (3, 0, 9), "intersect"),
        ranges((0, 0, 3), (3, 0, 6), "side-by-side"),
    ];
    let style_ops = [
        TreeOp::style("bold", None, "remove-bold"),
        TreeOp::style("bold", Some("aa"), "set-bold-aa"),
        TreeOp::style("bold", Some("bb"), "set-bold-bb"),
        TreeOp::style("italic", None, "remove-italic"),
        TreeOp::style("italic", Some("aa"), "set-italic-aa"),
        TreeOp::style("italic", Some("bb"), "set-italic-bb"),
    ];
    run_matrix(
        "concurrently-style-style-test",
        root,
        "<r><p>a</p><p>b</p><p>c</p></r>",
        &two_ranges,
        &style_ops,
        &style_ops,
    )
    .await;
}

/// JS "concurrently-edit-style-test".
#[tokio::test]
async fn concurrently_edit_style_matrix_converges() {
    let red_p = |value: &str| element_with("p", json!({"color": "red"}), vec![text(value)]);
    let root = element("r", vec![red_p("a"), red_p("b"), red_p("c")]);
    let two_ranges = [
        ranges((3, 3, 6), (3, 0, 6), "equal"),
        ranges((0, 3, 9), (0, 3, 9), "equal multiple"),
        ranges((0, 3, 9), (3, 0, 6), "A contains B"),
        ranges((3, 3, 6), (0, 0, 9), "B contains A"),
        ranges((0, 3, 6), (3, 0, 9), "intersect"),
        ranges((0, 3, 3), (3, 0, 6), "A -> B"),
        ranges((3, 3, 6), (0, 0, 3), "B -> A"),
    ];
    let content = Some(element_with(
        "p",
        json!({"italic": "true", "color": "blue"}),
        vec![text("d")],
    ));
    let edit_ops = [
        TreeOp::edit(Selector::Front, content.clone(), "insertFront"),
        TreeOp::edit(Selector::Middle, content.clone(), "insertMiddle"),
        TreeOp::edit(Selector::Back, content.clone(), "insertBack"),
        TreeOp::edit(Selector::All, None, "delete"),
        TreeOp::edit(Selector::All, content, "replace"),
        TreeOp::Merge { desc: "merge" },
    ];
    let style_ops = [
        TreeOp::style("color", None, "remove-color"),
        TreeOp::style("bold", Some("aa"), "set-bold-aa"),
    ];
    run_matrix(
        "concurrently-edit-style-test",
        root,
        r#"<r><p color="red">a</p><p color="red">b</p><p color="red">c</p></r>"#,
        &two_ranges,
        &edit_ops,
        &style_ops,
    )
    .await;
}

/// `<r><p><p><p><p>abcd</p><p>efgh</p></p><p>ijkl</p></p></p></r>`, the tree
/// of the JS split-split matrices.
fn nested_paragraphs() -> TreeNodeInput {
    let p = |children| element("p", children);
    element(
        "r",
        vec![p(vec![p(vec![
            p(vec![p(vec![text("abcd")]), p(vec![text("efgh")])]),
            p(vec![text("ijkl")]),
        ])])],
    )
}

fn split_split_ranges() -> [TwoRanges; 5] {
    [
        ranges((3, 6, 9), (3, 6, 9), "equal-single"),
        ranges((3, 9, 15), (3, 9, 15), "equal-multiple"),
        ranges((3, 9, 15), (9, 12, 15), "A contains B same level"),
        ranges((2, 16, 22), (9, 12, 15), "A contains B multiple level"),
        ranges((3, 6, 9), (9, 12, 15), "B is next to A"),
    ]
}

fn split_ops(level: usize) -> [TreeOp; 4] {
    let descs = match level {
        1 => [
            "split-front-1",
            "split-one-quarter-1",
            "split-three-quarter-1",
            "split-back-1",
        ],
        _ => [
            "split-front-2",
            "split-one-quarter-2",
            "split-three-quarter-2",
            "split-back-2",
        ],
    };
    [
        TreeOp::split(Selector::Front, level, descs[0]),
        TreeOp::split(Selector::OneQuarter, level, descs[1]),
        TreeOp::split(Selector::ThreeQuarter, level, descs[2]),
        TreeOp::split(Selector::Back, level, descs[3]),
    ]
}

/// JS "concurrently-split-split-test (splitLevel=1)".
#[tokio::test]
async fn concurrently_split_split_matrix_converges() {
    let ops = split_ops(1);
    run_matrix(
        "concurrently-split-split-test",
        nested_paragraphs(),
        "<r><p><p><p><p>abcd</p><p>efgh</p></p><p>ijkl</p></p></p></r>",
        &split_split_ranges(),
        &ops,
        &ops,
    )
    .await;
}

/// JS "concurrently-split-split-test (splitLevel>=2)".
#[tokio::test]
async fn concurrently_split_split_multi_level_matrix_converges() {
    let ops = [split_ops(1), split_ops(2)].concat();
    run_matrix(
        "concurrently-split-split-multi-level-test",
        nested_paragraphs(),
        "<r><p><p><p><p>abcd</p><p>efgh</p></p><p>ijkl</p></p></p></r>",
        &split_split_ranges(),
        &ops,
        &ops,
    )
    .await;
}

/// `<r><p><p><p italic="a">abcd</p><p italic="a">efgh</p></p><p
/// italic="a">ijkl</p></p></r>`, the tree of the JS split-edit matrices.
fn italic_paragraphs() -> TreeNodeInput {
    let italic = |value| element_with("p", json!({"italic": "a"}), vec![text(value)]);
    element(
        "r",
        vec![element(
            "p",
            vec![
                element("p", vec![italic("abcd"), italic("efgh")]),
                italic("ijkl"),
            ],
        )],
    )
}

async fn run_split_edit_matrix(name: &str, split_ops: &[TreeOp]) {
    let two_ranges = [
        ranges((2, 5, 8), (2, 5, 8), "equal"),
        ranges((2, 5, 8), (4, 5, 6), "A contains B"),
        ranges((2, 5, 8), (2, 8, 14), "B contains A"),
        ranges((2, 5, 8), (3, 4, 5), "left node(text)"),
        ranges((2, 5, 8), (5, 6, 7), "right node(text)"),
        ranges((2, 8, 14), (2, 5, 8), "left node(element)"),
        ranges((2, 8, 14), (8, 11, 14), "right node(element)"),
        ranges((2, 5, 8), (8, 11, 14), "A -> B"),
        ranges((8, 11, 14), (2, 5, 8), "B -> A"),
    ];
    let content = Some(element("i", vec![]));
    let edit_ops = [
        TreeOp::edit(Selector::Front, content.clone(), "insertFront"),
        TreeOp::edit(Selector::Middle, content.clone(), "insertMiddle"),
        TreeOp::edit(Selector::Back, content.clone(), "insertBack"),
        TreeOp::edit(Selector::All, content, "replace"),
        TreeOp::edit(Selector::All, None, "delete"),
        TreeOp::Merge { desc: "merge" },
        TreeOp::style("bold", Some("aa"), "style"),
        TreeOp::style("italic", None, "remove-style"),
    ];
    run_matrix(
        name,
        italic_paragraphs(),
        r#"<r><p><p><p italic="a">abcd</p><p italic="a">efgh</p></p><p italic="a">ijkl</p></p></r>"#,
        &two_ranges,
        split_ops,
        &edit_ops,
    )
    .await;
}

/// JS "concurrently-split-edit-test (splitLevel=1)".
#[tokio::test]
async fn concurrently_split_edit_matrix_converges() {
    run_split_edit_matrix(
        "concurrently-split-edit-test",
        &[TreeOp::split(Selector::Middle, 1, "split-1")],
    )
    .await;
}

/// JS "concurrently-split-edit-test (splitLevel>=2)".
#[tokio::test]
async fn concurrently_split_edit_multi_level_matrix_converges() {
    run_split_edit_matrix(
        "concurrently-split-edit-multi-level-test",
        &[
            TreeOp::split(Selector::Middle, 1, "split-1"),
            TreeOp::split(Selector::Middle, 2, "split-2"),
        ],
    )
    .await;
}
