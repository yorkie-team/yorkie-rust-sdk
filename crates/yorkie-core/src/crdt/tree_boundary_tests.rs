//! Tree edits at an element/text boundary.
//!
//! Ports JS `test/unit/document/tree_element_text_boundary_test.ts`
//! (`8ad8ce77`, yorkie-js-sdk#1482), itself a port of yorkie's
//! `pkg/document/crdt/tree_find_pos_test.go` and
//! `pkg/document/tree_element_text_boundary_test.go` (yorkie#2079). The test
//! names follow the JS ones.
//!
//! The index right after an element and right before a text sibling (index 8
//! below) resolves to the start of that text. `find_pos` used to anchor that
//! index on the text itself at offset 0. A text left sibling means "after its
//! first N characters", and with N = 0 there is nothing to split off, so the
//! whole text became the left sibling and the edit landed after it. It now
//! anchors on what precedes the text.
//!
//! ```text
//!     0   1   2 3 4 5 6 7    8 9 10 11 12 13 14    15
//! <r> <p> <b> h e l l o </b>   w  o  r  l  d   </p>  </r>
//! ```
//!
//! Where JS passes a change pack through protobuf, these pass it through the
//! `wire` projection `yorkie-protocol` encodes from.

use super::tree::{CrdtTree, TreeNode, TreeNodeId};
use crate::change::{ChangePack, Checkpoint};
use crate::wire::WireChangePack;
use crate::{Document, JsonTree, JsonValue, Result, TimeTicket, TreeNodeInput, VersionVector};
use serde_json::Value;
use std::collections::BTreeMap;

/// JS `timeT` and `posT`: tickets of one actor, one lamport after another.
#[derive(Default)]
struct Clock(i64);

impl Clock {
    fn tick(&mut self) -> TimeTicket {
        self.0 += 1;
        TimeTicket::new(self.0, 0, "000000000000000000000000")
    }

    fn element(&mut self, node_type: &str) -> TreeNode {
        TreeNode::create_element(TreeNodeId::new(self.tick(), 0), node_type, None, Vec::new())
    }

    /// JS `textNode`.
    fn text(&mut self, value: &str) -> TreeNode {
        TreeNode::create_text(TreeNodeId::new(self.tick(), 0), value)
    }
}

/// JS `editT`: replaces `from..to` with `node`, or deletes it when `node` is
/// `None`.
fn edit_t(
    tree: &mut CrdtTree,
    clock: &mut Clock,
    (from, to): (usize, usize),
    node: Option<TreeNode>,
    split_level: usize,
) -> Result<()> {
    let range = (tree.find_pos(from, true)?, tree.find_pos(to, true)?);
    tree.edit_by_range_with_changes(
        range,
        node.map(|node| vec![node]),
        split_level,
        clock.tick(),
        None,
    )?;
    Ok(())
}

/// JS `newBoldHelloWorldTree`: `<r><p><b>hello</b> world</p></r>`.
fn new_bold_hello_world_tree() -> Result<(CrdtTree, Clock)> {
    let mut clock = Clock::default();
    let root = clock.element("r");
    let mut tree = CrdtTree::create(root, clock.tick());
    let p = clock.element("p");
    edit_t(&mut tree, &mut clock, (0, 0), Some(p), 0)?;
    let b = clock.element("b");
    edit_t(&mut tree, &mut clock, (1, 1), Some(b), 0)?;
    let hello = clock.text("hello");
    edit_t(&mut tree, &mut clock, (2, 2), Some(hello), 0)?;
    // <p> has no text after </b> yet.
    let world = clock.text(" world");
    edit_t(&mut tree, &mut clock, (8, 8), Some(world), 0)?;
    assert_eq!("<r><p><b>hello</b> world</p></r>", tree.to_xml());
    Ok((tree, clock))
}

