//! CRDT-level tests of what a Tree edit reports for its reverse: the restore
//! spans of the nodes it removed and inserted, the pre-tombstoned set, the
//! split size, and the content it accepted.
//!
//! The span cases follow JS `CRDTTree.edit` (`src/document/crdt/tree.ts` at
//! `8629993e`, span capture and `spansComplete`); the duplicate-ID cases port
//! JS `test/unit/document/crdt/tree_duplicate_id_test.ts`. The restore and
//! retombstone cases port JS `tree_restore_span_kind_test.ts` and the Go
//! `pkg/document/crdt/tree_restore_test.go` cases that use only
//! `Restore`/`Retombstone`.

use super::rht::Rht;
use super::tree::{
    replace_tree_node_id, CrdtTree, TreeEditChange, TreeEditResult, TreeNode, TreeNodeId, TreePos,
    TreeRestoreSpan, TreeRestoredNode, TreeVisibleEdit,
};
use crate::{Result, TimeTicket, TreeNodeInput, VersionVector, YorkieError};
use std::collections::BTreeMap;

/// JS `InitialActorID`, the actor of `timeT` and `posT`.
const ACTOR: &str = "000000000000000000000000";
const OTHER_ACTOR: &str = "0123456789abcdef01234567";

fn ticket(lamport: i64) -> TimeTicket {
    TimeTicket::new(lamport, 0, ACTOR)
}

fn id(lamport: i64, offset: usize) -> TreeNodeId {
    TreeNodeId::new(ticket(lamport), offset)
}

fn element(lamport: i64, node_type: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::create_element(id(lamport, 0), node_type, None, children)
}

fn text(lamport: i64, value: &str) -> TreeNode {
    TreeNode::create_text(id(lamport, 0), value)
}

/// A tree whose root, created at lamport 1, holds `children`.
fn tree(children: Vec<TreeNode>) -> CrdtTree {
    CrdtTree::create(element(1, "root", children), ticket(1))
}

fn known(lamport: i64) -> VersionVector {
    let mut vector = VersionVector::new();
    vector.set(ACTOR, lamport);
    vector
}

/// Edits the visible range `from..to` (JS `editT`).
fn edit_t(
    tree: &mut CrdtTree,
    (from, to): (usize, usize),
    contents: Option<Vec<TreeNode>>,
    split_level: usize,
    edited_at: TimeTicket,
) -> Result<TreeEditResult> {
    let range = (tree.find_pos(from, true)?, tree.find_pos(to, true)?);
    tree.edit_by_range_with_changes(range, contents, split_level, edited_at, None)
}

fn text_span(
    id: TreeNodeId,
    value: &str,
    parent_id: TreeNodeId,
    left_sibling_id: Option<TreeNodeId>,
    right_sibling_id: Option<TreeNodeId>,
) -> TreeRestoreSpan {
    TreeRestoreSpan {
        id,
        node_type: "text".to_owned(),
        is_text: true,
        length: value.encode_utf16().count(),
        value: Some(value.to_owned()),
        attrs: None,
        parent_id: Some(parent_id),
        left_sibling_id,
        right_sibling_id,
    }
}

fn element_span(
    id: TreeNodeId,
    node_type: &str,
    attrs: Option<Rht>,
    parent_id: TreeNodeId,
    left_sibling_id: Option<TreeNodeId>,
    right_sibling_id: Option<TreeNodeId>,
) -> TreeRestoreSpan {
    TreeRestoreSpan {
        id,
        node_type: node_type.to_owned(),
        is_text: false,
        length: 0,
        value: None,
        attrs,
        parent_id: Some(parent_id),
        left_sibling_id,
        right_sibling_id,
    }
}

/// The IDs that name more than one node in the tree (JS `duplicatedIDs`).
fn duplicated_ids(tree: &CrdtTree) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for node in tree.nodes() {
        *counts.entry(node.id_string()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(id, count)| format!("{id} x{count}"))
        .collect()
}

/// JS `buildDigitTree`: `<r><p>0123456789</p></r>`, with the text created
/// at lamport 3.
fn digit_tree() -> CrdtTree {
    let tree = CrdtTree::create(
        element(1, "r", vec![element(2, "p", vec![text(3, "0123456789")])]),
        ticket(1),
    );
    assert_eq!("<r><p>0123456789</p></r>", tree.to_xml());
    tree
}

/// A text range delete reports one span per piece it removed, in document
/// order, each the UTF-16 interval of the piece with a copy of its value.
/// The left anchor is the last character of the left sibling, so it floor-
/// resolves to the piece right before the span.
#[test]
fn reports_spans_for_a_text_range_delete() -> Result<()> {
    // <root><p>abc😀de</p></root>, "abc" at lamport 3 and "😀de" at 4.
    let mut tree = tree(vec![element(
        2,
        "p",
        vec![text(3, "abc"), text(4, "\u{1F600}de")],
    )]);

    // Removes "c" and "😀d": the range splits both texts first.
    let result = edit_t(&mut tree, (3, 7), None, 0, ticket(10))?;
    assert_eq!("<root><p>abe</p></root>", tree.to_xml());

    assert_eq!(
        vec![
            text_span(id(3, 2), "c", id(2, 0), Some(id(3, 1)), Some(id(4, 0))),
            text_span(
                id(4, 0),
                "\u{1F600}d",
                id(2, 0),
                Some(id(3, 2)),
                Some(id(4, 3))
            ),
        ],
        result.removed_spans
    );
    assert_eq!(3, result.removed_spans[1].length);
    assert!(result.inserted_spans.is_empty());
    assert!(result.pre_tombstoned.is_empty());
    assert_eq!(3, result.from_idx);
    Ok(())
}

