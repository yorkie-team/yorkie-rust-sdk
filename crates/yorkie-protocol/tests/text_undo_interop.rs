//! Byte-level interop of Text undo with the JS SDK.
//!
//! The fixtures under `tests/fixtures/text_undo` were written by the JS SDK
//! at `84068f70` (checkout `8629993e`, which adds only upgrade notes) with
//! `tools/interop/text_undo_fixtures_test.ts`; its header has the exact
//! command. Each `.hex` file is a protobuf `ChangePack`, and each `.json`
//! file the sorted JSON of the document JS built.
//!
//! `tools/interop/text_undo_fixtures_test.go` builds the same scenarios with
//! Go at `54b96762`. Go writes the restore and retombstone pack byte for
//! byte as JS does, so it has no fixture of its own; Go's ledger snapshot
//! differs in one attribute ticket and is `go_ledger_snapshot_pack.hex`.

use std::collections::BTreeMap;
use std::error::Error;

use serde_json::Value;
use yorkie_core::wire::{
    WireChangePack, WireJsonElement, WireOperation, WireRestoreMode, WireRestoreSpan, WireTextNode,
};
use yorkie_core::{ChangePack, Checkpoint, Document, JsonValue, TimeTicket};
use yorkie_protocol::converter::{decode_change_pack, encode_change_pack};

type TestResult = Result<(), Box<dyn Error>>;

const A1: &str = "000000000000000000000001";
const A2: &str = "000000000000000000000002";
const KEY: &str = "text-undo-interop";