/// JS `childrenOfP`: the children of the first `<p>`, tombstones included,
/// as `b`, `"text"` or `~"removed text"~`.
fn children_of_p(tree: &CrdtTree) -> Vec<String> {
    let p = tree.root().children().next().expect("a <p>");
    p.all_children()
        .map(|node| {
            let name = if node.is_text() {
                format!("\"{}\"", node.value())
            } else {
                node.node_type().to_owned()
            };
            if node.is_removed() {
                format!("~{name}~")
            } else {
                name
            }
        })
        .collect()
}

/// JS "inserts before the text".
#[test]
fn inserts_before_the_text() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    let bar = clock.text("|");
    edit_t(&mut tree, &mut clock, (8, 8), Some(bar), 0)?;
    assert_eq!("<r><p><b>hello</b>| world</p></r>", tree.to_xml());
    Ok(())
}

/// JS "deletes the text from the boundary".
#[test]
fn deletes_the_text_from_the_boundary() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    edit_t(&mut tree, &mut clock, (8, 14), None, 0)?;
    assert_eq!("<r><p><b>hello</b></p></r>", tree.to_xml());
    Ok(())
}

/// JS "skips a tombstone between the element and the text": with
/// `<p><b>hello</b>[x removed] world</p>`, the previous visible sibling of
/// `" world"` is still `<b>`.
#[test]
fn skips_a_tombstone_between_the_element_and_the_text() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    let x = clock.text("x");
    edit_t(&mut tree, &mut clock, (8, 8), Some(x), 0)?;
    edit_t(&mut tree, &mut clock, (8, 9), None, 0)?;
    assert_eq!("<r><p><b>hello</b> world</p></r>", tree.to_xml());

    let bar = clock.text("|");
    edit_t(&mut tree, &mut clock, (8, 8), Some(bar), 0)?;
    assert_eq!("<r><p><b>hello</b>| world</p></r>", tree.to_xml());
    Ok(())
}

/// JS "skips a deleted head of the same text like a separate tombstone":
/// deleting `" w"` splits `" world"` into a tombstoned head and a live
/// `"orld"`. The insert still anchors on `<b>`, before the tombstone, exactly
/// as it does when the tombstone is a separate node.
#[test]
fn skips_a_deleted_head_of_the_same_text_like_a_separate_tombstone() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    edit_t(&mut tree, &mut clock, (8, 10), None, 0)?;
    assert_eq!("<r><p><b>hello</b>orld</p></r>", tree.to_xml());

    let bar = clock.text("|");
    edit_t(&mut tree, &mut clock, (8, 8), Some(bar), 0)?;
    assert_eq!("<r><p><b>hello</b>|orld</p></r>", tree.to_xml());
    assert_eq!(
        vec!["b", "\"|\"", "~\" w\"~", "\"orld\""],
        children_of_p(&tree)
    );

    let (mut separate, mut clock) = new_bold_hello_world_tree()?;
    let x = clock.text("x");
    edit_t(&mut separate, &mut clock, (8, 8), Some(x), 0)?;
    edit_t(&mut separate, &mut clock, (8, 9), None, 0)?;
    let bar = clock.text("|");
    edit_t(&mut separate, &mut clock, (8, 8), Some(bar), 0)?;
    assert_eq!(
        vec!["b", "\"|\"", "~\"x\"~", "\" world\""],
        children_of_p(&separate)
    );
    Ok(())
}

/// JS "anchors a text that is the first visible child on the parent": with
/// `<p>[<b>hello</b> removed] world</p>`, `" world"` is the first visible
/// child even though a tombstone precedes it.
#[test]
fn anchors_a_text_that_is_the_first_visible_child_on_the_parent() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    edit_t(&mut tree, &mut clock, (1, 8), None, 0)?;
    assert_eq!("<r><p> world</p></r>", tree.to_xml());

    let pos = tree.find_pos(1, true)?;
    assert_eq!(
        pos.parent_id().created_at(),
        pos.left_sibling_id().created_at()
    );

    let bar = clock.text("|");
    edit_t(&mut tree, &mut clock, (1, 1), Some(bar), 0)?;
    assert_eq!("<r><p>| world</p></r>", tree.to_xml());
    Ok(())
}

