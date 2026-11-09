//! CRDT-level tests of what a Tree edit reports for its reverse: the restore
//! spans of the nodes it removed and inserted, the pre-tombstoned set, the
//! split size, and the content it accepted.
//!
//! The span cases follow JS `CRDTTree.edit` (`src/document/crdt/tree.ts` at
//! `8629993e`, span capture and `spansComplete`); the duplicate-ID cases port
//! JS `test/unit/document/crdt/tree_duplicate_id_test.ts`.

use super::rht::Rht;
use super::tree::{
    replace_tree_node_id, CrdtTree, TreeEditResult, TreeNode, TreeNodeId, TreePos, TreeRestoreSpan,
};
use crate::{Result, TimeTicket, VersionVector, YorkieError};
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
