//! Byte-level interop of Tree undo with the JS SDK.
//!
//! The fixtures under `tests/fixtures/tree_undo` were written by the JS SDK
//! at `8629993e` with `tools/interop/tree_undo_fixtures_test.ts`; its header
//! has the exact command. Each `.hex` file is a protobuf `ChangePack` of one
//! replica's changes, and each `.json` file the XML JS read after each
//! change.
//!
//! The bytes are compared change by change, from the second change on. The
//! first change sets the Tree, and Rust does not write a container value
//! of a Set the way JS does, Tree undo aside: JS writes the value's
//! `moved_at` inside the nested `JSONElement` bytes only, while Rust writes
//! it on the `JSONElementSimple` too when it re-encodes a decoded value,
//! and nowhere when it writes a value of its own. That gap is older than
//! Tree undo and does not change what a replica builds.

use std::collections::BTreeMap;
use std::error::Error;

use prost::Message;
use serde_json::Value;
use yorkie_core::wire::{
    WireChangePack, WireOperation, WireRestoreMode, WireTreeNodeId, WireTreeRestoreSpan,
};
use yorkie_core::{ChangePack, Checkpoint, Document, JsonValue, TimeTicket, TreeNodeInput};
use yorkie_protocol::converter::{decode_change_pack, encode_change_pack};
use yorkie_protocol::yorkie::v1 as api;

type TestResult = Result<(), Box<dyn Error>>;

const A1: &str = "000000000000000000000001";
const A2: &str = "000000000000000000000002";
const KEY: &str = "tree-undo-interop";

fn fixture_path(name: &str) -> String {
    format!(
        "{}/tests/fixtures/tree_undo/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn fixture_bytes(name: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let hex = std::fs::read_to_string(fixture_path(&format!("{name}.hex")))?;
    let hex = hex.trim();
    (0..hex.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&hex[i..i + 2], 16)?))
        .collect()
}

/// The XML JS read after each change of the pack `name`.
fn fixture_xmls(name: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let json = std::fs::read_to_string(fixture_path(&format!("{name}.json")))?;
    Ok(serde_json::from_str(&json)?)
}

fn xml_of(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().expect("a document tree"),
        other => panic!("expected a tree at t, got {other:?}"),
    }
}

fn ticket(lamport: i64, delimiter: u32) -> TimeTicket {
    TimeTicket::new(lamport, delimiter, A1)
}

fn node_id(lamport: i64, delimiter: u32, offset: usize) -> WireTreeNodeId {
    WireTreeNodeId {
        created_at: ticket(lamport, delimiter),
        offset,
    }
}

fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

/// Asserts that the packs `want` (JS) and `got` (Rust) hold the same
/// number of changes and the same bytes for every change after the first,
/// which sets the Tree (see the module docs).
fn assert_same_edit_changes(name: &str, want: &[u8], got: &[u8]) -> TestResult {
    let changes = |bytes: &[u8]| -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
        Ok(api::ChangePack::decode(bytes)?
            .changes
            .iter()
            .map(Message::encode_to_vec)
            .collect())
    };
    let (want, got) = (changes(want)?, changes(got)?);
    assert_eq!(want.len(), got.len(), "{name}: change count");
    for (index, (want, got)) in want.iter().zip(&got).enumerate().skip(1) {
        assert_eq!(want, got, "{name}: change {index}");
    }
    Ok(())
}

/// Decodes the pack `name` and checks that its Tree edit changes encode
/// back to the bytes JS wrote.
fn reencoded(name: &str) -> Result<WireChangePack, Box<dyn Error>> {
    let bytes = fixture_bytes(name)?;
    let pack = decode_change_pack(&bytes)?;
    assert_same_edit_changes(name, &bytes, &encode_change_pack(&pack)?)?;
    Ok(WireChangePack::try_from(&pack)?)
}

/// The only operation of the change at `index`, a Tree edit.
fn tree_edit_of(pack: &WireChangePack, index: usize) -> &WireOperation {
    match &pack.changes[index].operations[..] {
        [operation @ WireOperation::TreeEdit { .. }] => operation,
        other => panic!("expected one tree edit in change {index}, got {other:?}"),
    }
}

type Identity<'a> = (
    Option<WireRestoreMode>,
    &'a [WireTreeRestoreSpan],
    &'a [WireTreeRestoreSpan],
);

/// The mode and spans of the Tree edit of the change at `index`.
fn identity_of(pack: &WireChangePack, index: usize) -> Identity<'_> {
    let WireOperation::TreeEdit {
        contents,
        restore_spans,
        restore_mode,
        retombstone_spans,
        ..
    } = tree_edit_of(pack, index)
    else {
        unreachable!();
    };
    if restore_mode.is_some() {
        assert_eq!(&None, contents, "an identity edit carries no content");
    }
    (*restore_mode, restore_spans, retombstone_spans)
}

