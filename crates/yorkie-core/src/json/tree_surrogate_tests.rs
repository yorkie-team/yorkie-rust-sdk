//! Tree indexes inside a UTF-16 surrogate pair.
//!
//! Ports the Tree cases of JS `test/unit/document/surrogate_index_test.ts`
//! (`6932da6a`, yorkie-js-sdk#1447, the JS half of yorkie#2085); the test
//! names follow the JS ones. Tree indexes count UTF-16 code units, so the
//! index between the two halves of a non-BMP character names no character
//! boundary. A local index there is rejected where it becomes a CRDT
//! position (`CrdtTree::find_pos`); indexes the document computes itself go
//! through `find_pos_unchecked`.
//!
//! Not ported:
//! - "Reject lone-surrogate content": a Rust `&str` cannot hold a lone half.
//! - The Text cases: `json/text.rs` ports them.
//! - "applies a remote mid-pair operation from an older client", "undoes and
//!   redoes around an intact pair", "runs a reconciled undo whose index lands
//!   inside a pair" and the undo/redo half of "discards earlier edits of the
//!   rejected update": Tree undo/redo is not ported.
//! - "styles a single path whose derived end lands inside a pair" lives in
//!   `tree_style_path_tests.rs`; "builds reverse operations at indexes inside
//!   a pair" in `operation/tree_edit_operation.rs`.

use crate::change::{ChangePack, Checkpoint};
use crate::wire::WireChangePack;
use crate::{Document, JsonTree, JsonValue, Result, TreeNodeInput, VersionVector, YorkieError};
use serde_json::Value;
use std::collections::BTreeMap;

/// One non-BMP character followed by a BMP one: 2 characters, 3 UTF-16 code
/// units. Tree index 2 is the only position that cuts the emoji in half.
const SURROGATE_TEXT: &str = "😀x";

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

fn surrogate_tree() -> TreeNodeInput {
    TreeNodeInput::element(
        "r",
        vec![TreeNodeInput::element("p", vec![text(SURROGATE_TEXT)])],
    )
}

fn new_actor(actor: &str) -> Document {
    let mut doc = Document::new("surrogate-index");
    doc.set_actor(actor);
    doc
}

/// JS `newSurrogateDoc`, Tree half: `<r><p>😀x</p></r>` under `t`.
fn new_surrogate_doc() -> Result<Document> {
    let mut doc = new_actor("000000000000000000000001");
    doc.update(|root| root.set_tree("t", surrogate_tree()).map(|_| ()))?;
    Ok(doc)
}

fn xml(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().expect("a document tree"),
        other => panic!("expected tree, got {other:?}"),
    }
}

fn update_tree(doc: &mut Document, f: impl FnOnce(&mut JsonTree) -> Result<()>) -> Result<()> {
    doc.update(|root| f(root.get_tree_mut("t")?))
}

fn mid_pair() -> YorkieError {
    YorkieError::InvalidArgument("index must not split a UTF-16 surrogate pair".to_owned())
}

/// JS `assertRejectsMidSurrogate`: `f` is refused and leaves the document
/// unchanged.
fn assert_rejects_mid_surrogate(doc: &mut Document, f: impl FnOnce(&mut JsonTree) -> Result<()>) {
    let before = doc.to_sorted_json();
    assert_eq!(Err(mid_pair()), update_tree(doc, f));
    assert_eq!(
        before,
        doc.to_sorted_json(),
        "the refused edit left no trace"
    );
}

fn bold() -> BTreeMap<String, Value> {
    BTreeMap::from([("bold".to_owned(), Value::from("true"))])
}