/// An element delete reports the element before its descendants, the
/// element whole (length 0) with a copy of its attributes.
#[test]
fn reports_spans_for_an_element_delete() -> Result<()> {
    let mut attrs = Rht::new();
    attrs.set("a", "1", ticket(2));
    let p = TreeNode::create_element(id(2, 0), "p", Some(attrs.clone()), vec![text(3, "xy")]);
    let mut tree = tree(vec![p, element(4, "q", Vec::new())]);
    assert_eq!("<root><p a=\"1\">xy</p><q></q></root>", tree.to_xml());

    let result = edit_t(&mut tree, (0, 4), None, 0, ticket(10))?;
    assert_eq!("<root><q></q></root>", tree.to_xml());

    assert_eq!(
        vec![
            element_span(id(2, 0), "p", Some(attrs), id(1, 0), None, Some(id(4, 0))),
            text_span(id(3, 0), "xy", id(2, 0), None, None),
        ],
        result.removed_spans
    );
    assert_eq!(0, result.merge_level);
    Ok(())
}

/// Only nodes the edit itself took from visible to removed get a span. A
/// node the editor did not know was removed is still in the range, so it is
/// reported as pre-tombstoned for the reverse to leave alone.
#[test]
fn reports_no_span_for_a_node_already_removed() -> Result<()> {
    // <root><p>[a][b][c]</p></root>, one text node per character.
    let mut tree = tree(vec![element(
        2,
        "p",
        vec![text(3, "a"), text(4, "b"), text(5, "c")],
    )]);
    let range = (tree.find_pos(1, true)?, tree.find_pos(4, true)?);

    // Another actor removes "b" first.
    let remove_b = (tree.find_pos(2, true)?, tree.find_pos(3, true)?);
    let other = TimeTicket::new(10, 0, OTHER_ACTOR);
    tree.edit_by_range_with_changes(remove_b, None, 0, other, None)?;
    assert_eq!("<root><p>ac</p></root>", tree.to_xml());

    // The editor deletes "abc" without knowing that.
    let result = tree.edit_by_range_with_changes(range, None, 0, ticket(11), Some(&known(5)))?;
    assert_eq!("<root><p></p></root>", tree.to_xml());

    assert_eq!(
        std::collections::BTreeSet::from([id(4, 0).to_id_string()]),
        result.pre_tombstoned
    );
    assert_eq!(
        vec![
            text_span(id(3, 0), "a", id(2, 0), None, Some(id(4, 0))),
            text_span(id(5, 0), "c", id(2, 0), Some(id(4, 0)), None),
        ],
        result.removed_spans
    );
    Ok(())
}

/// An insert reports a span per inserted node, parents before their
/// children. Captured children-first and reversed, as JS does, so siblings
/// come right to left.
#[test]
fn reports_spans_for_an_insert() -> Result<()> {
    let mut tree = tree(vec![element(2, "q", Vec::new())]);
    let content = element(10, "p", vec![text(11, "a"), text(12, "bc")]);

    let result = edit_t(&mut tree, (0, 0), Some(vec![content]), 0, ticket(13))?;
    assert_eq!("<root><p>abc</p><q></q></root>", tree.to_xml());

    assert_eq!(
        vec![
            element_span(id(10, 0), "p", None, id(1, 0), None, Some(id(2, 0))),
            text_span(id(12, 0), "bc", id(10, 0), Some(id(11, 0)), None),
            text_span(id(11, 0), "a", id(10, 0), None, Some(id(12, 0))),
        ],
        result.inserted_spans
    );
    assert!(result.removed_spans.is_empty());
    assert_eq!(5, result.inserted_size);
    Ok(())
}

/// Replacing text reports both lists: what it removed and what it inserted.
#[test]
fn reports_spans_for_a_replace() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "abcd")])]);

    let result = edit_t(&mut tree, (2, 4), Some(vec![text(10, "X")]), 0, ticket(11))?;
    assert_eq!("<root><p>aXd</p></root>", tree.to_xml());

    assert_eq!(
        vec![text_span(
            id(3, 1),
            "bc",
            id(2, 0),
            Some(id(3, 0)),
            Some(id(3, 3))
        )],
        result.removed_spans
    );
    // Inserted right after the left piece "a", before the removed "bc".
    assert_eq!(
        vec![text_span(
            id(10, 0),
            "X",
            id(2, 0),
            Some(id(3, 0)),
            Some(id(3, 1))
        )],
        result.inserted_spans
    );
    Ok(())
}

/// A merge is not a plain deletion, so the spans cannot describe it: both
/// lists are empty, and the merge boundary is reported instead.
#[test]
fn reports_no_spans_for_a_merge() -> Result<()> {
    let mut tree = tree(vec![
        element(2, "p", vec![text(3, "ab")]),
        element(4, "p", vec![text(5, "cd")]),
    ]);

    let result = edit_t(&mut tree, (3, 5), None, 0, ticket(10))?;
    assert_eq!("<root><p>abcd</p></root>", tree.to_xml());

    assert_eq!(1, result.merge_level);
    assert!(result.removed_spans.is_empty());
    assert!(result.inserted_spans.is_empty());
    let merged = result
        .merged_nodes
        .iter()
        .map(|node| node.id().clone())
        .collect::<Vec<_>>();
    assert_eq!(vec![id(4, 0)], merged);
    assert!(result.stranded_nodes.is_empty());
    Ok(())
}

/// A split opens two tokens per level, measured off the tree: a level the
/// tree has no ancestor for opens nothing.
#[test]
fn reports_the_split_size_per_opened_level() -> Result<()> {
    let mut one = tree(vec![element(2, "p", vec![text(3, "ab")])]);
    let result = edit_t(&mut one, (2, 2), None, 1, ticket(10))?;
    assert_eq!("<root><p>a</p><p>b</p></root>", one.to_xml());
    assert_eq!(2, result.split_size);
    assert!(result.removed_spans.is_empty());
    assert!(result.inserted_spans.is_empty());

    let mut two = tree(vec![element(
        2,
        "p",
        vec![element(3, "b", vec![text(4, "ab")])],
    )]);
    let result = edit_t(&mut two, (3, 3), None, 2, ticket(10))?;
    assert_eq!("<root><p><b>a</b></p><p><b>b</b></p></root>", two.to_xml());
    assert_eq!(4, result.split_size);

    // Only `<p>` lies between the position and the root.
    let mut short = tree(vec![element(2, "p", vec![text(3, "ab")])]);
    let result = edit_t(&mut short, (2, 2), None, 2, ticket(10))?;
    assert_eq!("<root><p>a</p><p>b</p></root>", short.to_xml());
    assert_eq!(2, result.split_size);

    // No split, no size.
    let mut none = tree(vec![element(2, "p", vec![text(3, "ab")])]);
    let result = edit_t(&mut none, (1, 2), None, 0, ticket(10))?;
    assert_eq!(0, result.split_size);
    Ok(())
}