/// JS "anchors on a split element before the text itself": splitting `<b>`
/// at `hel|lo` gives the right half an `insPrev`. Element IDs always carry
/// offset 0, so the anchor stays on the right half.
///
/// ```text
///     0   1   2 3 4 5    6   7 8 9    10 11 ...
/// <r> <p> <b> h e l </b> <b> l o </b>    w ...
/// ```
#[test]
fn anchors_on_a_split_element_before_the_text_itself() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    edit_t(&mut tree, &mut clock, (5, 5), None, 1)?;
    assert_eq!("<r><p><b>hel</b><b>lo</b> world</p></r>", tree.to_xml());

    let p = tree.root().children().next().expect("a <p>");
    let right_half = p.children().nth(1).expect("the right half of <b>");
    assert!(right_half.ins_prev_id().is_some());
    let right_half_id = right_half.id().clone();
    let pos = tree.find_pos(10, true)?;
    assert_eq!(&right_half_id, pos.left_sibling_id());

    let bar = clock.text("|");
    edit_t(&mut tree, &mut clock, (10, 10), Some(bar), 0)?;
    assert_eq!("<r><p><b>hel</b><b>lo</b>| world</p></r>", tree.to_xml());
    Ok(())
}

/// JS "resolves a boundary between two texts to the end of the left one":
/// a text reached at offset 0 never has a text right before it, so its
/// previous visible sibling is an element.
#[test]
fn resolves_a_boundary_between_two_texts_to_the_end_of_the_left_one() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    let bang = clock.text("!");
    edit_t(&mut tree, &mut clock, (14, 14), Some(bang), 0)?;
    assert_eq!("<r><p><b>hello</b> world!</p></r>", tree.to_xml());

    let p = tree.root().children().next().expect("a <p>");
    let world = p.children().nth(1).expect("the text after </b>");
    assert_eq!(" world", world.value());
    let world_id = world.id().clone();
    let pos = tree.find_pos(14, true)?;
    assert_eq!(world_id.created_at(), pos.left_sibling_id().created_at());
    assert_eq!(
        world_id.offset() + " world".len(),
        pos.left_sibling_id().offset()
    );
    Ok(())
}

/// JS "moves the text to the new paragraph when splitting at the boundary".
#[test]
fn moves_the_text_to_the_new_paragraph_when_splitting_at_the_boundary() -> Result<()> {
    let (mut tree, mut clock) = new_bold_hello_world_tree()?;
    edit_t(&mut tree, &mut clock, (8, 8), None, 1)?;
    assert_eq!("<r><p><b>hello</b></p><p> world</p></r>", tree.to_xml());
    Ok(())
}

/// JS `newActor`.
fn new_actor(actor: &str) -> Document {
    let mut doc = Document::new("d");
    doc.set_actor(actor);
    doc
}

/// JS `grab`: takes the pending local changes of `doc` through the wire and
/// acknowledges them.
fn grab(doc: &mut Document) -> Result<ChangePack> {
    let pack = doc.create_change_pack();
    let last_seq = pack
        .changes()
        .last()
        .map_or(0, |change| change.id().client_seq());
    let wire = ChangePack::try_from(WireChangePack::try_from(&pack)?)?;
    doc.apply_change_pack(&ChangePack::create(
        doc.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))?;
    Ok(wire)
}

/// JS `feed`.
fn feed(doc: &mut Document, batch: &ChangePack) -> Result<()> {
    doc.apply_change_pack(&ChangePack::create(
        batch.document_key(),
        Checkpoint::new(0, 0),
        false,
        batch.changes().to_vec(),
        Some(VersionVector::new()),
        None,
    ))
}

fn tree_xml(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().expect("a document tree"),
        other => panic!("expected tree, got {other:?}"),
    }
}

type TreeEdit = fn(&mut JsonTree) -> Result<()>;