/// JS "Reject mid-surrogate-pair indexes", the Tree cases. JS `edit` and
/// `editBulk` (and their path forms) are one Rust method each. The last
/// component of a path into a text node is a UTF-16 offset, so `[0, 1]` is
/// offset 1 of the text under the first `<p>`.
#[test]
fn rejects_mid_surrogate_pair_tree_indexes() -> Result<()> {
    type Case = (&'static str, fn(&mut JsonTree) -> Result<()>);
    let cases: [Case; 7] = [
        ("Tree.edit", |t| t.edit(2, 2, vec![text("y")], 0)),
        ("Tree.style", |t| t.style(1, 2, bold())),
        ("Tree.removeStyle", |t| t.remove_style(1, 2, ["bold"])),
        ("Tree.editByPath", |t| {
            t.edit_by_path(&[0, 1], &[0, 1], vec![text("y")], 0)
        }),
        ("Tree.styleByPath", |t| {
            t.style_by_path_range(&[0, 0], &[0, 1], bold())
        }),
        ("Tree.removeStyleByPath", |t| {
            t.remove_style_by_path(&[0, 0], &[0, 1], ["bold"])
        }),
        ("Tree.splitByPath", |t| t.split_by_path(&[0, 1])),
    ];
    for (name, f) in cases {
        let mut doc = new_surrogate_doc()?;
        let before = doc.to_sorted_json();
        assert_eq!(Err(mid_pair()), update_tree(&mut doc, f), "{name}");
        assert_eq!(before, doc.to_sorted_json(), "{name}");
    }
    Ok(())
}

/// JS "keeps valid Tree boundaries editable".
#[test]
fn keeps_valid_tree_boundaries_editable() -> Result<()> {
    for (index, expected) in [
        (1, "<r><p>y😀x</p></r>"),
        (3, "<r><p>😀yx</p></r>"),
        (4, "<r><p>😀xy</p></r>"),
    ] {
        let mut doc = new_surrogate_doc()?;
        update_tree(&mut doc, |t| t.edit(index, index, vec![text("y")], 0))?;
        assert_eq!(expected, xml(&doc), "index {index}");
    }
    Ok(())
}

/// JS "rejects a Tree index at a local seam inside a pair", first half:
/// converting a position back to an index splits the text node there, with
/// no operation, and an edit at that seam is still refused.
///
/// In JS that split stays on the document's tree, and the guard reads the
/// neighbouring node to see the pair across the seam. A Rust `JsonTree` is
/// a copy whose split never reaches the document, so the edit meets the
/// pair inside one node. The update records nothing, as in JS, so the
/// document keeps the pair whole.
#[test]
fn rejects_a_tree_index_at_a_local_seam_inside_a_pair() -> Result<()> {
    let mut doc = new_surrogate_doc()?;
    update_tree(&mut doc, |t| {
        let mid_pair = t.index_range_to_pos_range((2, 2))?;
        assert_eq!((2, 2), t.pos_range_to_index_range(&mid_pair)?);
        Ok(())
    })?;

    assert_rejects_mid_surrogate(&mut doc, |t| t.edit(2, 2, vec![text("y")], 0));
    Ok(())
}

/// JS "rejects a Tree index at a local seam inside a pair", second half:
/// the boundaries around the seam stay editable. The conversion runs in the
/// update that creates the tree, which records an operation, so the split
/// stays on the copy.
#[test]
fn keeps_the_boundaries_around_a_local_seam_editable() -> Result<()> {
    for (index, expected) in [(1, "<r><p>y😀x</p></r>"), (3, "<r><p>😀yx</p></r>")] {
        let mut clone = Document::new("surrogate-index");
        clone.update(|root| {
            let t = root.set_tree("t", surrogate_tree())?;
            let range = t.index_range_to_pos_range((2, 2))?;
            t.pos_range_to_index_range(&range)?;
            Ok(())
        })?;
        update_tree(&mut clone, |t| t.edit(index, index, vec![text("y")], 0))?;
        assert_eq!(expected, xml(&clone), "index {index}");
    }
    Ok(())
}

/// JS "resolves Tree indexes after a split": an earlier edit splits `<p>`'s
/// text node, so every offset is resolved relative to the node holding it.
#[test]
fn resolves_tree_indexes_after_a_split() -> Result<()> {
    let split_doc = || -> Result<Document> {
        let mut doc = new_surrogate_doc()?;
        update_tree(&mut doc, |t| t.edit(3, 3, vec![text("yz")], 0))?;
        Ok(doc)
    };

    assert_rejects_mid_surrogate(&mut split_doc()?, |t| t.edit(2, 2, vec![text("w")], 0));

    // The seam the split created (3) and the offsets inside the inserted
    // node (4, 5) are whole-character boundaries.
    for (index, expected) in [
        (3, "<r><p>😀wyzx</p></r>"),
        (4, "<r><p>😀ywzx</p></r>"),
        (5, "<r><p>😀yzwx</p></r>"),
    ] {
        let mut doc = split_doc()?;
        update_tree(&mut doc, |t| t.edit(index, index, vec![text("w")], 0))?;
        assert_eq!(expected, xml(&doc), "index {index}");
    }
    Ok(())
}

/// JS "discards earlier edits of the rejected update", Tree case, without
/// the undo/redo that follows it in JS (Tree undo is not ported).
#[test]
fn discards_earlier_edits_of_the_rejected_update() -> Result<()> {
    let mut doc = new_surrogate_doc()?;
    let original = xml(&doc);

    assert_rejects_mid_surrogate(&mut doc, |t| {
        t.edit(4, 4, vec![text("z")], 0)?;
        t.edit(2, 2, vec![text("z")], 0)
    });
    assert_eq!(original, xml(&doc));

    update_tree(&mut doc, |t| t.edit(4, 4, vec![text("z")], 0))?;
    assert_eq!("<r><p>😀xz</p></r>", xml(&doc));
    Ok(())
}

/// JS `grab`: takes the pending local changes through the wire and acks
/// them.
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

/// JS `newSectionReplicas`: two replicas holding
/// `<r><section><p>a😀b</p><p>c😀d</p></section></r>`.
fn new_section_replicas() -> Result<(Document, Document)> {
    let mut a = new_actor("000000000000000000000002");
    let mut b = new_actor("000000000000000000000001");
    a.update(|root| {
        root.set_tree(
            "t",
            TreeNodeInput::element(
                "r",
                vec![TreeNodeInput::element(
                    "section",
                    vec![
                        TreeNodeInput::element("p", vec![text("a😀b")]),
                        TreeNodeInput::element("p", vec![text("c😀d")]),
                    ],
                )],
            ),
        )
        .map(|_| ())
    })?;
    exchange(&mut a, &mut b)?;
    Ok((a, b))
}

/// JS `exchange`.
fn exchange(a: &mut Document, b: &mut Document) -> Result<()> {
    let pack_a = grab(a)?;
    let pack_b = grab(b)?;
    feed(a, &pack_b)?;
    feed(b, &pack_a)
}

fn edit(doc: &mut Document, from: usize, to: usize, value: Option<&str>) -> Result<()> {
    update_tree(doc, |t| {
        t.edit(from, to, value.map(text).into_iter().collect(), 0)
    })
}

/// JS "applies a remote change whose reverse lands inside a pair": applying
/// a change builds its reverse too, from indexes the receiver computes on
/// its own tree. On A the reverse of B's emoji insert ends inside one of A's
/// emojis; checking that index would refuse the remote change.
#[test]
fn applies_a_remote_change_whose_reverse_lands_inside_a_pair() -> Result<()> {
    let (mut a, mut b) = new_section_replicas()?;

    edit(&mut b, 1, 2, Some("😀"))?;
    edit(&mut a, 11, 11, Some("x"))?;
    edit(&mut a, 0, 2, Some("x"))?;
    edit(&mut a, 7, 7, Some("😀"))?;
    exchange(&mut a, &mut b)?;

    assert_eq!(xml(&a), xml(&b));
    assert_eq!(a.to_sorted_json(), b.to_sorted_json());
    Ok(())
}