/// A split of an element a concurrent edit removed opens no visible
/// boundary, so it reports none.
#[test]
fn reports_no_split_size_under_a_removed_parent() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "ab")])]);
    let pos = tree.find_pos(2, true)?;

    let delete = (tree.find_pos(0, true)?, tree.find_pos(4, true)?);
    let other = TimeTicket::new(10, 0, OTHER_ACTOR);
    tree.edit_by_range_with_changes(delete, None, 0, other, None)?;
    assert_eq!("<root></root>", tree.to_xml());

    let result =
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(10), Some(&known(5)))?;
    assert_eq!("<root></root>", tree.to_xml());
    assert_eq!(0, result.split_size);
    Ok(())
}

/// JS `tree_duplicate_id_test.ts` "drops content that reuses an id from an
/// earlier change", with the size check of "does not let a dropped copy
/// widen the reverse operation".
#[test]
fn drops_content_that_reuses_an_id_from_an_earlier_change() -> Result<()> {
    let mut tree = digit_tree();
    edit_t(&mut tree, (6, 7), None, 0, ticket(4))?;
    assert_eq!("<r><p>012346789</p></r>", tree.to_xml());

    // The undo arrives as a later change, so it carries a later lamport than
    // the id its content reuses.
    let undo_at = TimeTicket::new(5, 1, ACTOR);
    let result = edit_t(
        &mut tree,
        (6, 6),
        Some(vec![TreeNode::create_text(id(3, 5), "5")]),
        0,
        undo_at,
    )?;

    assert!(
        duplicated_ids(&tree).is_empty(),
        "an id names at most one node"
    );
    assert_eq!(
        "<r><p>012346789</p></r>",
        tree.to_xml(),
        "the copy is not inserted"
    );
    assert_eq!(
        0, result.inserted_size,
        "an edit whose content was dropped inserted nothing"
    );
    assert!(result.changes.is_empty());
    assert!(result.inserted_spans.is_empty());
    Ok(())
}

/// JS `tree_duplicate_id_test.ts` "drops content whose id this edit is about
/// to create by splitting".
#[test]
fn drops_content_whose_id_this_edit_is_about_to_create_by_splitting() -> Result<()> {
    let mut tree = digit_tree();

    // Nothing has split the text node yet, so no node carries (text, 5).
    // Resolving the insert position splits it there, which creates that id
    // moments before the copy is inserted under it.
    let undo_at = TimeTicket::new(5, 1, ACTOR);
    let result = edit_t(
        &mut tree,
        (6, 6),
        Some(vec![TreeNode::create_text(id(3, 5), "5")]),
        0,
        undo_at,
    )?;

    assert!(
        duplicated_ids(&tree).is_empty(),
        "an id names at most one node"
    );
    assert_eq!("<r><p>0123456789</p></r>", tree.to_xml());
    assert_eq!(0, result.inserted_size);
    Ok(())
}

/// JS `tree_duplicate_id_test.ts` "drops content that reuses an id from
/// another actor".
#[test]
fn drops_content_that_reuses_an_id_from_another_actor() -> Result<()> {
    let mut tree = digit_tree();
    edit_t(&mut tree, (6, 7), None, 0, ticket(4))?;

    let undo_at = TimeTicket::new(5, 1, OTHER_ACTOR);
    edit_t(
        &mut tree,
        (6, 6),
        Some(vec![TreeNode::create_text(id(3, 5), "5")]),
        0,
        undo_at,
    )?;

    assert!(
        duplicated_ids(&tree).is_empty(),
        "an id names at most one node"
    );
    assert_eq!("<r><p>012346789</p></r>", tree.to_xml());
    Ok(())
}

/// JS `tree_duplicate_id_test.ts` "keeps colliding content issued by this
/// change".
#[test]
fn keeps_colliding_content_issued_by_this_change() -> Result<()> {
    let mut tree = digit_tree();

    // The delimiters an element split consumes are simulated rather than
    // replayed, so an id issued by this edit can collide with one already in
    // the tree. That content belongs to the document.
    let existing = id(2, 0);
    let edited_at = TimeTicket::new(2, 1, ACTOR);
    let result = edit_t(
        &mut tree,
        (12, 12),
        Some(vec![TreeNode::create_element(
            existing,
            "p",
            None,
            Vec::new(),
        )]),
        0,
        edited_at,
    )?;

    assert_eq!(
        "<r><p>0123456789</p><p></p></r>",
        tree.to_xml(),
        "content issued by this change is inserted even when its id collides"
    );
    assert_eq!(2, result.inserted_size);
    Ok(())
}

/// JS `tree_duplicate_id_test.ts` "refuses to split a text node past its
/// end".
#[test]
fn refuses_to_split_a_text_node_past_its_end() {
    let mut p = element(2, "p", vec![text(3, "hello")]);

    let error = p.split_text_child_at(0, 6).unwrap_err();
    assert!(
        matches!(error, YorkieError::InvalidArgument(_)),
        "unexpected error {error:?}"
    );
    assert_eq!(
        "hello",
        p.all_children()
            .next()
            .map(TreeNode::value)
            .unwrap_or_default(),
        "a refused split leaves the node alone"
    );

    // At its end there is nothing to split off.
    assert_eq!(
        crate::DataSize::default(),
        p.split_text_child_at(0, 5)
            .expect("a split at the end is a no-op")
    );
}