/// A Rust replica applies the JS pack one change at a time and reads the
/// XML JS read after each, then the whole pack at once.
fn apply_one_by_one(name: &str) -> TestResult {
    let wire = WireChangePack::try_from(&decode_change_pack(&fixture_bytes(name)?)?)?;
    let xmls = fixture_xmls(name)?;
    assert_eq!(xmls.len(), wire.changes.len());

    let mut doc = Document::new(KEY);
    doc.set_actor(A2);
    for (index, (change, want)) in wire.changes.iter().zip(&xmls).enumerate() {
        let single = WireChangePack {
            checkpoint: Checkpoint::new(index as i64 + 1, 0),
            changes: vec![change.clone()],
            version_vector: None,
            ..wire.clone()
        };
        doc.apply_change_pack(&ChangePack::try_from(single)?)?;
        assert_eq!(want, &xml_of(&doc), "{name}: after change {index}");
    }

    let mut whole = Document::new(KEY);
    whole.set_actor(A2);
    whole.apply_change_pack(&decode_change_pack(&fixture_bytes(name)?)?)?;
    assert_eq!(xmls.last(), Some(&xml_of(&whole)), "{name}: whole pack");
    Ok(())
}

/// The pack of every local change of `doc`, as JS `record` writes it.
fn pack_bytes(doc: &Document) -> Result<Vec<u8>, Box<dyn Error>> {
    let local = WireChangePack::try_from(&doc.create_change_pack())?;
    let pack = WireChangePack {
        checkpoint: Checkpoint::new(local.changes.len() as i64, 0),
        ..local
    };
    Ok(encode_change_pack(&ChangePack::try_from(pack)?)?)
}

// --- (a) a deletion's undo and redo, and a replace's undo and redo ---

const RESTORE_PACK: &str = "restore_retombstone_pack";

/// The `(node_type, value, id, parent, left, right)` of each span.
#[allow(clippy::type_complexity)]
fn span_summary(
    spans: &[WireTreeRestoreSpan],
) -> Vec<(
    &str,
    &str,
    WireTreeNodeId,
    Option<WireTreeNodeId>,
    Option<WireTreeNodeId>,
    Option<WireTreeNodeId>,
)> {
    spans
        .iter()
        .map(|span| {
            (
                span.node_type.as_str(),
                span.value.as_str(),
                span.id.clone(),
                span.parent_id.clone(),
                span.left_sibling_id.clone(),
                span.right_sibling_id.clone(),
            )
        })
        .collect()
}

/// JS writes the undo of a Tree deletion in `restore` mode and its redo in
/// `retombstone` mode, and the reverse of a replace with both span lists,
/// an element span with its attribute snapshot and anchors; Rust decodes
/// the pack and encodes it back to the same bytes.
#[test]
fn reencodes_the_js_tree_restore_and_retombstone_pack() -> TestResult {
    let wire = reencoded(RESTORE_PACK)?;
    assert_eq!(7, wire.changes.len());
    for index in [1, 4] {
        assert_eq!((None, &[][..], &[][..]), identity_of(&wire, index));
    }

    // The deleted `bc`, between `a` and `d` of the first paragraph.
    let bc = WireTreeRestoreSpan {
        id: node_id(1, 4, 1),
        node_type: "text".to_owned(),
        is_text: true,
        length: 2,
        value: "bc".to_owned(),
        attributes: BTreeMap::new(),
        parent_id: Some(node_id(1, 3, 0)),
        left_sibling_id: Some(node_id(1, 4, 0)),
        right_sibling_id: Some(node_id(1, 4, 3)),
    };
    for (index, mode) in [
        (2, WireRestoreMode::Restore),
        (3, WireRestoreMode::Retombstone),
    ] {
        assert_eq!(
            (Some(mode), &[bc.clone()][..], &[][..]),
            identity_of(&wire, index),
            "change {index}"
        );
    }

    // The replaced `<p a="1">ef</p>`, parent before child, and the
    // `<p>X</p>` that replaced it, last in the root.
    let (_, removed, inserted) = identity_of(&wire, 5);
    assert_eq!(
        vec![
            (
                "p",
                "",
                node_id(1, 5, 0),
                Some(node_id(1, 2, 0)),
                Some(node_id(1, 3, 0)),
                None
            ),
            (
                "text",
                "ef",
                node_id(1, 6, 0),
                Some(node_id(1, 5, 0)),
                None,
                None
            ),
        ],
        span_summary(removed)
    );
    let attr = &removed[0].attributes["a"];
    assert_eq!(
        ("\"1\"", ticket(1, 5), false),
        (
            attr.value.as_str(),
            attr.updated_at.clone(),
            attr.is_removed
        )
    );
    assert_eq!(
        vec![
            (
                "p",
                "",
                node_id(5, 2, 0),
                Some(node_id(1, 2, 0)),
                Some(node_id(1, 3, 0)),
                Some(node_id(1, 5, 0))
            ),
            (
                "text",
                "X",
                node_id(5, 3, 0),
                Some(node_id(5, 2, 0)),
                None,
                None
            ),
        ],
        span_summary(inserted)
    );
    for (index, mode) in [
        (5, WireRestoreMode::Restore),
        (6, WireRestoreMode::Retombstone),
    ] {
        assert_eq!(
            (Some(mode), removed, inserted),
            identity_of(&wire, index),
            "change {index}"
        );
    }
    Ok(())
}