/// JS `runConcurrently`: seeds `<r><p><b>hello</b> world</p></r>` on two
/// replicas, runs `edit` on the first and `other` on the second without
/// either seeing the other, exchanges the changes and returns both results.
fn run_concurrently(edit: TreeEdit, other: TreeEdit) -> Result<(String, String)> {
    let mut seed = new_actor("000000000000000000000009");
    seed.update(|root| {
        root.set_tree(
            "t",
            TreeNodeInput::element(
                "r",
                vec![TreeNodeInput::element(
                    "p",
                    vec![
                        TreeNodeInput::element("b", vec![TreeNodeInput::text("hello")]),
                        TreeNodeInput::text(" world"),
                    ],
                )],
            ),
        )
        .map(|_| ())
    })?;
    let base = grab(&mut seed)?;

    let mut editor = new_actor("00000000000000000000000a");
    let mut second = new_actor("00000000000000000000000b");
    feed(&mut editor, &base)?;
    feed(&mut second, &base)?;

    editor.update(|root| edit(root.get_tree_mut("t")?))?;
    second.update(|root| other(root.get_tree_mut("t")?))?;

    let edits = grab(&mut editor)?;
    let others = grab(&mut second)?;
    feed(&mut editor, &others)?;
    feed(&mut second, &edits)?;

    Ok((tree_xml(&editor), tree_xml(&second)))
}

fn split_b(tree: &mut JsonTree) -> Result<()> {
    // <b>hel</b><b>lo</b>
    tree.edit(5, 5, Vec::new(), 1)
}

fn insert_bar(tree: &mut JsonTree) -> Result<()> {
    tree.edit(8, 8, vec![TreeNodeInput::text("|")], 0)
}

/// JS "Tree element/text boundary under a concurrent split": an index right
/// after an element and right before a text sibling anchors on the element.
/// On a replica that has split that element concurrently, the anchor has to
/// move past the split half the editing change did not know, or the edit
/// lands between the two halves instead of before the text.
#[test]
fn boundary_under_a_concurrent_split() -> Result<()> {
    let cases: [(&str, TreeEdit, &str); 4] = [
        (
            "insert before the text",
            insert_bar,
            "<r><p><b>hel</b><b>lo</b>| world</p></r>",
        ),
        (
            "delete the text from the boundary",
            |tree| tree.edit(8, 14, Vec::new(), 0),
            "<r><p><b>hel</b><b>lo</b></p></r>",
        ),
        (
            "split the paragraph at the boundary",
            |tree| tree.edit(8, 8, Vec::new(), 1),
            "<r><p><b>hel</b><b>lo</b></p><p> world</p></r>",
        ),
        (
            "style a range that ends at the boundary",
            |tree| {
                let attrs = BTreeMap::from([("k".to_owned(), Value::from("v"))]);
                tree.style(1, 8, attrs)
            },
            r#"<r><p><b k="v">hel</b><b k="v">lo</b> world</p></r>"#,
        ),
    ];

    for (name, edit, want) in cases {
        let (editor, splitter) = run_concurrently(edit, split_b)?;
        assert_eq!(want, editor, "{name}: editor");
        assert_eq!(want, splitter, "{name}: splitter");
    }
    Ok(())
}

/// JS "Tree element/text boundary under a concurrent edit": the element
/// anchor must also hold up when another replica removes the element the
/// edit anchored on, or inserts at the same index, so two edits share the
/// anchor and the RGA tie-break orders them.
#[test]
fn boundary_under_a_concurrent_edit() -> Result<()> {
    let cases: [(&str, TreeEdit, &str); 2] = [
        (
            "the anchor element is removed",
            |tree| tree.edit(1, 8, Vec::new(), 0),
            "<r><p>| world</p></r>",
        ),
        (
            "another insert at the same boundary",
            |tree| tree.edit(8, 8, vec![TreeNodeInput::text("x")], 0),
            "<r><p><b>hello</b>x| world</p></r>",
        ),
    ];

    for (name, other, want) in cases {
        let (editor, other) = run_concurrently(insert_bar, other)?;
        assert_eq!(want, editor, "{name}: editor");
        assert_eq!(want, other, "{name}: other");
    }
    Ok(())
}