/// JS `CRDTTreePos.replaceNodeID` and `replaceTreeNodeID`: the match is on
/// the creation ticket alone, and the offset of the reference is kept.
#[test]
fn replaces_a_node_id_in_a_position() {
    let pos = TreePos::new(id(2, 0), id(3, 4));

    let left = pos.replace_node_id(&id(3, 0), &id(9, 7));
    assert_eq!(TreePos::new(id(2, 0), id(9, 4)), left);

    let parent = pos.replace_node_id(&id(2, 0), &id(9, 0));
    assert_eq!(TreePos::new(id(9, 0), id(3, 4)), parent);

    assert_eq!(pos, pos.replace_node_id(&id(5, 0), &id(9, 0)));
    assert_eq!(
        id(3, 4),
        replace_tree_node_id(&id(3, 4), &id(5, 0), &id(9, 0))
    );
}

/// Content inserted into a parent a concurrent edit removed is born
/// removed, which books garbage beyond the plain deletes, so the spans
/// cannot describe the edit (JS `spansComplete`, `tree.ts` 3743-3750).
#[test]
fn reports_no_spans_for_content_under_a_removed_parent() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "ab")])]);
    let pos = TreePos::new(id(2, 0), id(2, 0));

    let delete = (tree.find_pos(0, true)?, tree.find_pos(4, true)?);
    let other = TimeTicket::new(10, 0, OTHER_ACTOR);
    tree.edit_by_range_with_changes(delete, None, 0, other, None)?;
    assert_eq!("<root></root>", tree.to_xml());

    let result = tree.edit_by_range_with_changes(
        (pos.clone(), pos),
        Some(vec![text(11, "x")]),
        0,
        ticket(11),
        Some(&known(5)),
    )?;
    assert_eq!("<root></root>", tree.to_xml());
    assert_eq!(1, result.gc_pairs.len(), "the content is born removed");
    assert!(result.inserted_spans.is_empty());
    assert!(result.removed_spans.is_empty());
    Ok(())
}

/// A range boundary inside a removed text node splits it, and the piece
/// split off is born removed. Its pending GC pair is drained into the edit,
/// beyond the plain deletes, so the spans cannot describe the edit (JS
/// `spansComplete`, `tree.ts` 3868-3869).
#[test]
fn reports_no_spans_when_a_pending_gc_pair_is_drained() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "abcdef")])]);
    edit_t(&mut tree, (3, 5), None, 0, ticket(10))?;
    assert_eq!("<root><p>abef</p></root>", tree.to_xml());

    // An editor that did not see the delete removes "de": the start of its
    // range falls inside the removed "cd".
    let range = (
        TreePos::new(id(2, 0), id(3, 3)),
        TreePos::new(id(2, 0), id(3, 5)),
    );
    let other = TimeTicket::new(8, 0, OTHER_ACTOR);
    let result = tree.edit_by_range_with_changes(range, None, 0, other, Some(&known(3)))?;
    assert_eq!("<root><p>abf</p></root>", tree.to_xml());

    let pairs = result
        .gc_pairs
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        vec![id(3, 4).to_id_string(), id(3, 3).to_id_string()],
        pairs,
        "the delete of \"e\", then the drained piece \"d\""
    );
    assert!(result.removed_spans.is_empty());
    assert!(result.inserted_spans.is_empty());
    Ok(())
}

/// JS `helloTree` of `tree_restore_span_kind_test.ts` (Go
/// `createHelloTree`): `<r><p>hello</p></r>`, the root at lamport 1, `<p>`
/// at 2 and the text at 3.
fn hello_tree() -> CrdtTree {
    let tree = CrdtTree::create(
        element(1, "r", vec![element(2, "p", vec![text(3, "hello")])]),
        ticket(1),
    );
    assert_eq!("<r><p>hello</p></r>", tree.to_xml());
    tree
}

/// The span an honest replica captures for the "hello" text of
/// `hello_tree`, with no anchors (JS `textSpan`).
fn hello_span() -> TreeRestoreSpan {
    text_span(id(3, 0), "hello", id(2, 0), None, None)
}

fn restored_ids(nodes: &[TreeRestoredNode]) -> Vec<TreeNodeId> {
    nodes.iter().map(|node| node.id.clone()).collect()
}

/// Each visible edit as `(from, to, inserted_size)`.
fn edit_ranges(edits: &[TreeVisibleEdit]) -> Vec<(usize, usize, usize)> {
    edits
        .iter()
        .map(|edit| (edit.change.from, edit.change.to, edit.inserted_size))
        .collect()
}

fn pair_ids(pairs: &[(String, crate::DataSize, TimeTicket)]) -> Vec<String> {
    pairs.iter().map(|(id, _, _)| id.clone()).collect()
}

/// Reports whether a node of the tree holds exactly `id`.
fn holds(tree: &CrdtTree, id: &TreeNodeId) -> bool {
    tree.find_floor_node(id).is_some_and(|node| node.id() == id)
}

/// Purges the removed node `id`, as GC does.
fn purge(tree: &mut CrdtTree, id: &TreeNodeId) {
    assert!(
        tree.purge_gc_pair_by_id(&id.to_id_string()),
        "{} is not in the tree",
        id.to_test_string()
    );
}

/// JS `tree_restore_span_kind_test.ts` "skips a span under a text parent
/// (isText=false)" and "(isText=true)"; Go `TestTreeRestoreTextParentSkip`.
/// A text node cannot hold children and node kind never changes, so every
/// replica skips the same span.
#[test]
fn skips_a_span_under_a_text_parent() -> Result<()> {
    for is_text in [false, true] {
        let mut tree = hello_tree();
        let span = if is_text {
            text_span(id(9, 0), "x", id(3, 0), None, None)
        } else {
            element_span(id(9, 0), "p", None, id(3, 0), None, None)
        };

        let result = tree.restore(std::slice::from_ref(&span))?;
        assert!(result.untombstoned.is_empty(), "is_text={is_text}");
        assert!(
            result.recreated.is_empty(),
            "is_text={is_text}: a node under a text parent is skipped"
        );
        assert!(result.gc_pairs.is_empty(), "is_text={is_text}");
        assert_eq!("<r><p>hello</p></r>", tree.to_xml(), "is_text={is_text}");
        assert!(
            !holds(&tree, &span.id),
            "is_text={is_text}: the node is never registered"
        );
    }
    Ok(())
}