fn fixture_path(name: &str) -> String {
    format!(
        "{}/tests/fixtures/text_undo/{name}",
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

fn fixture_json(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(
        std::fs::read_to_string(fixture_path(&format!("{name}.json")))?
            .trim()
            .to_owned(),
    )
}

fn text_of(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

fn ticket(lamport: i64, delimiter: u32, actor: &str) -> TimeTicket {
    TimeTicket::new(lamport, delimiter, actor)
}

// --- (a) a deletion's undo and redo, and a replace's undo and redo ---

const RESTORE_PACK: &str = "restore_retombstone_pack";

/// The text after each change of the JS pack: the insertion, the deletion
/// of `234`, its undo and redo, the replace of `01` by `ab`, and its undo
/// and redo.
const RESTORE_TEXTS: [&str; 7] = [
    "0123456789",
    "0156789",
    "0123456789",
    "0156789",
    "ab56789",
    "0156789",
    "ab56789",
];

/// The identity edit of the change at `index`, as its mode and the
/// `(start, end, content)` of its restore and retombstone spans.
#[allow(clippy::type_complexity)]
fn identity_edit_of(
    pack: &WireChangePack,
    index: usize,
) -> (
    Option<WireRestoreMode>,
    Vec<(usize, usize, String)>,
    Vec<(usize, usize, String)>,
) {
    let parts = |spans: &[WireRestoreSpan]| {
        spans
            .iter()
            .map(|span| (span.start, span.end, span.content.clone()))
            .collect::<Vec<_>>()
    };
    match &pack.changes[index].operations[..] {
        [WireOperation::Edit {
            content,
            restore_spans,
            restore_mode,
            retombstone_spans,
            ..
        }] => {
            assert_eq!("", content, "an identity edit carries no content");
            (
                *restore_mode,
                parts(restore_spans),
                parts(retombstone_spans),
            )
        }
        other => panic!("expected one edit in change {index}, got {other:?}"),
    }
}

/// JS writes the undo of a deletion in `restore` mode, its redo in
/// `retombstone` mode, and the reverse of a replace with both span lists;
/// Rust decodes each pack and encodes it back to the same bytes.
#[test]
fn reencodes_the_js_restore_and_retombstone_pack() -> TestResult {
    let bytes = fixture_bytes(RESTORE_PACK)?;
    let pack = decode_change_pack(&bytes)?;
    assert_eq!(bytes, encode_change_pack(&pack)?);

    let wire = WireChangePack::try_from(&pack)?;
    assert_eq!(RESTORE_TEXTS.len(), wire.changes.len());
    let span = |start, end, content: &str| (start, end, content.to_owned());
    assert_eq!(
        (
            Some(WireRestoreMode::Restore),
            vec![span(2, 5, "234")],
            vec![]
        ),
        identity_edit_of(&wire, 2)
    );
    assert_eq!(
        (
            Some(WireRestoreMode::Retombstone),
            vec![span(2, 5, "234")],
            vec![]
        ),
        identity_edit_of(&wire, 3)
    );
    for (index, mode) in [
        (5, WireRestoreMode::Restore),
        (6, WireRestoreMode::Retombstone),
    ] {
        assert_eq!(
            (Some(mode), vec![span(0, 2, "01")], vec![span(0, 2, "ab")]),
            identity_edit_of(&wire, index)
        );
    }

    // The restored `01` keeps its attribute, and each span names the
    // insertion it came from.
    let WireOperation::Edit {
        restore_spans,
        retombstone_spans,
        ..
    } = &wire.changes[5].operations[0]
    else {
        unreachable!();
    };
    assert_eq!(ticket(1, 2, A1), restore_spans[0].created_at);
    assert_eq!(
        BTreeMap::from([("b".to_owned(), "\"1\"".to_owned())]),
        restore_spans[0].attributes
    );
    assert_eq!(ticket(5, 1, A1), retombstone_spans[0].created_at);
    assert!(retombstone_spans[0].attributes.is_empty());
    Ok(())
}

/// A Rust replica applies the JS pack one change at a time and reads the
/// text JS read after each, ending on the document JS built.
#[test]
fn applies_the_js_restore_and_retombstone_pack() -> TestResult {
    let wire = WireChangePack::try_from(&decode_change_pack(&fixture_bytes(RESTORE_PACK)?)?)?;

    let mut doc = Document::new(KEY);
    doc.set_actor(A2);
    for (index, (change, want)) in wire.changes.iter().zip(RESTORE_TEXTS).enumerate() {
        let single = WireChangePack {
            checkpoint: Checkpoint::new(index as i64 + 1, 0),
            changes: vec![change.clone()],
            version_vector: None,
            ..wire.clone()
        };
        doc.apply_change_pack(&ChangePack::try_from(single)?)?;
        assert_eq!(want, text_of(&doc), "after change {index}");
    }
    assert_eq!(fixture_json(RESTORE_PACK)?, doc.to_sorted_json());

    // The whole pack, with its version vector, reads the same.
    let mut whole = Document::new(KEY);
    whole.set_actor(A2);
    whole.apply_change_pack(&decode_change_pack(&fixture_bytes(RESTORE_PACK)?)?)?;
    assert_eq!(fixture_json(RESTORE_PACK)?, whole.to_sorted_json());
    Ok(())
}

/// A Rust replica making the same edits, undos, and redos as the JS one
/// writes the same bytes. (Its snapshot counterpart is the `yorkie-core`
/// test `builds_the_root_of_the_js_ledger_snapshot_fixture`: the protocol
/// crate cannot read a document's root.)
#[test]
fn writes_the_js_restore_and_retombstone_pack() -> TestResult {
    let mut doc = Document::new(KEY);
    doc.set_actor(A1);
    let attributes = BTreeMap::from([("b".to_owned(), Value::String("1".to_owned()))]);
    doc.update(|root| {
        root.set_text("t")?
            .edit(0, 0, "0123456789", Some(attributes.clone()))
            .map(|_| ())
    })?;
    doc.update(|root| root.get_text_mut("t")?.edit(2, 5, "", None).map(|_| ()))?;
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;
    doc.update(|root| root.get_text_mut("t")?.edit(0, 2, "ab", None).map(|_| ()))?;
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;
    assert_eq!("ab56789", text_of(&doc));

    let local = WireChangePack::try_from(&doc.create_change_pack())?;
    let pack = WireChangePack {
        checkpoint: Checkpoint::new(local.changes.len() as i64, 0),
        ..local
    };
    assert_eq!(
        fixture_bytes(RESTORE_PACK)?,
        encode_change_pack(&ChangePack::try_from(pack)?)?
    );
    Ok(())
}

// --- (b) a snapshot holding ledgers ---

const SNAPSHOT_PACK: &str = "ledger_snapshot_pack";

fn snapshot_text_nodes(pack: &WireChangePack) -> &[WireTextNode] {
    let Some(WireJsonElement::Object { nodes, .. }) = &pack.snapshot_root else {
        panic!("expected an object snapshot root");
    };
    match &nodes[..] {
        [node] if node.key == "t" => match &node.element {
            WireJsonElement::Text { nodes, .. } => nodes,
            other => panic!("expected a text at t, got {other:?}"),
        },
        other => panic!("expected only t in the root, got {other:?}"),
    }
}

const GO_SNAPSHOT_PACK: &str = "go_ledger_snapshot_pack";

/// Decodes the snapshot pack `name`, checks its Text node by node, and
/// checks that the root Rust decoded encodes back to the same bytes. The
/// recreated `ef` carries its attribute under `ef_attr_at`.
fn reencode_ledger_snapshot(name: &str, ef_attr_at: TimeTicket) -> TestResult {
    let bytes = fixture_bytes(name)?;
    let pack = decode_change_pack(&bytes)?;

    // Drop the raw snapshot so the encoder writes the root Rust decoded.
    let mut wire = WireChangePack::try_from(&pack)?;
    assert!(wire.snapshot.is_some());
    wire.snapshot = None;
    let nodes = snapshot_text_nodes(&wire);

    let inserted = ticket(1, 2, A1);
    let ledgers = nodes
        .iter()
        .map(|node| {
            assert_eq!(inserted, node.id.created_at);
            let attr = &node.attributes["b"];
            assert_eq!(("\"1\"", false), (attr.value.as_str(), attr.is_removed));
            (
                node.value.as_str(),
                node.id.offset,
                node.removed_at.clone(),
                node.removal_events.clone(),
                attr.updated_at.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vec![
            ("ab", 0, None, vec![], inserted.clone()),
            // Removed by A1 at 4, restored by A1's undo at 5, then removed
            // by A2 at 7, which is `removed_at`.
            (
                "cd",
                2,
                Some(ticket(7, 1, A2)),
                vec![ticket(4, 1, A1), ticket(5, 1, A1)],
                inserted.clone(),
            ),
            // Purged, then recreated by A1's undo at 3: the lamport-0
            // stand-in removal and its cancel.
            (
                "ef",
                4,
                None,
                vec![ticket(0, 0, A1), ticket(3, 1, A1)],
                ef_attr_at,
            ),
            ("gh", 6, None, vec![], inserted.clone()),
        ],
        ledgers
    );

    assert_eq!(bytes, encode_change_pack(&ChangePack::try_from(wire)?)?);
    Ok(())
}

/// Rust reads every ledger of the JS snapshot, and the root it decoded
/// encodes back to the bytes JS wrote. JS keeps the insertion's attribute
/// ticket on the recreated piece.
#[test]
fn reencodes_the_js_ledger_snapshot() -> TestResult {
    reencode_ledger_snapshot(SNAPSHOT_PACK, ticket(1, 2, A1))
}

/// The same for the snapshot Go writes from the same edits, which differs
/// from the JS one only in the recreated piece's attribute: Go stamps it
/// with the undo's ticket.
#[test]
fn reencodes_the_go_ledger_snapshot() -> TestResult {
    reencode_ledger_snapshot(GO_SNAPSHOT_PACK, ticket(3, 1, A1))
}

/// A Rust replica loads the JS snapshot, and the Go one, to the document
/// JS built.
#[test]
fn applies_the_js_and_go_ledger_snapshots() -> TestResult {
    for name in [SNAPSHOT_PACK, GO_SNAPSHOT_PACK] {
        let mut doc = Document::new(KEY);
        doc.set_actor("000000000000000000000003");
        doc.apply_change_pack(&decode_change_pack(&fixture_bytes(name)?)?)?;
        assert_eq!("abefgh", text_of(&doc), "{name}");
        assert_eq!(fixture_json(SNAPSHOT_PACK)?, doc.to_sorted_json(), "{name}");
    }
    Ok(())
}

// --- (c) a snapshot holding a removed Text attribute ---

const REMOVED_ATTR_PACK: &str = "removed_text_attr_snapshot_pack";

/// JS `toTextNodes` writes a removed Text attribute with `isRemoved` (since
/// yorkie-js-sdk#1364): the `bold` the undo of a style tombstoned keeps its
/// value and the undo's ticket. Rust reads it as written, and the root it
/// decoded encodes back to the bytes JS wrote.
#[test]
fn reencodes_the_js_removed_text_attr_snapshot() -> TestResult {
    let bytes = fixture_bytes(REMOVED_ATTR_PACK)?;
    let pack = decode_change_pack(&bytes)?;

    let mut wire = WireChangePack::try_from(&pack)?;
    assert!(wire.snapshot.is_some());
    wire.snapshot = None;
    let nodes = snapshot_text_nodes(&wire);
    assert_eq!(1, nodes.len());
    assert_eq!("hello", nodes[0].value);
    let attr = &nodes[0].attributes["bold"];
    assert_eq!(
        ("\"true\"", ticket(3, 1, A1), true),
        (
            attr.value.as_str(),
            attr.updated_at.clone(),
            attr.is_removed
        )
    );

    assert_eq!(bytes, encode_change_pack(&ChangePack::try_from(wire)?)?);
    Ok(())
}

/// JS `should round-trip a tombstoned text attribute`: a replica loading
/// the snapshot does not render the removed attribute and holds its
/// tombstone as the one GC pair the writer holds.
#[test]
fn applies_the_js_removed_text_attr_snapshot() -> TestResult {
    let mut doc = Document::new(KEY);
    doc.set_actor(A2);
    doc.apply_change_pack(&decode_change_pack(&fixture_bytes(REMOVED_ATTR_PACK)?)?)?;
    assert_eq!(fixture_json(REMOVED_ATTR_PACK)?, doc.to_sorted_json());
    assert_eq!(1, doc.garbage_len());
    Ok(())
}
