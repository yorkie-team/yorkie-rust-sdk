//! Document-level tests of Tree undo and redo.
//!
//! The first groups port the single-client cases of JS
//! `test/integration/history_tree_test.ts` and
//! `test/integration/history_tree_split_test.ts` (`8629993e`), run on a
//! `Document` without a server; the test names follow the JS `describe` and
//! `it` names, and each JS loop becomes a table that names its row on
//! failure. The rest port the Tree undo cases of the JS unit tests named on
//! each test.
//!
//! Where JS hands changes to another replica in memory, so do these; where
//! JS passes them through protobuf, these pass them through the `wire`
//! projection `yorkie-protocol` encodes from.

use super::adv_sync::through_wire;
use super::Document;
use crate::change::{Change, ChangePack, Checkpoint};
use crate::crdt::element::CrdtElement;
use crate::crdt::tree::{CrdtTree, TreeRestoreSpan};
use crate::history::HistoryOperation;
use crate::operation::{force_copy_path, Operation, TreeEditOperation};
use crate::{JsonTree, JsonValue, Result, TreeNodeInput, VersionVector};
use serde_json::Value;
use std::collections::BTreeMap;

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

/// `<p>` holding the text `value`.
fn p(value: &str) -> TreeNodeInput {
    element("p", vec![text(value)])
}

fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Value::from(*value)))
        .collect()
}

/// A document holding `root` as the Tree under `t`, set in one change.
fn doc_with(root: TreeNodeInput) -> Result<Document> {
    let mut doc = Document::new("test-doc");
    doc.update(|r| r.set_tree("t", root).map(|_| ()))?;
    Ok(doc)
}

/// JS `xmlOf`.
fn xml(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().expect("a document tree"),
        other => panic!("expected a tree at t, got {other:?}"),
    }
}

fn edit_tree(doc: &mut Document, f: impl FnOnce(&mut JsonTree) -> Result<()>) -> Result<()> {
    doc.update(|root| f(root.get_tree_mut("t")?))
}

fn undo(doc: &mut Document) -> Result<()> {
    doc.history_mut().undo()
}

fn redo(doc: &mut Document) -> Result<()> {
    doc.history_mut().redo()
}

/// The CRDT tree under `t`.
fn crdt_tree(doc: &Document) -> &CrdtTree {
    match doc.crdt_root.object().get("t") {
        Some(CrdtElement::Tree(tree)) => tree,
        other => panic!("expected a tree at t, got {other:?}"),
    }
}

/// JS `duplicatedIDs`: the IDs naming more than one node, live or removed.
fn duplicated_ids(doc: &Document) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for node in crdt_tree(doc).nodes() {
        *counts.entry(node.id_string()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(id, _)| id)
        .collect()
}

/// JS `liveIDs`: the IDs of the live nodes, in document order.
fn live_ids(doc: &Document) -> Vec<String> {
    crdt_tree(doc)
        .nodes()
        .filter(|node| !node.is_removed())
        .map(|node| node.id_string())
        .collect()
}

/// Runs `edit`, then checks that an undo goes back to the tree before it and
/// a redo forward to the tree after it.
fn assert_undo_redo(
    doc: &mut Document,
    name: &str,
    edit: impl FnOnce(&mut JsonTree) -> Result<()>,
) -> Result<()> {
    let before = xml(doc);
    edit_tree(doc, edit)?;
    let after = xml(doc);

    undo(doc)?;
    assert_eq!(before, xml(doc), "undo {name}");
    redo(doc)?;
    assert_eq!(after, xml(doc), "redo {name}");
    Ok(())
}

/// An edit of the tree, run as one change.
type TreeChange<'a> = &'a dyn Fn(&mut JsonTree) -> Result<()>;