/// JS `tree_restore_span_kind_test.ts` "skips a span whose kind disagrees
/// (isText=false)" and "(isText=true)"; Go
/// `TestTreeRestoreKindMismatchSkip`. Recreating it would build a node of
/// the wrong kind, and a second copy of the span another node under the
/// same ID.
#[test]
fn skips_a_span_whose_kind_disagrees() -> Result<()> {
    for is_text in [false, true] {
        let mut tree = hello_tree();
        let span = if is_text {
            TreeRestoreSpan {
                node_type: "p".to_owned(),
                ..text_span(id(9, 0), "x", id(2, 0), None, None)
            }
        } else {
            element_span(id(9, 0), "text", None, id(2, 0), None, None)
        };

        let result = tree.restore(&[span.clone(), span])?;
        assert!(result.untombstoned.is_empty(), "is_text={is_text}");
        assert!(
            result.recreated.is_empty(),
            "is_text={is_text}: a mismatched span is skipped"
        );
        assert_eq!("<r><p>hello</p></r>", tree.to_xml(), "is_text={is_text}");
    }
    Ok(())
}

/// JS `tree_restore_span_kind_test.ts` "keeps a tombstone a mismatched span
/// names"; Go `TestTreeRestoreKindMismatchKeepsTombstone`. The kind check
/// sits before the un-tombstone branch.
#[test]
fn keeps_a_tombstone_a_mismatched_span_names() -> Result<()> {
    let mut tree = hello_tree();
    edit_t(&mut tree, (1, 6), None, 0, ticket(20))?;
    assert_eq!("<r><p></p></r>", tree.to_xml());

    // Same identity as the tombstoned text, but typed as an element.
    let span = TreeRestoreSpan {
        node_type: "p".to_owned(),
        ..hello_span()
    };
    let result = tree.restore(&[span])?;
    assert!(
        result.untombstoned.is_empty(),
        "a mismatched span revives nothing"
    );
    assert!(result.recreated.is_empty());
    assert_eq!("<r><p></p></r>", tree.to_xml(), "the tombstone stays dead");
    Ok(())
}

/// JS `tree_restore_span_kind_test.ts` "retombstone skips a mismatched
/// span: ..." (table); Go `TestTreeRetombstoneKindMismatchSkip`. The spans
/// name a live node, so retombstone would remove it without the check.
#[test]
fn retombstone_skips_a_mismatched_span() -> Result<()> {
    for (name, is_text, node_type) in [
        ("text node in an element-flagged span", false, "text"),
        ("text node typed as an element", true, "p"),
    ] {
        let mut tree = hello_tree();
        let span = TreeRestoreSpan {
            is_text,
            node_type: node_type.to_owned(),
            ..hello_span()
        };

        let result = tree.retombstone(&[span], &ticket(20))?;
        assert!(
            result.gc_pairs.is_empty(),
            "{name}: a mismatched span re-tombstones nothing"
        );
        assert_eq!(
            "<r><p>hello</p></r>",
            tree.to_xml(),
            "{name}: tree is untouched"
        );
    }
    Ok(())
}

/// JS `tree_restore_span_kind_test.ts` "skips a text span naming a live
/// element"; Go `TestTreeRestoreClaimedIdentitySkip`. The overlap lookup
/// stops at the element floor node, so the gap branch would recreate a text
/// node under `<p>`'s identity.
#[test]
fn skips_a_text_span_naming_a_live_element() -> Result<()> {
    let mut tree = hello_tree();
    let span = text_span(id(2, 0), "x", id(1, 0), None, None);

    let result = tree.restore(&[span])?;
    assert!(result.untombstoned.is_empty());
    assert!(
        result.recreated.is_empty(),
        "a claimed identity is not recreated"
    );
    assert_eq!("<r><p>hello</p></r>", tree.to_xml());
    assert_eq!(
        Some("p"),
        tree.find_floor_node(&id(2, 0)).map(TreeNode::node_type),
        "<p> still holds its own identity"
    );
    assert!(duplicated_ids(&tree).is_empty());
    Ok(())
}

/// JS `tree_restore_span_kind_test.ts` "skips an element span inside a live
/// text run"; Go `TestTreeRestoreClaimedIdentitySkip`. The ID is the one the
/// next honest split of the run mints.
#[test]
fn skips_an_element_span_inside_a_live_text_run() -> Result<()> {
    let mut tree = hello_tree();
    let span = element_span(id(3, 2), "p", None, id(2, 0), None, None);

    let result = tree.restore(&[span])?;
    assert!(result.untombstoned.is_empty());
    assert!(
        result.recreated.is_empty(),
        "an id inside a live run is not recreated"
    );
    assert_eq!("<r><p>hello</p></r>", tree.to_xml());

    // The honest split that mints that id still works afterwards.
    edit_t(&mut tree, (3, 3), Some(vec![text(21, "Z")]), 0, ticket(22))?;
    assert_eq!("<r><p>heZllo</p></r>", tree.to_xml());
    assert_eq!(8, tree.root().len(), "the size stays sound");
    assert!(duplicated_ids(&tree).is_empty());
    Ok(())
}

/// Go `TestTreeRestoreUnremove`: a tombstone still in the tree comes back in
/// place under its own identity, and the size is what it was.
#[test]
fn restores_a_tombstone_in_place() -> Result<()> {
    let mut tree = hello_tree();
    let before = tree.root().len();
    edit_t(&mut tree, (1, 6), None, 0, ticket(10))?;
    assert_eq!("<r><p></p></r>", tree.to_xml());

    let result = tree.restore(&[hello_span()])?;
    assert_eq!(vec![id(3, 0)], restored_ids(&result.untombstoned));
    assert!(result.recreated.is_empty(), "nothing was purged");
    assert_eq!("<r><p>hello</p></r>", tree.to_xml());
    assert_eq!(before, tree.root().len());
    assert_eq!(
        vec![TreeVisibleEdit {
            change: TreeEditChange {
                from: 1,
                to: 1,
                from_path: vec![0, 0],
                to_path: vec![0, 0],
                value: Some(vec![TreeNodeInput::text("hello")]),
            },
            inserted_size: 5,
        }],
        result.changes
    );
    // The revived node is measured live, the size the caller unregisters.
    assert_eq!(
        tree.find_floor_node(&id(3, 0)).map(TreeNode::data_size),
        result.untombstoned.first().map(|node| node.size)
    );
    Ok(())
}