/// A Rust replica applies the JS pack one change at a time and reads the
/// XML JS read after each.
#[test]
fn applies_the_js_tree_restore_and_retombstone_pack() -> TestResult {
    apply_one_by_one(RESTORE_PACK)
}

/// A Rust replica making the same edits, undos and redos as the JS one
/// writes the same bytes.
#[test]
fn writes_the_js_tree_restore_and_retombstone_pack() -> TestResult {
    let mut doc = Document::new(KEY);
    doc.set_actor(A1);
    let styled = TreeNodeInput::element_with_attributes(
        "p",
        BTreeMap::from([("a".to_owned(), Value::String("1".to_owned()))]),
        vec![text("ef")],
    );
    doc.update(|root| {
        root.set_tree(
            "t",
            element("doc", vec![element("p", vec![text("abcd")]), styled]),
        )
        .map(|_| ())
    })?;
    doc.update(|root| root.get_tree_mut("t")?.edit(2, 4, Vec::new(), 0))?;
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;
    doc.update(|root| {
        root.get_tree_mut("t")?
            .edit(4, 8, vec![element("p", vec![text("X")])], 0)
    })?;
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;
    assert_eq!(fixture_xmls(RESTORE_PACK)?.last(), Some(&xml_of(&doc)));

    assert_same_edit_changes(
        RESTORE_PACK,
        &fixture_bytes(RESTORE_PACK)?,
        &pack_bytes(&doc)?,
    )
}

// --- (b) a merge's undo and redo ---

const MERGE_PACK: &str = "merge_undo_pack";

/// JS writes the undo of a merge as a split under the ticket the undo
/// issued for the element it re-creates, carried as `split_tickets`, and
/// its redo as a boundary deletion up to that element; neither runs by
/// identity. Rust decodes the pack and encodes it back to the same bytes.
#[test]
fn reencodes_the_js_merge_undo_pack() -> TestResult {
    let wire = reencoded(MERGE_PACK)?;
    assert_eq!(4, wire.changes.len());
    for index in 1..4 {
        assert_eq!(
            (None, &[][..], &[][..]),
            identity_of(&wire, index),
            "change {index}"
        );
    }
    let WireOperation::TreeEdit {
        split_level,
        split_tickets,
        executed_at,
        ..
    } = tree_edit_of(&wire, 2)
    else {
        unreachable!();
    };
    assert_eq!(1, *split_level);
    assert_eq!(&ticket(3, 1), executed_at);
    assert_eq!(&vec![ticket(3, 2)], split_tickets);

    let WireOperation::TreeEdit {
        to, split_level, ..
    } = tree_edit_of(&wire, 3)
    else {
        unreachable!();
    };
    assert_eq!(0, *split_level);
    assert_eq!(node_id(3, 2, 0), to.parent_id);
    Ok(())
}

/// A Rust replica applies the JS pack one change at a time and reads the
/// XML JS read after each.
#[test]
fn applies_the_js_merge_undo_pack() -> TestResult {
    apply_one_by_one(MERGE_PACK)
}

/// A Rust replica making the same merge, undo and redo writes the same
/// bytes.
#[test]
fn writes_the_js_merge_undo_pack() -> TestResult {
    let mut doc = Document::new(KEY);
    doc.set_actor(A1);
    doc.update(|root| {
        root.set_tree(
            "t",
            element(
                "doc",
                vec![
                    element("p", vec![text("ab")]),
                    element("p", vec![text("cd")]),
                ],
            ),
        )
        .map(|_| ())
    })?;
    doc.update(|root| root.get_tree_mut("t")?.edit(3, 5, Vec::new(), 0))?;
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;
    assert_eq!(fixture_xmls(MERGE_PACK)?.last(), Some(&xml_of(&doc)));

    assert_same_edit_changes(MERGE_PACK, &fixture_bytes(MERGE_PACK)?, &pack_bytes(&doc)?)
}