/// Runs `ops` one change each, then undoes them all and redoes them all,
/// checking every state on the way back and forth.
fn assert_chain(doc: &mut Document, name: &str, ops: &[TreeChange<'_>]) -> Result<()> {
    let mut states = vec![xml(doc)];
    for op in ops {
        edit_tree(doc, op)?;
        states.push(xml(doc));
    }
    for i in (1..states.len()).rev() {
        undo(doc)?;
        assert_eq!(states[i - 1], xml(doc), "{name}: undo to S{}", i - 1);
    }
    for (i, state) in states.iter().enumerate().skip(1) {
        redo(doc)?;
        assert_eq!(*state, xml(doc), "{name}: redo to S{i}");
    }
    Ok(())
}

// --- history_tree_test.ts ---

/// JS `initTree`: `<doc><p>The fox jumped.</p></doc>`.
fn fox_doc() -> Result<Document> {
    doc_with(element("doc", vec![p("The fox jumped.")]))
}

type TreeOp = (&'static str, fn(&mut JsonTree) -> Result<()>);

/// JS `applyTreeOp1`, one row per `TreeOp`.
const TREE_OPS: [TreeOp; 6] = [
    ("insert-text", |t| t.edit(16, 16, vec![text("X")], 0)),
    ("delete-text", |t| t.edit(6, 7, Vec::new(), 0)),
    ("insert-element", |t| t.edit(17, 17, vec![p("New")], 0)),
    ("delete-element", |t| t.edit(0, 17, Vec::new(), 0)),
    ("replace-text", |t| t.edit(5, 8, vec![text("cat")], 0)),
    ("replace-element", |t| t.edit(0, 17, vec![p("Replaced")], 0)),
];

/// JS `history_tree_test.ts` "Tree History - single client basic", "should
/// undo/redo ${op}".
#[test]
fn single_client_undoes_and_redoes_each_op() -> Result<()> {
    for (name, op) in TREE_OPS {
        let mut doc = fox_doc()?;
        assert_undo_redo(&mut doc, name, op)?;
    }
    Ok(())
}

/// JS "should handle undo-redo round trip multiple times".
#[test]
fn single_client_round_trips_undo_and_redo_many_times() -> Result<()> {
    let mut doc = fox_doc()?;
    let initial = xml(&doc);
    edit_tree(&mut doc, |t| t.edit(1, 1, vec![text("Hello ")], 0))?;
    let modified = xml(&doc);

    for round in 0..3 {
        undo(&mut doc)?;
        assert_eq!(initial, xml(&doc), "round {round} undo");
        redo(&mut doc)?;
        assert_eq!(modified, xml(&doc), "round {round} redo");
    }
    Ok(())
}

/// JS "should clear redo stack when new edit is made after undo".
#[test]
fn single_client_clears_the_redo_stack_on_a_new_edit() -> Result<()> {
    let mut doc = fox_doc()?;
    edit_tree(&mut doc, |t| t.edit(16, 16, vec![text("X")], 0))?;

    undo(&mut doc)?;
    assert!(doc.history().can_redo());

    edit_tree(&mut doc, |t| t.edit(1, 1, vec![text("Z")], 0))?;
    assert!(!doc.history().can_redo());
    Ok(())
}

/// JS "Tree History - single client chained ops", every `op1-op2-op3` of
/// insert-text, delete-text and insert-element.
#[test]
fn single_client_undoes_and_redoes_chained_ops() -> Result<()> {
    let chain_ops: [TreeOp; 3] = [
        ("insert-text", |t| {
            t.edit_by_path(&[0, 1], &[0, 1], vec![text("X")], 0)
        }),
        ("delete-text", |t| t.edit(1, 2, Vec::new(), 0)),
        ("insert-element", |t| {
            t.edit_by_path(&[1], &[1], vec![p("N")], 0)
        }),
    ];
    for (name1, op1) in chain_ops {
        for (name2, op2) in chain_ops {
            for (name3, op3) in chain_ops {
                let mut doc = doc_with(element("doc", vec![p("ABCD")]))?;
                assert_chain(
                    &mut doc,
                    &format!("{name1}-{name2}-{name3}"),
                    &[&op1, &op2, &op3],
                )?;
            }
        }
    }
    Ok(())
}

/// JS "Tree History - single client edge cases": "should handle edit at
/// start position", "... middle position" and "... end position".
#[test]
fn single_client_undoes_edits_at_the_start_middle_and_end() -> Result<()> {
    let cases: [TreeOp; 3] = [
        ("start", |t| t.edit(1, 4, vec![text("A")], 0)),
        ("middle", |t| t.edit(5, 8, vec![text("cat")], 0)),
        ("end", |t| t.edit(16, 16, vec![text("!")], 0)),
    ];
    for (name, op) in cases {
        let mut doc = fox_doc()?;
        assert_undo_redo(&mut doc, name, op)?;
    }
    Ok(())
}

/// JS "should handle full tree deletion + undo".
#[test]
fn single_client_undoes_a_full_tree_deletion() -> Result<()> {
    let mut doc = fox_doc()?;
    let before = xml(&doc);
    edit_tree(&mut doc, |t| t.edit(0, 17, Vec::new(), 0))?;
    assert_eq!("<doc></doc>", xml(&doc));

    undo(&mut doc)?;
    assert_eq!(before, xml(&doc));
    redo(&mut doc)?;
    assert_eq!("<doc></doc>", xml(&doc));
    Ok(())
}

/// JS "should handle empty undo/redo stacks".
#[test]
fn single_client_empties_the_undo_stack() -> Result<()> {
    let mut doc = fox_doc()?;
    assert!(doc.history().can_undo());
    undo(&mut doc)?;
    assert!(!doc.history().can_undo());
    Ok(())
}

/// JS "should handle rapid consecutive edits".
#[test]
fn single_client_undoes_and_redoes_rapid_consecutive_edits() -> Result<()> {
    let mut doc = doc_with(element("doc", vec![element("p", Vec::new())]))?;
    let mut states = vec![xml(&doc)];
    for i in 0..10 {
        edit_tree(&mut doc, |t| t.edit(1, 1, vec![text(&i.to_string())], 0))?;
        states.push(xml(&doc));
    }

    for i in (0..10).rev() {
        undo(&mut doc)?;
        assert_eq!(states[i], xml(&doc), "undo to state {i}");
    }
    for (i, state) in states.iter().enumerate().skip(1) {
        redo(&mut doc)?;
        assert_eq!(*state, xml(&doc), "redo to state {i}");
    }
    Ok(())
}

/// JS "Tree History - tree style undo/redo", `makeTree`.
fn styled_doc() -> Result<Document> {
    doc_with(element(
        "doc",
        vec![
            TreeNodeInput::element_with_attributes(
                "p",
                attrs(&[("bold", "true")]),
                vec![text("AB")],
            ),
            p("CD"),
        ],
    ))
}

/// JS `applyStyleOp`, one row per `StyleOp`.
const STYLE_OPS: [TreeOp; 4] = [
    ("set-bold", |t| t.style(0, 1, attrs(&[("bold", "true")]))),
    ("set-italic", |t| {
        t.style(0, 1, attrs(&[("italic", "true")]))
    }),
    ("set-color", |t| t.style(3, 4, attrs(&[("color", "red")]))),
    ("remove-bold", |t| t.remove_style(0, 1, ["bold"])),
];

/// JS "Tree History - tree style undo/redo", "should undo/redo: ${op}".
#[test]
fn single_client_undoes_and_redoes_each_style() -> Result<()> {
    for (name, op) in STYLE_OPS {
        let mut doc = styled_doc()?;
        assert_undo_redo(&mut doc, name, op)?;
    }
    Ok(())
}

/// JS "should undo chain: ${op1} → ${op2}", every pair of style ops but
/// `remove-bold → remove-bold`, whose second op has nothing to remove.
#[test]
fn single_client_undoes_and_redoes_chained_styles() -> Result<()> {
    for (name1, op1) in STYLE_OPS {
        for (name2, op2) in STYLE_OPS {
            if name1 == "remove-bold" && name2 == "remove-bold" {
                continue;
            }
            let mut doc = styled_doc()?;
            assert_chain(&mut doc, &format!("{name1} → {name2}"), &[&op1, &op2])?;
        }
    }
    Ok(())
}

/// JS "should undo style after edit" and "should undo edit after style".
#[test]
fn single_client_undoes_styles_and_edits_in_either_order() -> Result<()> {
    let insert = |t: &mut JsonTree| t.edit(1, 1, vec![text("X")], 0);
    let italic = STYLE_OPS[1].1;

    let mut doc = styled_doc()?;
    assert_chain(&mut doc, "style after edit", &[&insert, &italic])?;
    let mut doc = styled_doc()?;
    assert_chain(&mut doc, "edit after style", &[&italic, &insert])
}

// --- history_tree_split_test.ts ---

/// `<doc><p>ABCD</p></doc>`.
fn abcd_doc() -> Result<Document> {
    doc_with(element("doc", vec![p("ABCD")]))
}

/// `<doc><div><p>ABCD</p></div></doc>`.
fn nested_abcd_doc() -> Result<Document> {
    doc_with(element("doc", vec![element("div", vec![p("ABCD")])]))
}

/// JS `history_tree_split_test.ts` "Tree History - single client
/// split/merge", "should undo editByPath split" and "should redo editByPath
/// split".
#[test]
fn single_client_undoes_and_redoes_a_split_by_path() -> Result<()> {
    let mut doc = abcd_doc()?;
    assert_eq!("<doc><p>ABCD</p></doc>", xml(&doc));
    edit_tree(&mut doc, |t| {
        t.edit_by_path(&[0, 2], &[0, 2], Vec::new(), 1)
    })?;
    assert_eq!("<doc><p>AB</p><p>CD</p></doc>", xml(&doc));

    undo(&mut doc)?;
    assert_eq!("<doc><p>ABCD</p></doc>", xml(&doc));
    redo(&mut doc)?;
    assert_eq!("<doc><p>AB</p><p>CD</p></doc>", xml(&doc));
    Ok(())
}

/// JS "should undo a split deeper than the tree": a split level the tree
/// has no room for opens fewer boundaries, and the undo must merge only the
/// one it opened.
#[test]
fn single_client_undoes_a_split_deeper_than_the_tree() -> Result<()> {
    let mut doc = doc_with(element("doc", vec![p("ABCD"), p("0123456789")]))?;
    let before = xml(&doc);
    assert_eq!("<doc><p>ABCD</p><p>0123456789</p></doc>", before);

    edit_tree(&mut doc, |t| {
        t.edit_by_path(&[0, 2], &[0, 2], Vec::new(), 3)
    })?;
    let split = "<doc><p>AB</p><p>CD</p><p>0123456789</p></doc>";
    assert_eq!(split, xml(&doc));

    undo(&mut doc)?;
    assert_eq!(
        before,
        xml(&doc),
        "the undo merges only what the split opened"
    );
    redo(&mut doc)?;
    assert_eq!(split, xml(&doc));
    Ok(())
}

/// JS "should undo editByPath merge" and "should redo editByPath merge".
#[test]
fn single_client_undoes_and_redoes_a_merge_by_path() -> Result<()> {
    let mut doc = doc_with(element("doc", vec![p("AB"), p("CD")]))?;
    assert_eq!("<doc><p>AB</p><p>CD</p></doc>", xml(&doc));
    edit_tree(&mut doc, |t| {
        t.edit_by_path(&[0, 2], &[1, 0], Vec::new(), 0)
    })?;
    assert_eq!("<doc><p>ABCD</p></doc>", xml(&doc));

    undo(&mut doc)?;
    assert_eq!("<doc><p>AB</p><p>CD</p></doc>", xml(&doc));
    redo(&mut doc)?;
    assert_eq!("<doc><p>ABCD</p></doc>", xml(&doc));
    Ok(())
}

/// One row of a split table: the position name, the split index and the
/// tree after the split.
type SplitCase = (&'static str, usize, &'static str);

/// Runs the three JS split-table tests for one row: "should undo split at
/// ${pos}", "should undo-redo split at ${pos}" and "should undo-redo-undo
/// split at ${pos}".
fn assert_split_row(
    new_doc: fn() -> Result<Document>,
    level: usize,
    (pos, at, after): SplitCase,
) -> Result<()> {
    let mut doc = new_doc()?;
    let before = xml(&doc);
    edit_tree(&mut doc, |t| t.edit(at, at, Vec::new(), level))?;
    assert_eq!(after, xml(&doc), "split at {pos}");

    undo(&mut doc)?;
    assert_eq!(before, xml(&doc), "undo split at {pos}");
    redo(&mut doc)?;
    assert_eq!(after, xml(&doc), "redo split at {pos}");
    undo(&mut doc)?;
    assert_eq!(before, xml(&doc), "undo-redo-undo split at {pos}");
    Ok(())
}

/// JS "Tree History - single client split L1 undo/redo".
#[test]
fn single_client_undoes_and_redoes_level_one_splits() -> Result<()> {
    for case in [
        ("front", 1, "<doc><p></p><p>ABCD</p></doc>"),
        ("middle", 3, "<doc><p>AB</p><p>CD</p></doc>"),
        ("back", 5, "<doc><p>ABCD</p><p></p></doc>"),
    ] {
        assert_split_row(abcd_doc, 1, case)?;
    }
    Ok(())
}

/// JS "Tree History - single client split L1 chained ops", every pair of
/// split, insert-text and delete-text.
#[test]
fn single_client_undoes_level_one_splits_chained_with_edits() -> Result<()> {
    let chain_ops: [TreeOp; 3] = [
        ("split", |t| t.edit_by_path(&[0, 2], &[0, 2], Vec::new(), 1)),
        ("insert-text", |t| {
            t.edit_by_path(&[0, 0], &[0, 0], vec![text("X")], 0)
        }),
        ("delete-text", |t| t.edit(1, 2, Vec::new(), 0)),
    ];
    for (name1, op1) in chain_ops {
        for (name2, op2) in chain_ops {
            let mut doc = abcd_doc()?;
            assert_chain(&mut doc, &format!("{name1} → {name2}"), &[&op1, &op2])?;
        }
    }
    Ok(())
}

/// JS "Tree History - split L1 edge cases", "should undo front split with
/// empty paragraph" and "should undo back split with empty paragraph".
#[test]
fn single_client_undoes_splits_that_leave_an_empty_paragraph() -> Result<()> {
    for (pos, at, after) in [
        ("front", 1, "<doc><p></p><p>AB</p></doc>"),
        ("back", 3, "<doc><p>AB</p><p></p></doc>"),
    ] {
        let mut doc = doc_with(element("doc", vec![p("AB")]))?;
        let before = xml(&doc);
        edit_tree(&mut doc, |t| t.edit(at, at, Vec::new(), 1))?;
        assert_eq!(after, xml(&doc), "{pos} split");

        undo(&mut doc)?;
        assert_eq!(before, xml(&doc), "undo {pos} split");
        redo(&mut doc)?;
        assert_eq!(after, xml(&doc), "redo {pos} split");
    }
    Ok(())
}

/// JS "should clear redo stack when new edit is made after split undo".
#[test]
fn single_client_clears_the_redo_stack_after_a_split_undo() -> Result<()> {
    let mut doc = abcd_doc()?;
    edit_tree(&mut doc, |t| t.edit(3, 3, Vec::new(), 1))?;

    undo(&mut doc)?;
    assert!(doc.history().can_redo());

    edit_tree(&mut doc, |t| t.edit(1, 1, vec![text("Z")], 0))?;
    assert!(!doc.history().can_redo());
    Ok(())
}

/// JS "Tree History - single client split L2 undo/redo".
#[test]
fn single_client_undoes_and_redoes_level_two_splits() -> Result<()> {
    // <doc> <div> <p> A B C D </p> </div> </doc>
    //   0     1    2  3 4 5 6   7     8
    for case in [
        (
            "front",
            2,
            "<doc><div><p></p></div><div><p>ABCD</p></div></doc>",
        ),
        (
            "middle",
            4,
            "<doc><div><p>AB</p></div><div><p>CD</p></div></doc>",
        ),
        (
            "back",
            6,
            "<doc><div><p>ABCD</p></div><div><p></p></div></doc>",
        ),
    ] {
        assert_split_row(nested_abcd_doc, 2, case)?;
    }
    Ok(())
}

const LEVEL_TWO_CHAIN_OPS: [TreeOp; 3] = [
    ("split-l2", |t| {
        t.edit_by_path(&[0, 0, 2], &[0, 0, 2], Vec::new(), 2)
    }),
    ("insert-text", |t| {
        t.edit_by_path(&[0, 0, 0], &[0, 0, 0], vec![text("X")], 0)
    }),
    ("delete-text", |t| {
        t.edit_by_path(&[0, 0, 0], &[0, 0, 1], Vec::new(), 0)
    }),
];

/// JS "Tree History - single client split L2 chained ops", every pair of
/// split-l2, insert-text and delete-text but `split-l2 → split-l2` (see
/// the test below).
#[test]
fn single_client_undoes_level_two_splits_chained_with_edits() -> Result<()> {
    for (name1, op1) in LEVEL_TWO_CHAIN_OPS {
        for (name2, op2) in LEVEL_TWO_CHAIN_OPS {
            if name1 == "split-l2" && name2 == "split-l2" {
                continue;
            }
            let mut doc = nested_abcd_doc()?;
            assert_chain(&mut doc, &format!("{name1} → {name2}"), &[&op1, &op2])?;
        }
    }
    Ok(())
}

/// JS "should undo chain: split-l2 → split-l2", skipped in JS and Go
/// (`go:history_tree_split_test.go` 737) for the same known bug.
#[test]
#[ignore = "TODO(#1235): split-l2 → split-l2 undo chain has a known undo bug: \
            the boundary-deletion reverse op doesn't correctly restore the state \
            when two consecutive L2 splits produce tombstoned structure"]
fn single_client_undoes_two_chained_level_two_splits() -> Result<()> {
    let split = LEVEL_TWO_CHAIN_OPS[0].1;
    let mut doc = nested_abcd_doc()?;
    assert_chain(&mut doc, "split-l2 → split-l2", &[&split, &split])
}

/// JS `initDoc`: `<doc><p><inline></inline></p></doc>`.
fn inline_doc() -> Result<Document> {
    let mut doc = Document::new("split-repro");
    doc.update(|r| {
        r.set_tree(
            "t",
            element(
                "doc",
                vec![element("p", vec![element("inline", Vec::new())])],
            ),
        )
        .map(|_| ())
    })?;
    Ok(doc)
}

/// JS `insertSiblingBlock`.
fn insert_sibling_block(doc: &mut Document) -> Result<()> {
    edit_tree(doc, |t| {
        t.edit_by_path(
            &[1],
            &[1],
            vec![element("p", vec![element("inline", Vec::new())])],
            0,
        )
    })
}

/// JS `typeInSecondBlock`: types `ch` at the end of the second `<p>`'s
/// inline, located from the visible XML.
fn type_in_second_block(doc: &mut Document, ch: &str) -> Result<()> {
    edit_tree(doc, |t| {
        let xml = t.to_xml()?;
        let typed = xml
            .strip_suffix("</inline></p></doc>")
            .and_then(|head| head.rsplit_once("<inline>"))
            .map_or(0, |(_, typed)| typed.encode_utf16().count());
        t.edit_by_path(&[1, 0, typed], &[1, 0, typed], vec![text(ch)], 0)
    })
}

/// JS `topRedoTreeEdit`: the newest Tree edit of the top redo entry.
fn top_redo_tree_edit(doc: &Document) -> Option<&TreeEditOperation> {
    doc.redo_stack_for_test()
        .last()?
        .iter()
        .rev()
        .find_map(|op| match op {
            HistoryOperation::Operation(op) => match op.as_ref() {
                Operation::TreeEdit(op) => Some(op),
                _ => None,
            },
            HistoryOperation::Presence(_) => None,
        })
}

/// JS `reversePayload`: the spans the reverse carries, either kind.
fn reverse_payload(op: Option<&TreeEditOperation>) -> Vec<&TreeRestoreSpan> {
    op.map(|op| {
        op.restore_spans()
            .iter()
            .chain(op.retombstone_spans())
            .collect()
    })
    .unwrap_or_default()
}

/// JS `fingerprint`: an ordered, identity-bearing description of a reverse
/// payload, so a return to copy-reinsert, which mints fresh IDs every
/// cycle, shows.
fn fingerprint(op: Option<&TreeEditOperation>) -> Vec<String> {
    reverse_payload(op)
        .into_iter()
        .map(|span| {
            [
                span.id.to_id_string(),
                span.parent_id
                    .as_ref()
                    .map_or("-".to_owned(), |id| id.to_id_string()),
                span.node_type.clone(),
                span.length.to_string(),
                span.value.clone().unwrap_or_default(),
            ]
            .join("/")
        })
        .collect()
}

/// Types "asdf" in the second block, undoes each character and then the
/// block insert, as the JS pre-tombstoned filter cases do.
fn type_and_undo_back_to_one_block(doc: &mut Document) -> Result<()> {
    for ch in ["a", "s", "d", "f"] {
        type_in_second_block(doc, ch)?;
    }
    for _ in 0..4 {
        undo(doc)?;
    }
    undo(doc)
}

/// JS "Tree History - single client reverseOp pre-tombstoned filter",
/// "should not accumulate reverseOp payload across redo cycles".
#[test]
fn keeps_the_reverse_payload_across_redo_cycles() -> Result<()> {
    let mut doc = inline_doc()?;
    insert_sibling_block(&mut doc)?;

    let mut fingerprints = Vec::new();
    for _ in 0..4 {
        type_and_undo_back_to_one_block(&mut doc)?;
        fingerprints.push(fingerprint(top_redo_tree_edit(&doc)));
        redo(&mut doc)?;
    }

    // Non-empty matters as much as constant: an empty payload would be
    // constant while carrying nothing.
    assert!(!fingerprints[0].is_empty());
    assert!(
        fingerprints.iter().all(|f| *f == fingerprints[0]),
        "{fingerprints:?}"
    );
    Ok(())
}

/// JS "should produce a reverse payload that omits pre-tombstoned nodes".
#[test]
fn leaves_pre_tombstoned_nodes_out_of_the_reverse_payload() -> Result<()> {
    let mut doc = inline_doc()?;
    insert_sibling_block(&mut doc)?;
    type_and_undo_back_to_one_block(&mut doc)?;

    let redo_top = top_redo_tree_edit(&doc);
    assert!(redo_top.is_some());
    let spans = reverse_payload(redo_top);
    // The reversed edit inserted <p><inline></inline></p> and nothing else:
    // the characters were removed by their own undos before it.
    assert_eq!(
        vec!["p", "inline"],
        spans
            .iter()
            .map(|span| span.node_type.as_str())
            .collect::<Vec<_>>()
    );
    assert!(!spans.iter().any(|span| span.is_text));

    let mut violations = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (i, span) in spans.iter().enumerate() {
        let key = span.id.to_id_string();
        if !seen.insert(key.clone()) {
            violations.push(format!("{}: duplicate id {key}", span.node_type));
        }
        // Without a parent anchor, recreating the node skips it.
        let Some(parent_id) = &span.parent_id else {
            violations.push(format!("{}: missing parentID", span.node_type));
            continue;
        };
        // Parent before child, so a purged subtree is rebuilt top-down.
        if spans
            .iter()
            .position(|s| s.id == *parent_id)
            .is_some_and(|parent| parent > i)
        {
            violations.push(format!(
                "{}: parent span comes after the child",
                span.node_type
            ));
        }
        let expected = span
            .value
            .as_deref()
            .filter(|_| span.is_text)
            .map_or(0, |value| value.encode_utf16().count());
        if span.length != expected {
            violations.push(format!(
                "{}: length={} expected={expected}",
                span.node_type, span.length
            ));
        }
    }
    assert_eq!(Vec::<String>::new(), violations);
    Ok(())
}

/// JS "should record text span length in UTF-16 code units".
#[test]
fn records_a_text_span_length_in_utf16_code_units() -> Result<()> {
    let mut doc = inline_doc()?;
    insert_sibling_block(&mut doc)?;

    // U+20BB7: one code point, two UTF-16 code units.
    let astral = "𠮷";
    assert_eq!(1, astral.chars().count());
    assert_eq!(2, astral.encode_utf16().count());

    edit_tree(&mut doc, |t| {
        t.edit_by_path(&[1, 0, 0], &[1, 0, 0], vec![text(astral)], 0)
    })?;
    undo(&mut doc)?;

    let spans = reverse_payload(top_redo_tree_edit(&doc));
    let text_spans = spans
        .into_iter()
        .filter(|span| span.is_text)
        .collect::<Vec<_>>();
    assert_eq!(1, text_spans.len());
    assert_eq!(Some(astral), text_spans[0].value.as_deref());
    assert_eq!(2, text_spans[0].length);
    Ok(())
}

/// JS "should allow typing at the correct position after redo".
#[test]
fn types_at_the_right_position_after_a_redo() -> Result<()> {
    let mut doc = inline_doc()?;
    insert_sibling_block(&mut doc)?;
    type_and_undo_back_to_one_block(&mut doc)?;
    assert_eq!("<doc><p><inline></inline></p></doc>", xml(&doc));

    redo(&mut doc)?;
    assert_eq!(
        "<doc><p><inline></inline></p><p><inline></inline></p></doc>",
        xml(&doc)
    );
    type_in_second_block(&mut doc, "x")?;
    assert_eq!(
        "<doc><p><inline></inline></p><p><inline>x</inline></p></doc>",
        xml(&doc)
    );
    Ok(())
}

/// JS "should remain stable across three cycles followed by typing".
#[test]
fn stays_stable_across_three_cycles_and_then_typing() -> Result<()> {
    let mut doc = inline_doc()?;
    insert_sibling_block(&mut doc)?;

    for cycle in 0..3 {
        type_and_undo_back_to_one_block(&mut doc)?;
        redo(&mut doc)?;
        assert_eq!(
            "<doc><p><inline></inline></p><p><inline></inline></p></doc>",
            xml(&doc),
            "cycle {cycle}"
        );
    }
    type_in_second_block(&mut doc, "z")?;
    assert_eq!(
        "<doc><p><inline></inline></p><p><inline>z</inline></p></doc>",
        xml(&doc)
    );
    Ok(())
}

// --- split_ticket_test.ts ---

const ACTOR_1: &str = "000000000000000000000001";
const ACTOR_2: &str = "000000000000000000000002";

/// JS `split_ticket_test.ts` `threeBlockDoc`:
/// `<r><d><p>ab</p></d><d><p>cd</p></d><d><p>ef</p></d></r>`, the shape that
/// puts two level 2 split reverses into one undo entry.
fn three_block_doc() -> Result<Document> {
    let mut doc = Document::new("doc");
    doc.set_actor(ACTOR_1);
    doc.update(|r| {
        r.set_tree(
            "t",
            element(
                "r",
                ["ab", "cd", "ef"]
                    .into_iter()
                    .map(|value| element("d", vec![p(value)]))
                    .collect(),
            ),
        )
        .map(|_| ())
    })?;
    Ok(doc)
}

/// Merges the three blocks of `three_block_doc` in one change: two level 2
/// merges, whose reverses are two level 2 splits in one undo entry.
fn merge_three_blocks(doc: &mut Document) -> Result<()> {
    edit_tree(doc, |t| {
        t.edit(4, 8, Vec::new(), 0)?;
        t.edit(6, 10, Vec::new(), 0)
    })?;
    assert_eq!("<r><d><p>abcdef</p></d></r>", xml(doc));
    Ok(())
}

/// JS `split_ticket_test.ts` "gives each split reverse in one undo entry
/// its own tickets": each split reverse takes tickets the undo issued for
/// it, rather than rebuilding them from its own ticket onto the next
/// operation's (Go `TestTreeSplitUndo`).
#[test]
fn gives_each_split_reverse_in_one_undo_entry_its_own_tickets() -> Result<()> {
    let mut doc = three_block_doc()?;
    let before = xml(&doc);
    merge_three_blocks(&mut doc)?;

    undo(&mut doc)?;
    assert_eq!(before, xml(&doc));
    assert_eq!(Vec::<String>::new(), duplicated_ids(&doc));

    // A redo replays the merges and a second undo splits again, each split
    // under tickets of its own.
    redo(&mut doc)?;
    assert_eq!("<r><d><p>abcdef</p></d></r>", xml(&doc));
    undo(&mut doc)?;
    assert_eq!(before, xml(&doc));
    assert_eq!(Vec::<String>::new(), duplicated_ids(&doc));
    Ok(())
}

/// JS "applies such an undo entry on a replica across the wire".
#[test]
fn applies_an_undo_entry_of_split_reverses_on_a_replica() -> Result<()> {
    let mut doc = three_block_doc()?;
    merge_three_blocks(&mut doc)?;
    undo(&mut doc)?;

    let pack = through_wire(&doc.create_change_pack())?;
    let mut replica = Document::new("doc");
    replica.set_actor(ACTOR_2);
    replica.apply_change_pack(&ChangePack::create(
        pack.document_key(),
        Checkpoint::new(0, 0),
        false,
        pack.changes().to_vec(),
        Some(VersionVector::new()),
        None,
    ))?;

    assert_eq!(xml(&doc), xml(&replica));
    assert_eq!(live_ids(&doc), live_ids(&replica));
    assert_eq!(Vec::<String>::new(), duplicated_ids(&replica));
    Ok(())
}

// --- undo_copy_path_test.ts ---

/// JS `undo_copy_path_test.ts` `buildDoc`: `<r><p>abcdef</p></r>`.
fn abcdef_doc() -> Result<Document> {
    let mut doc = Document::new("doc");
    doc.update(|r| r.set_tree("t", element("r", vec![p("abcdef")])).map(|_| ()))?;
    Ok(doc)
}

/// JS "undo through the copy-reinsert reverse", "restores the text without
/// duplicating an id": the copy the undo inserts takes fresh IDs, not those
/// of the tombstones it was copied from.
#[test]
fn restores_text_through_a_copy_without_duplicating_an_id() -> Result<()> {
    let mut doc = abcdef_doc()?;
    let before = xml(&doc);

    let _forced = force_copy_path::force();
    edit_tree(&mut doc, |t| t.edit(2, 5, Vec::new(), 0))?;
    assert_eq!("<r><p>aef</p></r>", xml(&doc));

    undo(&mut doc)?;
    assert_eq!(before, xml(&doc), "the undo restores the text");
    assert_eq!(
        Vec::<String>::new(),
        duplicated_ids(&doc),
        "the copy must not reuse the tombstone it came from"
    );
    Ok(())
}

/// JS "does not splice the copy into the chain it came from": a fresh
/// identity is fresh in the split chain and merge lineage too.
#[test]
fn does_not_splice_a_copy_into_the_chain_it_came_from() -> Result<()> {
    let mut doc = abcdef_doc()?;

    let _forced = force_copy_path::force();
    edit_tree(&mut doc, |t| t.edit(2, 5, Vec::new(), 0))?;
    let before = crdt_tree(&doc)
        .nodes()
        .map(|node| node.id_string())
        .collect::<std::collections::BTreeSet<_>>();

    undo(&mut doc)?;

    let mut inserted = Vec::new();
    for node in crdt_tree(&doc).nodes() {
        let id = node.id_string();
        if before.contains(&id) {
            continue;
        }
        assert_eq!(None, node.ins_prev_id(), "{id} kept a split chain");
        assert_eq!(None, node.ins_next_id(), "{id} kept a split chain");
        assert_eq!(None, node.merged_from(), "{id} kept a merge lineage");
        assert_eq!(None, node.merged_at(), "{id} kept a merge lineage");
        inserted.push(id);
    }
    assert!(
        !inserted.is_empty(),
        "the undo re-inserted the removed content"
    );
    Ok(())
}

/// JS "returns to the deleted state on redo".
#[test]
fn returns_to_the_deleted_state_on_a_copy_redo() -> Result<()> {
    let mut doc = abcdef_doc()?;

    let _forced = force_copy_path::force();
    edit_tree(&mut doc, |t| t.edit(2, 5, Vec::new(), 0))?;
    let deleted = xml(&doc);

    undo(&mut doc)?;
    redo(&mut doc)?;
    assert_eq!(deleted, xml(&doc));
    assert_eq!(Vec::<String>::new(), duplicated_ids(&doc));
    Ok(())
}

// --- Delivery between replicas ---

/// A replica under actor `n` (JS `newReplica`).
fn replica(key: &str, actor: &str) -> Document {
    let mut doc = Document::new(key);
    doc.set_actor(actor);
    doc
}

/// JS `recordChanges`: drains the replica's pending local changes and acks
/// them. A step that produced no change fails, so a convergence check after
/// it cannot hold trivially.
fn record_changes(from: &mut Document, what: &str) -> Result<Vec<Change>> {
    let pack = from.create_change_pack();
    let changes = pack.changes().to_vec();
    assert!(!changes.is_empty(), "{what} produced no change to deliver");
    let last_seq = changes.last().map_or(0, |change| change.id().client_seq());
    from.apply_change_pack(&ChangePack::create(
        from.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))?;
    Ok(changes)
}

/// JS `deliver`, in memory: the neutral checkpoint keeps the receiver's own
/// pending changes, and the empty vector keeps GC out of the delivery.
fn deliver(to: &mut Document, changes: Vec<Change>) -> Result<()> {
    to.apply_change_pack(&ChangePack::create(
        to.key(),
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    ))
}

/// JS `deliver` of `tree_style_combined_test.ts`, which routes the changes
/// through the encoding, so the peer sees only what the wire carried.
fn deliver_through_wire(to: &mut Document, changes: Vec<Change>) -> Result<()> {
    let pack = through_wire(&ChangePack::create(
        to.key(),
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    ))?;
    to.apply_change_pack(&pack)
}

// --- tree_style_combined_test.ts ---

/// JS `tree_style_combined_test.ts` "converges after undoing a style that
/// overwrote and added attributes": the reverse restores `b` and removes
/// `c` in one operation, and the peer decodes both halves of it. As in JS
/// and Go, a style carrying both runs the set half only.
#[test]
fn converges_after_undoing_a_style_that_overwrote_and_added() -> Result<()> {
    let key = "tree-style-combined";
    let mut a = replica(key, ACTOR_1);
    let mut b = replica(key, ACTOR_2);

    a.update(|r| {
        r.set_tree(
            "t",
            element(
                "doc",
                vec![TreeNodeInput::element_with_attributes(
                    "p",
                    attrs(&[("b", "x")]),
                    Vec::new(),
                )],
            ),
        )
        .map(|_| ())
    })?;
    deliver_through_wire(&mut b, record_changes(&mut a, "initial tree")?)?;

    edit_tree(&mut a, |t| t.style(0, 1, attrs(&[("b", "y"), ("c", "z")])))?;
    deliver_through_wire(&mut b, record_changes(&mut a, "combined style")?)?;
    assert_eq!(r#"<doc><p b="y" c="z"></p></doc>"#, xml(&a));
    assert_eq!(xml(&a), xml(&b));

    undo(&mut a)?;
    deliver_through_wire(&mut b, record_changes(&mut a, "undo the combined style")?)?;
    assert_eq!(r#"<doc><p b="x" c="z"></p></doc>"#, xml(&a));
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

// --- style_tombstone_test.ts ---

/// JS `style_tombstone_test.ts` `crossSync`, in memory.
fn cross_sync(d1: &mut Document, d2: &mut Document) -> Result<()> {
    let p1 = d1.create_change_pack().changes().to_vec();
    let p2 = d2.create_change_pack().changes().to_vec();
    deliver(d2, p1)?;
    deliver(d1, p2)?;
    for doc in [d1, d2] {
        let last_seq = doc
            .create_change_pack()
            .changes()
            .last()
            .map_or(0, |change| change.id().client_seq());
        doc.apply_change_pack(&ChangePack::create(
            doc.key(),
            Checkpoint::new(0, last_seq),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        ))?;
    }
    Ok(())
}

/// JS `nodeAttrs`: every piece of the Text under `t`, live or removed, with
/// its attributes, tombstoned ones starred.
fn node_attrs(doc: &Document) -> Vec<String> {
    let Some(CrdtElement::Text(text)) = doc.crdt_root.object().get("t") else {
        panic!("expected a text at t");
    };
    text.nodes()
        .map(|node| {
            let mut attrs = node
                .value()
                .attributes()
                .iter()
                .map(|attr| {
                    let star = if attr.is_removed() { "*" } else { "" };
                    format!("{}={}{star}", attr.key(), attr.value())
                })
                .collect::<Vec<_>>();
            attrs.sort();
            let removed = if node.is_removed() { " (removed)" } else { "" };
            format!(
                "\"{}\"{removed} [{}]",
                node.value().content(),
                attrs.join(",")
            )
        })
        .collect()
}

fn text_json(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_sorted_json(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

/// JS `style_tombstone_test.ts` "sends an undo whose style lands only on a
/// tombstone": the reverse style changes a removed piece and reports no
/// position, yet it ran, so it is sent.
///
/// Rust already sends an undo whose operations ran without reporting a
/// position; what fails is the setup. A style concurrent with a removal
/// lands on the tombstone in JS since `e0609c7a`, where Rust `can_style`
/// still refuses a node removed before the style's ticket, so the replicas
/// disagree before the undo runs.
#[test]
#[ignore = "needs JS e0609c7a: Rust can_style still reads the removal, so a style \
            concurrent with a removal misses the tombstone on the peer (see the \
            porting gaps)"]
fn sends_an_undo_whose_style_lands_only_on_a_tombstone() -> Result<()> {
    let mut d1 = replica("test-doc", ACTOR_1);
    let mut d2 = replica("test-doc", ACTOR_2);
    d1.update(|r| r.set_text("t")?.edit(0, 0, "abcdefghij", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;

    d1.update(|r| r.get_text_mut("t")?.set_style(4, 6, attrs(&[("b", "1")])))?;
    d2.update(|r| r.get_text_mut("t")?.edit(4, 6, "", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!(node_attrs(&d1), node_attrs(&d2), "converged first");

    undo(&mut d1)?;
    assert!(
        !d1.create_change_pack().changes().is_empty(),
        "the undo changed the tombstone, so it has to reach the other replica"
    );

    cross_sync(&mut d1, &mut d2)?;
    assert_eq!(node_attrs(&d1), node_attrs(&d2));
    Ok(())
}

/// JS "does not restore a tombstone's attribute on undo": the previous
/// values of a style come from the first live piece of its range, not from
/// a removed one before it.
#[test]
fn does_not_restore_a_tombstones_attribute_on_undo() -> Result<()> {
    let mut d = Document::new("test-doc");
    d.update(|r| r.set_text("t")?.edit(0, 0, "abcdefghij", None).map(|_| ()))?;
    d.update(|r| r.get_text_mut("t")?.set_style(0, 4, attrs(&[("b", "OLD")])))?;
    d.update(|r| r.get_text_mut("t")?.edit(0, 4, "", None).map(|_| ()))?;
    assert_eq!(r#"[{"val":"efghij"}]"#, text_json(&d));

    d.update(|r| r.get_text_mut("t")?.set_style(0, 4, attrs(&[("b", "NEW")])))?;
    undo(&mut d)?;
    assert_eq!(
        r#"[{"val":"efgh"},{"val":"ij"}]"#,
        text_json(&d),
        "the undo restored an attribute the visible text never carried"
    );
    Ok(())
}

// --- tree_undo_opinfo_test.ts ---

/// `<doc><p>ab</p><p>cd</p></doc>` on two replicas, A having made it.
fn opinfo_replicas() -> Result<(Document, Document)> {
    let key = "tree-undo-opinfo";
    let mut a = replica(key, ACTOR_1);
    let mut b = replica(key, ACTOR_2);
    a.update(|r| {
        r.set_tree("t", element("doc", vec![p("ab"), p("cd")]))
            .map(|_| ())
    })?;
    deliver(&mut b, record_changes(&mut a, "initial tree")?)?;
    Ok((a, b))
}

/// JS `collect`: collects both replicas off the min of their vectors.
fn collect_both(a: &mut Document, b: &mut Document) {
    let min = super::adv_sync::min_version_vector([a.version_vector(), b.version_vector()]);
    a.garbage_collect(&min);
    b.garbage_collect(&min);
}

/// JS `tree_undo_opinfo_test.ts` "reports no position, and still publishes
/// and delivers the undo": the text the undo brings back sits under a
/// paragraph a peer removed, so it takes no room in the index and reports
/// nothing, yet the undo ran and has to reach the peer.
#[test]
fn publishes_and_delivers_an_undo_under_a_removed_ancestor() -> Result<()> {
    use std::sync::{Arc, Mutex};

    let (mut a, mut b) = opinfo_replicas()?;
    edit_tree(&mut a, |t| t.edit_by_path(&[0, 0], &[0, 2], Vec::new(), 0))?;
    deliver(&mut b, record_changes(&mut a, "delete text")?)?;
    edit_tree(&mut b, |t| t.edit_by_path(&[0], &[1], Vec::new(), 0))?;
    deliver(&mut a, record_changes(&mut b, "remove paragraph")?)?;
    assert_eq!("<doc><p>cd</p></doc>", xml(&a));

    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = published.clone();
    let subscription = a.subscribe(move |event| {
        if let crate::DocEvent::LocalChange(info) = event {
            if info.source == crate::operation::OpSource::UndoRedo {
                sink.lock().unwrap().push(info.operations.len());
            }
        }
    });
    undo(&mut a)?;
    a.unsubscribe(subscription);

    assert_eq!(
        vec![0],
        *published.lock().unwrap(),
        "one event, no position"
    );
    assert_eq!("<doc><p>cd</p></doc>", xml(&a));

    // Undoing B's removal brings the paragraph back, and the text with it
    // only if A's restore reached B too.
    deliver(&mut b, record_changes(&mut a, "undo the text deletion")?)?;
    undo(&mut b)?;
    deliver(
        &mut a,
        record_changes(&mut b, "undo the paragraph removal")?,
    )?;

    assert_eq!("<doc><p>ab</p><p>cd</p></doc>", xml(&b));
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

/// JS "leaves a collected node tombstoned when its parent comes back": the
/// undo recreates the purged text under a paragraph removed at that moment,
/// so it is born removed and stays so when the paragraph comes back, on
/// both replicas alike.
#[test]
fn leaves_a_collected_node_removed_when_its_parent_comes_back() -> Result<()> {
    let (mut a, mut b) = opinfo_replicas()?;
    edit_tree(&mut a, |t| t.edit_by_path(&[0, 0], &[0, 2], Vec::new(), 0))?;
    deliver(&mut b, record_changes(&mut a, "delete text")?)?;
    collect_both(&mut a, &mut b);
    assert_eq!(0, a.garbage_len(), "the text tombstone is collected");
    assert_eq!(0, b.garbage_len(), "on both replicas");

    edit_tree(&mut b, |t| t.edit_by_path(&[0], &[1], Vec::new(), 0))?;
    deliver(&mut a, record_changes(&mut b, "remove paragraph")?)?;
    assert_eq!("<doc><p>cd</p></doc>", xml(&a));

    undo(&mut a)?;
    deliver(&mut b, record_changes(&mut a, "undo the text deletion")?)?;
    undo(&mut b)?;
    deliver(
        &mut a,
        record_changes(&mut b, "undo the paragraph removal")?,
    )?;

    assert_eq!("<doc><p></p><p>cd</p></doc>", xml(&b));
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

// --- One undo entry ---

/// JS `Document.update` pushes every reverse of a change as one entry,
/// newest first: a change mixing a Text edit, a Tree edit and a Tree style
/// with an object set is undone and redone as one.
#[test]
fn undoes_and_redoes_a_change_mixing_text_tree_and_object_as_one() -> Result<()> {
    // What the change shows: the text, the tree and the object member.
    let state = |doc: &Document| -> (String, String, String) {
        let Some(JsonValue::Text(x)) = doc.get_root().get("x") else {
            panic!("expected a text at x");
        };
        let k = doc.get_root().get("k").map(|k| format!("{k:?}"));
        (x.to_string(), xml(doc), k.unwrap_or_default())
    };
    let mut doc = Document::new("test-doc");
    doc.update(|r| {
        r.set_text("x")?.edit(0, 0, "hello", None)?;
        r.set_tree("t", element("doc", vec![p("ab")]))?;
        r.set("k", "v").map(|_| ())
    })?;
    let before = state(&doc);

    doc.update(|r| {
        r.get_text_mut("x")?.edit(0, 1, "J", None)?;
        let tree = r.get_tree_mut("t")?;
        tree.edit(2, 2, vec![text("X")], 0)?;
        tree.style(0, 1, attrs(&[("bold", "true")]))?;
        r.set("k", "w").map(|_| ())
    })?;
    let after = state(&doc);
    assert_eq!(r#"<doc><p bold="true">aXb</p></doc>"#, after.1);

    let entry = doc.undo_stack_for_test().last().expect("one entry");
    let kinds = entry
        .iter()
        .map(|op| match op {
            HistoryOperation::Operation(op) => match op.as_ref() {
                Operation::Set(_) => "set",
                Operation::TreeStyle(_) => "tree-style",
                Operation::TreeEdit(_) => "tree-edit",
                Operation::Edit(_) => "edit",
                other => panic!("unexpected reverse {other:?}"),
            },
            HistoryOperation::Presence(_) => "presence",
        })
        .collect::<Vec<_>>();
    assert_eq!(vec!["set", "tree-style", "tree-edit", "edit"], kinds);

    undo(&mut doc)?;
    assert_eq!(before, state(&doc));
    redo(&mut doc)?;
    assert_eq!(after, state(&doc));
    undo(&mut doc)?;
    assert_eq!(before, state(&doc));
    Ok(())
}

/// JS `TreeEditOperation.execute` resolves an undo edit's indexes right
/// before that edit runs, and the change carries the positions it resolved
/// to other replicas. Here one change splits at 5 and then at 2, so the
/// entry deletes the boundary at 2 first, which moves the one recorded at
/// 5 back to where it was; positions resolved before that would name the
/// wrong nodes on a peer that reads them off the wire.
#[test]
fn resolves_each_reverse_of_an_entry_after_the_one_before_it_ran() -> Result<()> {
    let mut a = replica("test-doc", ACTOR_1);
    let mut b = replica("test-doc", ACTOR_2);
    a.update(|r| {
        r.set_tree("t", element("doc", vec![p("abcdef")]))
            .map(|_| ())
    })?;
    let before = xml(&a);
    edit_tree(&mut a, |t| {
        t.edit(5, 5, Vec::new(), 1)?;
        t.edit(2, 2, Vec::new(), 1)
    })?;
    let after = "<doc><p>a</p><p>bcd</p><p>ef</p></doc>";
    assert_eq!(after, xml(&a));
    deliver_through_wire(&mut b, record_changes(&mut a, "two splits")?)?;

    undo(&mut a)?;
    assert_eq!(before, xml(&a));
    deliver_through_wire(&mut b, record_changes(&mut a, "undo")?)?;
    assert_eq!(
        xml(&a),
        xml(&b),
        "the peer ran the positions the undo carried"
    );

    redo(&mut a)?;
    assert_eq!(after, xml(&a));
    deliver_through_wire(&mut b, record_changes(&mut a, "redo")?)?;
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

/// JS `applyChangeInternal`: an undo recorded before a peer's edit is
/// moved past what that edit changed (`reconcileTreeEdit`). The reverse of
/// a split deletes the boundary by index, and the peer's insert before it
/// shifts that index.
#[test]
fn moves_an_undo_past_a_remote_tree_edit() -> Result<()> {
    let mut a = replica("test-doc", ACTOR_1);
    let mut b = replica("test-doc", ACTOR_2);
    a.update(|r| r.set_tree("t", element("doc", vec![p("abcd")])).map(|_| ()))?;
    deliver(&mut b, record_changes(&mut a, "initial tree")?)?;

    edit_tree(&mut a, |t| t.edit(3, 3, Vec::new(), 1))?;
    edit_tree(&mut b, |t| t.edit(1, 1, vec![text("XY")], 0))?;
    deliver(&mut b, record_changes(&mut a, "split")?)?;
    deliver(&mut a, record_changes(&mut b, "insert")?)?;
    assert_eq!("<doc><p>XYab</p><p>cd</p></doc>", xml(&a));
    assert_eq!(xml(&a), xml(&b));

    undo(&mut a)?;
    assert_eq!("<doc><p>XYabcd</p></doc>", xml(&a));
    deliver(&mut b, record_changes(&mut a, "undo the split")?)?;
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

// --- tree_redo_split_style_test.ts ---

/// JS `grab` of `tree_redo_split_style_test.ts`: the pending local changes,
/// through the wire, acked.
fn grab(from: &mut Document) -> Result<Vec<Change>> {
    let pack = from.create_change_pack();
    let wire = through_wire(&pack)?;
    ack(from, &pack)?;
    Ok(wire.changes().to_vec())
}

/// `grab` without the wire, for the cases whose undo is an identity edit:
/// the wire does not carry restore spans yet (Task 6 adds them and moves
/// these cases to `grab`, as JS sends them through protobuf).
fn grab_in_memory(from: &mut Document) -> Result<Vec<Change>> {
    let pack = from.create_change_pack();
    ack(from, &pack)?;
    Ok(pack.changes().to_vec())
}

/// Acks the changes of `pack`, as the server would.
fn ack(from: &mut Document, pack: &ChangePack) -> Result<()> {
    let last_seq = pack
        .changes()
        .last()
        .map_or(0, |change| change.id().client_seq());
    from.apply_change_pack(&ChangePack::create(
        from.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))
}

/// How a test hands a replica's changes over.
type Grab = fn(&mut Document) -> Result<Vec<Change>>;

/// `hello world` in an inline in a paragraph, two levels to split (JS
/// `seed`).
fn seed_inline(doc: &mut Document) -> Result<()> {
    doc.update(|r| {
        r.set_tree(
            "t",
            element(
                "root",
                vec![element(
                    "paragraph",
                    vec![element("inline", vec![text("hello world")])],
                )],
            ),
        )
        .map(|_| ())
    })
}

/// `<p>ab</p><p>cd</p>`, two blocks to merge and split again (JS
/// `seedBlocks`).
fn seed_blocks(doc: &mut Document) -> Result<()> {
    doc.update(|r| {
        r.set_tree("t", element("root", vec![p("ab"), p("cd")]))
            .map(|_| ())
    })
}

/// JS `collectBoth`: purges every tombstone both replicas have seen.
fn collect_all(a: &mut Document, b: &mut Document) {
    let mut vector = VersionVector::new();
    for doc in [&*a, &*b] {
        vector.set(doc.change_id.actor_id().clone(), i64::MAX);
    }
    a.garbage_collect(&vector);
    b.garbage_collect(&vector);
}

fn bold() -> BTreeMap<String, Value> {
    attrs(&[("bold", "true")])
}

/// Two replicas of the inline seed, B having received it, with A's history
/// cleared.
fn seeded_pair(seed: fn(&mut Document) -> Result<()>) -> Result<(Document, Document)> {
    let mut a = replica("d", ACTOR_1);
    let mut b = replica("d", ACTOR_2);
    seed(&mut a)?;
    a.clear_history();
    deliver(&mut b, grab(&mut a)?)?;
    Ok((a, b))
}

/// JS `tree_redo_split_style_test.ts` "lets a peer apply the redo", the
/// five-row table, each with and without GC: a split and an operation
/// naming the element it created, in one change. The redo re-mints the
/// element, so the rest of the entry and the entry it pushes have to name
/// the new one, or the peer cannot apply them.
#[test]
fn lets_a_peer_apply_the_redo_of_a_split_and_what_names_it() -> Result<()> {
    // The last row's insert reverses to an identity edit, whose spans name
    // the split-created element as their parent.
    let cases: [(&str, TreeChange<'_>, Grab); 5] = [
        (
            "split in the middle, bold the right piece",
            &|t| {
                t.edit_by_path(&[0, 0, 6], &[0, 0, 6], Vec::new(), 1)?;
                t.style_by_path(&[0, 1], bold())
            },
            grab,
        ),
        (
            "split at the end, bold the empty piece",
            &|t| {
                t.edit_by_path(&[0, 0, 11], &[0, 0, 11], Vec::new(), 1)?;
                t.style_by_path(&[0, 1], bold())
            },
            grab,
        ),
        (
            "split two levels, bold the new paragraph",
            &|t| {
                t.edit_by_path(&[0, 0, 6], &[0, 0, 6], Vec::new(), 2)?;
                t.style_by_path(&[1], bold())
            },
            grab,
        ),
        (
            // The style names the inner element the second ticket did not
            // mint, which resolves only if the pairing is innermost first.
            "split two levels, bold the new inline",
            &|t| {
                t.edit_by_path(&[0, 0, 6], &[0, 0, 6], Vec::new(), 2)?;
                t.style_by_path(&[1, 0], bold())
            },
            grab,
        ),
        (
            "split, then insert into the new element",
            &|t| {
                t.edit_by_path(&[0, 0, 6], &[0, 0, 6], Vec::new(), 1)?;
                t.edit_by_path(&[0, 1, 0], &[0, 1, 0], vec![text("X")], 0)
            },
            grab_in_memory,
        ),
    ];

    for (name, edit, grab) in cases {
        for collect in [false, true] {
            let name = format!("{name}{}", if collect { ", after GC" } else { "" });
            let (mut a, mut b) = seeded_pair(seed_inline)?;

            edit_tree(&mut a, edit)?;
            let edited = xml(&a);
            deliver(&mut b, grab(&mut a)?)?;
            assert_eq!(edited, xml(&b), "{name}: edit");

            undo(&mut a)?;
            deliver(&mut b, grab(&mut a)?)?;
            let undone = xml(&a);
            assert_eq!(undone, xml(&b), "{name}: undo");
            // Both have seen the undo, so its tombstones can be purged.
            if collect {
                collect_all(&mut a, &mut b);
            }

            redo(&mut a)?;
            assert_eq!(edited, xml(&a), "{name}: redo, locally");
            deliver(&mut b, grab(&mut a)?)?;
            assert_eq!(edited, xml(&b), "{name}: redo, on the peer");

            // A second round trip: the redo re-minted the split elements,
            // so the entry it pushed onto the undo stack has to name them.
            undo(&mut a)?;
            deliver(&mut b, grab(&mut a)?)?;
            assert_eq!(undone, xml(&a), "{name}: second undo, locally");
            assert_eq!(undone, xml(&b), "{name}: second undo, on the peer");
            redo(&mut a)?;
            deliver(&mut b, grab(&mut a)?)?;
            assert_eq!(edited, xml(&a), "{name}: second redo, locally");
            assert_eq!(edited, xml(&b), "{name}: second redo, on the peer");
        }
    }
    Ok(())
}

/// JS `tree_redo_split_style_test.ts` "re-points another history entry at
/// the re-split elements", with and without GC: the split and the style
/// naming its element are in different entries, so only the sweep of the
/// stacks (`History::reconcile_tree_node_id`) can re-point the style.
#[test]
fn re_points_another_history_entry_at_the_re_split_elements() -> Result<()> {
    for collect in [false, true] {
        let name = if collect { "after GC" } else { "without GC" };
        let (mut a, mut b) = seeded_pair(seed_inline)?;

        // Entry 1: the split. Entry 2: a style naming what it created.
        edit_tree(&mut a, |t| {
            t.edit_by_path(&[0, 0, 6], &[0, 0, 6], Vec::new(), 1)
        })?;
        let split = xml(&a);
        edit_tree(&mut a, |t| t.style_by_path(&[0, 1], bold()))?;
        let styled = xml(&a);
        deliver(&mut b, grab(&mut a)?)?;
        assert_eq!(styled, xml(&b), "{name}: edits");

        // Undo the style, then the split. The redo stack now holds a
        // re-split entry and, above it, a style naming the pre-split element.
        undo(&mut a)?;
        deliver(&mut b, grab(&mut a)?)?;
        undo(&mut a)?;
        deliver(&mut b, grab(&mut a)?)?;
        assert_eq!(xml(&a), xml(&b), "{name}: both undone");
        if collect {
            collect_all(&mut a, &mut b);
        }

        // Redoing the split mints new elements; the style entry still on
        // the stack has to follow them, or the peer cannot apply it.
        redo(&mut a)?;
        deliver(&mut b, grab(&mut a)?)?;
        assert_eq!(split, xml(&a), "{name}: redo split, locally");
        assert_eq!(split, xml(&b), "{name}: redo split, on the peer");
        if collect {
            collect_all(&mut a, &mut b);
        }

        redo(&mut a)?;
        assert_eq!(styled, xml(&a), "{name}: redo style, locally");
        deliver(&mut b, grab(&mut a)?)?;
        assert_eq!(styled, xml(&b), "{name}: redo style, on the peer");
    }
    Ok(())
}

/// JS `tree_redo_split_style_test.ts` "re-points the history when the split
/// is a local edit": a peer merges two blocks, and a plain split here
/// separates them again under a new ID. An entry naming the merged-away
/// block has to follow it, or its undo names a purged node.
#[test]
fn re_points_the_history_when_the_split_is_a_local_edit() -> Result<()> {
    let (mut a, mut b) = seeded_pair(seed_blocks)?;

    // A's entry names the second block; B merges it into the first.
    edit_tree(&mut a, |t| t.style_by_path(&[1], bold()))?;
    deliver(&mut b, grab(&mut a)?)?;
    edit_tree(&mut b, |t| t.edit_by_path(&[0, 2], &[1, 0], Vec::new(), 0))?;
    deliver(&mut a, grab(&mut b)?)?;

    // A splits the blocks apart again, then the merged-away block is purged.
    edit_tree(&mut a, |t| t.edit_by_path(&[0, 2], &[0, 2], Vec::new(), 1))?;
    deliver(&mut b, grab(&mut a)?)?;
    collect_all(&mut a, &mut b);

    undo(&mut a)?;
    deliver(&mut b, grab(&mut a)?)?;
    undo(&mut a)?;
    deliver(&mut b, grab(&mut a)?)?;
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

/// JS `tree_redo_split_style_test.ts` "re-points the history when the split
/// arrives from a peer": the same, with the merge and the split made by the
/// peer, so the pairs come from applying its change.
#[test]
fn re_points_the_history_when_the_split_arrives_from_a_peer() -> Result<()> {
    let (mut a, mut b) = seeded_pair(seed_blocks)?;

    // B's entry names the second block; A merges it away and splits again.
    edit_tree(&mut b, |t| t.style_by_path(&[1], bold()))?;
    deliver(&mut a, grab(&mut b)?)?;
    edit_tree(&mut a, |t| t.edit_by_path(&[0, 2], &[1, 0], Vec::new(), 0))?;
    edit_tree(&mut a, |t| t.edit_by_path(&[0, 2], &[0, 2], Vec::new(), 1))?;
    deliver(&mut b, grab(&mut a)?)?;
    collect_all(&mut a, &mut b);

    undo(&mut b)?;
    deliver(&mut a, grab(&mut b)?)?;
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

/// JS `executeUndoRedoInternal`, the stacks re-pointed from what the tree
/// says the split re-created: a peer merged two blocks while the redo of an
/// unrelated split sat on the stack, and the redo separates them again. The
/// redo's own `replaced_ids` name the element its undo merged away, not the
/// peer's block, so only the pair the tree derives re-points the entry that
/// names that block.
#[test]
fn re_points_the_history_at_a_block_a_redo_split_re_creates() -> Result<()> {
    let (mut a, mut b) = seeded_pair(seed_blocks)?;

    // A's first entry names the second block. A's split at the end of the
    // first block opens an empty block, and its undo merges that back.
    edit_tree(&mut a, |t| t.style_by_path(&[1], bold()))?;
    edit_tree(&mut a, |t| t.edit_by_path(&[0, 2], &[0, 2], Vec::new(), 1))?;
    undo(&mut a)?;
    deliver(&mut b, grab(&mut a)?)?;

    // B merges the second block into the first while the redo waits.
    edit_tree(&mut b, |t| t.edit_by_path(&[0, 2], &[1, 0], Vec::new(), 0))?;
    deliver(&mut a, grab(&mut b)?)?;
    assert_eq!("<root><p>abcd</p></root>", xml(&a));

    // The redo splits where the merge joined them, re-creating the block.
    redo(&mut a)?;
    deliver(&mut b, grab(&mut a)?)?;
    assert_eq!("<root><p>ab</p><p>cd</p></root>", xml(&a));
    assert_eq!(xml(&a), xml(&b));
    collect_all(&mut a, &mut b);

    undo(&mut a)?;
    deliver(&mut b, grab(&mut a)?)?;
    undo(&mut a)?;
    deliver(&mut b, grab(&mut a)?)?;
    assert_eq!(xml(&a), xml(&b));
    Ok(())
}

/// `<r><p>ab</p></r>`.
fn ab_doc() -> Result<Document> {
    let mut doc = Document::new("doc");
    doc.update(|r| r.set_tree("t", element("r", vec![p("ab")])).map(|_| ()))?;
    Ok(doc)
}

/// Purges every tombstone of a document edited by one replica.
fn collect_one(doc: &mut Document) {
    let mut vector = VersionVector::new();
    vector.set(doc.change_id.actor_id().clone(), i64::MAX);
    doc.garbage_collect(&vector);
}

/// JS `executeUndoRedoInternal` (`repointRest` of the re-issued content
/// IDs): the undo of a deletion re-inserts a copy under fresh IDs, and the
/// reverse after it in the same entry, the undo of a style on the deleted
/// element, has to follow the copy. Left naming the tombstone, its own
/// reverse names it too, and the redo cannot find it once GC purged it.
#[test]
fn re_points_the_rest_of_an_entry_at_a_re_inserted_copy() -> Result<()> {
    let mut doc = ab_doc()?;
    let before = xml(&doc);

    let _forced = force_copy_path::force();
    edit_tree(&mut doc, |t| {
        t.style(0, 1, bold())?;
        t.edit(0, 4, Vec::new(), 0)
    })?;
    let after = xml(&doc);
    assert_eq!("<r></r>", after);

    undo(&mut doc)?;
    assert_eq!(before, xml(&doc), "undo");
    collect_one(&mut doc);
    redo(&mut doc)?;
    assert_eq!(after, xml(&doc), "redo");
    undo(&mut doc)?;
    assert_eq!(before, xml(&doc), "second undo");
    Ok(())
}

/// The same, with the style and the deletion in separate entries: the
/// stacks follow the re-issued IDs once the undo has run (JS `pending`),
/// or the style's undo names a tombstone GC purged.
#[test]
fn re_points_another_entry_at_a_re_inserted_copy() -> Result<()> {
    let mut doc = ab_doc()?;
    let before = xml(&doc);

    let _forced = force_copy_path::force();
    edit_tree(&mut doc, |t| t.style(0, 1, bold()))?;
    edit_tree(&mut doc, |t| t.edit(0, 4, Vec::new(), 0))?;

    undo(&mut doc)?;
    assert_eq!(r#"<r><p bold="true">ab</p></r>"#, xml(&doc));
    collect_one(&mut doc);
    undo(&mut doc)?;
    assert_eq!(before, xml(&doc));
    Ok(())
}

// --- tree_split_cascade_regression_test.ts ---
//
// JS sends these through protobuf. The undo of a span deletion is an
// identity edit, whose restore spans the wire does not carry yet, so the
// changes go in memory until Task 6 adds them.

/// JS `start`: `n` synced replicas under actors 1..n holding
/// `<p><span>abc</span><span>de</span></p><p><span>fg</span></p>`.
fn cascade_replicas(n: usize) -> Result<Vec<Document>> {
    let mut replicas = (1..=n)
        .map(|i| replica("doc", &format!("{i:024x}")))
        .collect::<Vec<_>>();
    replicas[0].update(|r| {
        r.set_tree(
            "t",
            element(
                "doc",
                vec![
                    element(
                        "p",
                        vec![
                            element("span", vec![text("abc")]),
                            element("span", vec![text("de")]),
                        ],
                    ),
                    element("p", vec![element("span", vec![text("fg")])]),
                ],
            ),
        )
        .map(|_| ())
    })?;
    let init = grab(&mut replicas[0])?;
    for replica in replicas.iter_mut().skip(1) {
        deliver(replica, init.clone())?;
    }
    Ok(replicas)
}

/// JS `visibleText`.
fn visible_text(xml: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            ch if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// A split a replica makes concurrently with the span deletion.
type SplitEdit = fn(&mut JsonTree) -> Result<()>;

/// The editor's Enter right before a styled run (JS `enterAtSpanStart`).
fn enter_at_span_start(t: &mut JsonTree) -> Result<()> {
    t.edit_by_path(&[0, 1, 0], &[0, 1, 0], Vec::new(), 1)?;
    t.split_by_path(&[0, 2])?;
    t.edit_by_path(&[0, 1], &[0, 2], Vec::new(), 0)
}

/// JS `splitAt0DropLeft`.
fn split_at_0_drop_left(t: &mut JsonTree) -> Result<()> {
    t.edit_by_path(&[0, 1, 0], &[0, 1, 0], Vec::new(), 1)?;
    t.edit_by_path(&[0, 1], &[0, 2], Vec::new(), 0)
}

/// JS `tree_split_cascade_regression_test.ts` "delete span + undo vs
/// ${name}: "de" stays visible on both", both rows: r1 deletes the styled
/// span and undoes it while r2 presses Enter right before it; nobody
/// deleted "de".
#[test]
fn keeps_de_after_a_span_delete_and_undo_vs_a_split() -> Result<()> {
    let cases: [(&str, SplitEdit); 2] = [
        ("Enter at span start", enter_at_span_start),
        ("split at 0 + drop left", split_at_0_drop_left),
    ];
    for (name, op) in cases {
        let mut replicas = cascade_replicas(2)?;
        let [r1, r2] = replicas.as_mut_slice() else {
            unreachable!()
        };
        edit_tree(r1, |t| t.edit_by_path(&[0, 1], &[0, 2], Vec::new(), 0))?;
        undo(r1)?;
        edit_tree(r2, op)?;
        let p1 = grab_in_memory(r1)?;
        let p2 = grab_in_memory(r2)?;
        deliver(r2, p1)?;
        deliver(r1, p2)?;
        assert!(
            visible_text(&xml(r1)).contains("de"),
            "{name}: r1 lost \"de\""
        );
        assert!(
            visible_text(&xml(r2)).contains("de"),
            "{name}: r2 lost \"de\""
        );
    }
    Ok(())
}

/// JS `tree_split_cascade_regression_test.ts` "delete span + undo vs
/// split-at-0+drop vs split-in-middle+drop: converge".
#[test]
fn converges_after_a_span_delete_and_undo_vs_two_splits_that_drop() -> Result<()> {
    let mut replicas = cascade_replicas(3)?;
    let [r1, r2, r3] = replicas.as_mut_slice() else {
        unreachable!()
    };
    edit_tree(r1, |t| t.edit_by_path(&[0, 1], &[0, 2], Vec::new(), 0))?;
    undo(r1)?;
    let p1 = grab_in_memory(r1)?;
    edit_tree(r2, split_at_0_drop_left)?;
    let p2 = grab_in_memory(r2)?;
    deliver(r3, p1.clone())?;
    edit_tree(r3, |t| {
        t.edit_by_path(&[0, 1, 1], &[0, 1, 1], Vec::new(), 1)?;
        t.edit_by_path(&[0, 1], &[0, 2], Vec::new(), 0)
    })?;
    let p3 = grab_in_memory(r3)?;
    deliver(r1, p2.clone())?;
    deliver(r1, p3.clone())?;
    deliver(r2, p1)?;
    deliver(r2, p3)?;
    deliver(r3, p2)?;
    assert_eq!(xml(r1), xml(r2), "r2 diverged from r1");
    assert_eq!(xml(r1), xml(r3), "r3 diverged from r1");
    Ok(())
}