/// Go `TestTreeRetombstoneThenRestore`: retombstone removes live nodes by
/// identity, and restoring the same spans brings them back, parent first.
/// Each change is measured when its node leaves or comes back.
#[test]
fn retombstones_and_restores_by_identity() -> Result<()> {
    let mut tree = hello_tree();
    let spans = [
        element_span(id(2, 0), "p", None, id(1, 0), None, None),
        hello_span(),
    ];

    let result = tree.retombstone(&spans, &ticket(10))?;
    assert_eq!(
        vec![id(2, 0).to_id_string(), id(3, 0).to_id_string()],
        pair_ids(&result.gc_pairs)
    );
    assert_eq!("<r></r>", tree.to_xml());
    // <p> is measured with its text while still visible; the text is then
    // under a removed ancestor and takes no room of its own.
    assert_eq!(vec![(0, 7, 0)], edit_ranges(&result.changes));
    assert_eq!(None, result.changes[0].change.value);
    assert_eq!(crate::DataSize::default(), result.diff);

    // Again: nothing is live, so nothing changes.
    let again = tree.retombstone(&spans, &ticket(11))?;
    assert!(again.gc_pairs.is_empty());
    assert!(again.changes.is_empty());

    let result = tree.restore(&spans)?;
    assert_eq!(vec![id(2, 0), id(3, 0)], restored_ids(&result.untombstoned));
    assert!(result.recreated.is_empty());
    assert_eq!("<r><p>hello</p></r>", tree.to_xml());
    assert_eq!(vec![(0, 0, 2), (1, 1, 5)], edit_ranges(&result.changes));
    assert_eq!(
        Some(vec![TreeNodeInput::element("p", Vec::new())]),
        result.changes[0].change.value,
        "<p> comes back before its text"
    );

    // Again: nothing is removed, so nothing changes.
    let again = tree.restore(&spans)?;
    assert!(again.untombstoned.is_empty());
    assert!(again.changes.is_empty());
    Ok(())
}

/// Go `TestTreeRestoreLengthCacheIntegrity`: either span order restores
/// the subtree. A child revived under a removed parent takes no room, so
/// it shows only with the parent.
#[test]
fn restores_a_subtree_in_either_span_order() -> Result<()> {
    let element = element_span(id(2, 0), "p", None, id(1, 0), None, None);
    for (name, spans, ranges) in [
        (
            "parent before child",
            [element.clone(), hello_span()],
            vec![(0, 0, 2), (1, 1, 5)],
        ),
        (
            "child before parent",
            [hello_span(), element.clone()],
            vec![(0, 0, 7)],
        ),
    ] {
        let mut tree = hello_tree();
        let before = tree.root().len();
        edit_t(&mut tree, (0, 7), None, 0, ticket(10))?;
        assert_eq!("<r></r>", tree.to_xml(), "{name}");

        let result = tree.restore(&spans)?;
        assert_eq!("<r><p>hello</p></r>", tree.to_xml(), "{name}");
        assert_eq!(before, tree.root().len(), "{name}");
        assert_eq!(ranges, edit_ranges(&result.changes), "{name}");
    }
    Ok(())
}

/// A node under a removed ancestor takes no room, so leaving or coming back
/// it reports no change; its ancestor's change covers it (JS
/// `visibleRangeOf`). The ancestor follows other content, so the node's
/// own index would look like a range.
#[test]
fn reports_no_change_under_a_removed_ancestor() -> Result<()> {
    let mut tree = tree(vec![
        element(2, "p", vec![text(3, "hello")]),
        element(4, "q", vec![text(5, "ab")]),
    ]);
    let spans = [
        element_span(id(4, 0), "q", None, id(1, 0), Some(id(2, 0)), None),
        text_span(id(5, 0), "ab", id(4, 0), None, None),
    ];

    let result = tree.retombstone(&spans, &ticket(10))?;
    assert_eq!("<root><p>hello</p></root>", tree.to_xml());
    assert_eq!(2, result.gc_pairs.len());
    assert_eq!(vec![(7, 11, 0)], edit_ranges(&result.changes));

    // Child first: the text comes back under a removed <q>.
    let result = tree.restore(&[spans[1].clone(), spans[0].clone()])?;
    assert_eq!("<root><p>hello</p><q>ab</q></root>", tree.to_xml());
    assert_eq!(vec![(7, 7, 4)], edit_ranges(&result.changes));
    Ok(())
}

/// Go `TestTreeRestoreParentGoneSkip`: a node whose parent is gone is left
/// unplaced, alike on every replica.
#[test]
fn skips_a_span_whose_parent_is_gone() -> Result<()> {
    let mut tree = hello_tree();
    let span = text_span(id(9, 0), "x", id(9, 0), None, None);

    let result = tree.restore(&[span])?;
    assert!(result.untombstoned.is_empty());
    assert!(result.recreated.is_empty());
    assert_eq!("<r><p>hello</p></r>", tree.to_xml());
    Ok(())
}

/// Go `TestTreeRestoreConvergesUnderConcurrentEdit`: a delete, its restore,
/// and a concurrent insert inside the deleted text converge in either
/// causal order, and the insert survives.
#[test]
fn restores_text_around_a_concurrent_insert() -> Result<()> {
    let deleted_at = TimeTicket::new(100, 0, "0000000000000000000000aa");
    let inserted_at = TimeTicket::new(150, 0, "0000000000000000000000bb");
    // Knows the creation of the tree, but neither editor.
    let vector = known(i64::MAX);

    let replica = || -> Result<(CrdtTree, (TreePos, TreePos))> {
        let tree = hello_tree();
        let range = (tree.find_pos(1, true)?, tree.find_pos(6, true)?);
        Ok((tree, range))
    };
    let delete = |tree: &mut CrdtTree, range: &(TreePos, TreePos)| -> Result<()> {
        tree.edit_by_range_with_changes(range.clone(), None, 0, deleted_at.clone(), Some(&vector))?;
        Ok(())
    };
    let restore = |tree: &mut CrdtTree| -> Result<()> {
        tree.restore(&[hello_span()])?;
        Ok(())
    };
    let insert = |tree: &mut CrdtTree| -> Result<()> {
        let content = TreeNode::create_text(TreeNodeId::new(inserted_at.clone(), 0), "W");
        edit_t(tree, (3, 3), Some(vec![content]), 0, inserted_at.clone())?;
        Ok(())
    };

    let (mut first, range) = replica()?;
    delete(&mut first, &range)?;
    restore(&mut first)?;
    insert(&mut first)?;

    let (mut second, range) = replica()?;
    insert(&mut second)?;
    delete(&mut second, &range)?;
    restore(&mut second)?;

    assert_eq!(first.to_xml(), second.to_xml());
    assert_eq!("<r><p>heWllo</p></r>", first.to_xml());
    assert_eq!(first.root().len(), second.root().len());
    Ok(())
}

/// Restoring the part of a removed piece a span names isolates it: the
/// removed piece is split at both ends of the span, the pieces split off
/// are born removed and come back as pending GC pairs, and only the span's
/// part is revived (JS `isolateTextRange`).
#[test]
fn restores_the_part_of_a_removed_piece_a_span_names() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "abcdef")])]);
    edit_t(&mut tree, (2, 6), None, 0, ticket(10))?;
    assert_eq!("<root><p>af</p></root>", tree.to_xml());

    let result = tree.restore(&[text_span(id(3, 2), "cd", id(2, 0), None, None)])?;
    assert_eq!("<root><p>acdf</p></root>", tree.to_xml());
    assert_eq!(vec![id(3, 2)], restored_ids(&result.untombstoned));
    assert!(result.recreated.is_empty());
    assert_eq!(
        vec![id(3, 2).to_id_string(), id(3, 4).to_id_string()],
        pair_ids(&result.gc_pairs),
        "both pieces split off the tombstone, the revived one included"
    );
    assert!(result.gc_pairs.iter().all(|(_, _, at)| at == &ticket(10)));
    assert_eq!(crate::DataSize::default(), result.diff);
    assert_eq!(vec![(2, 2, 2)], edit_ranges(&result.changes));
    Ok(())
}

/// Retombstone removes only the part of a live piece a span names,
/// splitting it at both ends and charging the splits to the diff.
#[test]
fn retombstones_the_part_of_a_live_piece_a_span_names() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "abcdef")])]);

    let span = text_span(id(3, 2), "cd", id(2, 0), None, None);
    let result = tree.retombstone(std::slice::from_ref(&span), &ticket(10))?;
    assert_eq!("<root><p>abef</p></root>", tree.to_xml());
    assert_eq!(vec![id(3, 2).to_id_string()], pair_ids(&result.gc_pairs));
    assert_ne!(crate::DataSize::default(), result.diff);
    assert_eq!(vec![(3, 5, 0)], edit_ranges(&result.changes));
    assert_eq!(vec![0, 2], result.changes[0].change.from_path);
    assert_eq!(vec![0, 4], result.changes[0].change.to_path);

    let result = tree.restore(&[span])?;
    assert_eq!("<root><p>abcdef</p></root>", tree.to_xml());
    assert_eq!(vec![id(3, 2)], restored_ids(&result.untombstoned));
    Ok(())
}

/// Restoring an element a merge removed undoes the merge's pointers: the
/// source forgets its target, and the children the merge moved forget the
/// source, so nothing is left to derive the merge from (JS
/// `dissolveMerge`). The moved children stay where the merge put them.
#[test]
fn restores_a_merged_away_element_without_its_merge() -> Result<()> {
    let mut tree = tree(vec![
        element(2, "p", vec![text(3, "ab")]),
        element(4, "p", vec![text(5, "cd")]),
    ]);
    edit_t(&mut tree, (3, 5), None, 0, ticket(10))?;
    assert_eq!("<root><p>abcd</p></root>", tree.to_xml());
    let node = |tree: &CrdtTree, id: &TreeNodeId| tree.find_floor_node(id).cloned();
    assert_eq!(
        Some(&id(2, 0)),
        node(&tree, &id(4, 0))
            .as_ref()
            .and_then(TreeNode::merged_into)
    );
    assert_eq!(
        Some(&id(4, 0)),
        node(&tree, &id(5, 0))
            .as_ref()
            .and_then(TreeNode::merged_from)
    );

    let span = element_span(id(4, 0), "p", None, id(1, 0), Some(id(2, 0)), None);
    let result = tree.restore(&[span])?;
    assert_eq!("<root><p>abcd</p><p></p></root>", tree.to_xml());
    assert_eq!(vec![id(4, 0)], restored_ids(&result.untombstoned));
    assert_eq!(vec![(6, 6, 2)], edit_ranges(&result.changes));
    let source = node(&tree, &id(4, 0)).expect("the source is in the tree");
    assert_eq!(None, source.merged_into());
    let moved = node(&tree, &id(5, 0)).expect("the moved text is in the tree");
    assert_eq!(None, moved.merged_from());
    assert_eq!(None, moved.merged_at());
    Ok(())
}

/// `<root><a/><b/><c/></root>` with `<b>` created after `<c>`, so the ID
/// order fallback would put a recreated `<b>` after both: a test that
/// places it between them proves an anchor did.
fn anchor_tree() -> CrdtTree {
    let tree = tree(vec![
        element(2, "a", Vec::new()),
        element(4, "b", Vec::new()),
        element(3, "c", Vec::new()),
    ]);
    assert_eq!("<root><a></a><b></b><c></c></root>", tree.to_xml());
    tree
}

/// A node GC purged is recreated under its own identity, right after its
/// left anchor (JS `recreateFromSpan` rung b), even when a node inserted
/// since stands between its anchors.
#[test]
fn recreates_a_purged_node_after_its_left_anchor() -> Result<()> {
    let mut tree = anchor_tree();
    let result = edit_t(&mut tree, (2, 4), None, 0, ticket(10))?;
    let spans = result.removed_spans;
    assert_eq!(
        vec![element_span(
            id(4, 0),
            "b",
            None,
            id(1, 0),
            Some(id(2, 0)),
            Some(id(3, 0))
        )],
        spans
    );
    purge(&mut tree, &id(4, 0));
    assert!(!holds(&tree, &id(4, 0)));
    edit_t(
        &mut tree,
        (2, 2),
        Some(vec![element(20, "n", Vec::new())]),
        0,
        ticket(20),
    )?;
    assert_eq!("<root><a></a><n></n><c></c></root>", tree.to_xml());

    let result = tree.restore(&spans)?;
    assert_eq!("<root><a></a><b></b><n></n><c></c></root>", tree.to_xml());
    assert!(result.untombstoned.is_empty());
    assert_eq!(vec![id(4, 0)], restored_ids(&result.recreated));
    assert!(result.gc_pairs.is_empty());
    assert_eq!(vec![(2, 2, 2)], edit_ranges(&result.changes));
    assert!(holds(&tree, &id(4, 0)), "the recreated node is registered");
    Ok(())
}

/// With its left anchor purged too, a recreated node goes right before its
/// right anchor (JS `recreateFromSpan` rung c).
#[test]
fn recreates_a_purged_node_before_its_right_anchor() -> Result<()> {
    let mut tree = anchor_tree();
    let spans = edit_t(&mut tree, (2, 4), None, 0, ticket(10))?.removed_spans;
    edit_t(&mut tree, (0, 2), None, 0, ticket(11))?;
    assert_eq!("<root><c></c></root>", tree.to_xml());
    purge(&mut tree, &id(2, 0));
    purge(&mut tree, &id(4, 0));

    let result = tree.restore(&spans)?;
    assert_eq!("<root><b></b><c></c></root>", tree.to_xml());
    assert_eq!(vec![id(4, 0)], restored_ids(&result.recreated));
    assert_eq!(vec![(0, 0, 2)], edit_ranges(&result.changes));
    Ok(())
}

/// With both anchors purged, a recreated node goes under its parent before
/// the first child whose ID is greater than its own, a slot every replica
/// computes alike (JS `recreateFromSpan` rung d).
#[test]
fn recreates_a_purged_node_in_id_order_without_anchors() -> Result<()> {
    let mut tree = tree(vec![
        element(2, "x", Vec::new()),
        element(3, "a", Vec::new()),
        element(5, "b", Vec::new()),
        element(4, "c", Vec::new()),
    ]);
    let spans = edit_t(&mut tree, (4, 6), None, 0, ticket(10))?.removed_spans;
    edit_t(&mut tree, (2, 6), None, 0, ticket(11))?;
    assert_eq!("<root><x></x></root>", tree.to_xml());
    for lamport in [3, 4, 5] {
        purge(&mut tree, &id(lamport, 0));
    }
    edit_t(
        &mut tree,
        (2, 2),
        Some(vec![element(20, "d", Vec::new())]),
        0,
        ticket(20),
    )?;
    assert_eq!("<root><x></x><d></d></root>", tree.to_xml());

    let result = tree.restore(&spans)?;
    assert_eq!("<root><x></x><b></b><d></d></root>", tree.to_xml());
    assert_eq!(vec![id(5, 0)], restored_ids(&result.recreated));
    assert_eq!(vec![(2, 2, 2)], edit_ranges(&result.changes));
    Ok(())
}

/// A purged gap inside a text span is recreated in the exact slot of its
/// insertion, next to the surviving piece that follows it, ahead of the
/// span's anchors: the left anchor names the piece before the whole span,
/// not before the gap (JS `recreateFromSpan` rung a).
#[test]
fn recreates_a_purged_gap_of_a_text_span() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "abcdef")])]);
    edit_t(&mut tree, (2, 3), None, 0, ticket(10))?;
    edit_t(&mut tree, (2, 4), None, 0, ticket(11))?;
    edit_t(&mut tree, (2, 3), None, 0, ticket(12))?;
    assert_eq!("<root><p>af</p></root>", tree.to_xml());
    purge(&mut tree, &id(3, 2));

    let span = text_span(id(3, 1), "bcde", id(2, 0), Some(id(3, 0)), Some(id(3, 5)));
    let result = tree.restore(&[span])?;
    assert_eq!("<root><p>abcdef</p></root>", tree.to_xml());
    assert_eq!(vec![id(3, 1), id(3, 4)], restored_ids(&result.untombstoned));
    assert_eq!(vec![id(3, 2)], restored_ids(&result.recreated));
    assert_eq!(
        vec![(2, 2, 1), (3, 3, 2), (5, 5, 1)],
        edit_ranges(&result.changes)
    );
    Ok(())
}

/// A node recreated under a parent removed since is born removed with the
/// parent's removal ticket: it shows in no change and is not reported as
/// recreated, and its GC pair is pending for the caller to register.
#[test]
fn recreates_a_purged_node_under_a_removed_parent_as_removed() -> Result<()> {
    let mut tree = tree(vec![element(2, "p", vec![text(3, "ab")])]);
    let spans = edit_t(&mut tree, (1, 3), None, 0, ticket(10))?.removed_spans;
    purge(&mut tree, &id(3, 0));
    edit_t(&mut tree, (0, 2), None, 0, ticket(11))?;
    assert_eq!("<root></root>", tree.to_xml());

    let result = tree.restore(&spans)?;
    assert_eq!("<root></root>", tree.to_xml());
    assert!(result.recreated.is_empty());
    assert!(result.changes.is_empty());
    assert_eq!(vec![id(3, 0).to_id_string()], pair_ids(&result.gc_pairs));
    assert_eq!(
        Some(&ticket(11)),
        result.gc_pairs.first().map(|(_, _, at)| at)
    );
    let node = tree.find_floor_node(&id(3, 0)).expect("the node is placed");
    assert_eq!(Some(&ticket(11)), node.removed_at());
    assert_eq!(
        Some(node.data_size()),
        result.gc_pairs.first().map(|(_, size, _)| *size)
    );
    Ok(())
}
