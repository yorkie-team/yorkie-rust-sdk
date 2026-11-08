//! Tests that read the undo and redo stacks or the queued changes.
//!
//! JS references: `test/integration/object_test.ts` and `counter_test.ts`
//! (`can get proper reverse operations`), `doc_presence_test.ts` (the
//! `getUndoStackForTest` checks), `src/document/history.ts`, and
//! `EditOperation.reconcileOperation` with the reconcile cases of
//! `test/integration/history_text_test.ts`, run here by handing changes
//! between two documents instead of through a server.

use super::{History, HistoryOperation, MAX_UNDO_REDO_STACK_DEPTH};
use crate::change::{ChangePack, Checkpoint};
use crate::crdt::rga_tree_split::{RgaTreeSplitNodeId, RgaTreeSplitPos};
use crate::crdt::tree::{TreeNodeId, TreePos};
use crate::operation::{EditOperation, OpSource, Operation};
use crate::presence::ReversePresence;
use crate::{DocStatus, Document, JsonObject, Presence, Result, TimeTicket};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn top_strings(stack: &[Vec<HistoryOperation>]) -> Vec<String> {
    stack
        .last()
        .map(|entry| entry.iter().map(HistoryOperation::to_test_string).collect())
        .unwrap_or_default()
}

#[test]
fn records_reverse_operations_of_object_changes() -> Result<()> {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        let mut shape = JsonObject::new();
        shape.set("color", "black")?;
        root.set("shape", shape).map(|_| ())
    })?;
    assert_eq!(r#"{"shape":{"color":"black"}}"#, doc.to_sorted_json());
    assert_eq!(
        vec!["0:00:0.REMOVE.1:00:1"],
        top_strings(doc.undo_stack_for_test())
    );

    doc.history_mut().undo()?;
    assert_eq!("{}", doc.to_sorted_json());
    assert_eq!(
        vec![r#"0:00:0.SET.shape={"color":"black"}"#],
        top_strings(doc.redo_stack_for_test())
    );

    doc.history_mut().redo()?;
    assert_eq!(r#"{"shape":{"color":"black"}}"#, doc.to_sorted_json());
    doc.update(|root| {
        root.get_object_mut("shape")?
            .set("color", "red")
            .map(|_| ())
    })?;
    assert_eq!(r#"{"shape":{"color":"red"}}"#, doc.to_sorted_json());
    assert_eq!(
        vec![r#"1:00:1.SET.color="black""#],
        top_strings(doc.undo_stack_for_test())
    );

    doc.history_mut().undo()?;
    assert_eq!(r#"{"shape":{"color":"black"}}"#, doc.to_sorted_json());
    assert_eq!(
        vec![r#"1:00:1.SET.color="red""#],
        top_strings(doc.redo_stack_for_test())
    );

    doc.history_mut().redo()?;
    assert_eq!(r#"{"shape":{"color":"red"}}"#, doc.to_sorted_json());
    Ok(())
}

#[test]
fn records_reverse_operations_of_counter_increases() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        root.set_counter("cnt", 0)?;
        root.set_long_counter("longCnt", 0)?;
        Ok(())
    })?;
    assert_eq!(r#"{"cnt":0,"longCnt":0}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.get_counter_mut("cnt")?.increase(1.5)?;
        root.get_counter_mut("longCnt")?
            .increase(9223372036854775807i64)?;
        Ok(())
    })?;
    assert_eq!(
        r#"{"cnt":1,"longCnt":9223372036854775807}"#,
        doc.to_sorted_json()
    );
    assert_eq!(
        vec![
            "1:00:2.INCREASE.-9223372036854775807",
            "1:00:1.INCREASE.-1.5"
        ],
        top_strings(doc.undo_stack_for_test())
    );

    doc.history_mut().undo()?;
    assert_eq!(r#"{"cnt":0,"longCnt":0}"#, doc.to_sorted_json());
    assert_eq!(
        vec!["1:00:1.INCREASE.1.5", "1:00:2.INCREASE.9223372036854775807"],
        top_strings(doc.redo_stack_for_test())
    );
    Ok(())
}

fn presence_of(value: Value) -> Presence {
    value.as_object().unwrap().clone()
}

fn reverse(value: Value) -> HistoryOperation {
    let values: ReversePresence = value
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), Some(value.clone())))
        .collect();
    HistoryOperation::Presence(values)
}

#[test]
fn records_only_the_presence_keys_set_with_history() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.set_actor("000000000000000000000001");
    doc.apply_status(DocStatus::Attached);
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(
            json!({"color": "red", "cursor": {"x": 0, "y": 0}}),
        ));
        Ok(())
    })?;
    assert!(doc.undo_stack_for_test().is_empty());

    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(
            json!({"color": "blue", "cursor": {"x": 1, "y": 1}}),
        ));
        Ok(())
    })?;
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(json!({"color": "green"})));
        presence.set_with_history(presence_of(json!({"cursor": {"x": 2, "y": 2}})));
        Ok(())
    })?;
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(json!({"color": "black"})));
        presence.set(presence_of(json!({"cursor": {"x": 3, "y": 3}})));
        Ok(())
    })?;

    assert_eq!(
        vec![
            vec![reverse(json!({"color": "red", "cursor": {"x": 0, "y": 0}}))],
            vec![reverse(json!({"cursor": {"x": 1, "y": 1}}))],
        ],
        doc.undo_stack_for_test()
    );
    Ok(())
}

#[test]
fn queues_undo_and_redo_changes_marked_as_undo_redo() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| root.set("a", 1).map(|_| ()))?;
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;

    let pack = doc.create_change_pack();
    let sources: Vec<_> = pack.changes().iter().map(|c| c.replay_source()).collect();
    assert_eq!(
        vec![OpSource::Local, OpSource::UndoRedo, OpSource::UndoRedo],
        sources
    );
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    Ok(())
}

// --- Stacks (history.ts) ---

fn entry(n: usize) -> Vec<HistoryOperation> {
    let mut values = ReversePresence::new();
    values.insert("n".to_owned(), Some(json!(n)));
    vec![HistoryOperation::Presence(values)]
}

#[test]
fn drops_the_oldest_entry_past_the_maximum_depth() {
    let mut history = History::default();
    for n in 0..MAX_UNDO_REDO_STACK_DEPTH + 2 {
        history.push_undo(entry(n));
        history.push_redo(entry(n));
    }

    assert_eq!(MAX_UNDO_REDO_STACK_DEPTH, history.undo_stack().len());
    assert_eq!(MAX_UNDO_REDO_STACK_DEPTH, history.redo_stack().len());
    assert_eq!(entry(2), history.undo_stack()[0]);
    assert_eq!(
        Some(entry(MAX_UNDO_REDO_STACK_DEPTH + 1)),
        history.pop_redo()
    );

    history.clear_redo();
    assert!(!history.has_redo());
    assert!(history.has_undo());
}

#[test]
fn leaves_tree_reverses_out_of_an_entry() {
    let root = || TreeNodeId::new(TimeTicket::initial(), 0);
    let tree_style = crate::operation::TreeStyleOperation::create(
        TimeTicket::initial(),
        TreePos::new(root(), root()),
        TreePos::new(root(), root()),
        BTreeMap::new(),
        None,
    );

    assert!(HistoryOperation::entry(vec![Operation::TreeStyle(tree_style)], None).is_empty());
}

/// Tree undo is not ported: a tree edit or style pushes nothing, and still
/// clears the redo stack like any other local edit.
#[test]
fn pushes_nothing_for_tree_edits_and_styles() -> Result<()> {
    use crate::TreeNodeInput;

    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        let paragraph = TreeNodeInput::element("p", vec![TreeNodeInput::text("ab")]);
        root.set_tree("t", TreeNodeInput::element("doc", vec![paragraph]))
            .map(|_| ())
    })?;
    doc.update(|root| root.set("a", 1).map(|_| ()))?;
    doc.history_mut().undo()?;
    assert_eq!(1, doc.undo_stack_for_test().len());
    assert!(doc.history().can_redo());

    doc.update(|root| {
        let tree = root.get_tree_mut("t")?;
        tree.edit(1, 1, vec![TreeNodeInput::text("X")], 0)?;
        let mut attributes = BTreeMap::new();
        attributes.insert("bold".to_owned(), json!(true));
        tree.style(0, 1, attributes)
    })?;

    assert_eq!(1, doc.undo_stack_for_test().len());
    assert!(!doc.history().can_redo());
    Ok(())
}

/// An undo edit over `from..to` on a text whose positions were normalized to
/// the head, as `EditOperation::to_reverse_operation` builds it.
fn undo_edit(from: usize, to: usize) -> EditOperation {
    let head = RgaTreeSplitNodeId::new(TimeTicket::initial(), 0);
    EditOperation::new(
        TimeTicket::initial(),
        RgaTreeSplitPos::new(head.clone(), from),
        RgaTreeSplitPos::new(head, to),
        "",
        BTreeMap::new(),
        None,
        true,
    )
}

fn reconciled(local: (usize, usize), remote: (usize, usize), content_len: usize) -> (usize, usize) {
    let mut history = History::default();
    history.push_undo(edit_entry(undo_edit(local.0, local.1)));
    history.reconcile_text_edit(&TimeTicket::initial(), remote.0, remote.1, content_len);
    let op = edit_of(&history.undo_stack()[0][0]);
    (
        op.range_from().relative_offset(),
        op.to_pos().relative_offset(),
    )
}

fn edit_entry(op: EditOperation) -> Vec<HistoryOperation> {
    vec![HistoryOperation::Operation(Box::new(Operation::Edit(op)))]
}

fn edit_of(history_op: &HistoryOperation) -> &EditOperation {
    match history_op {
        HistoryOperation::Operation(op) => match op.as_ref() {
            Operation::Edit(op) => op,
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}

/// The six cases of JS `reconcileOperation`, with the undo range `[4, 6)`.
#[test]
fn reconciles_undo_text_edits_with_remote_edits() {
    // 1. Remote edit left of the range: shift by the length change.
    assert_eq!((5, 7), reconciled((4, 6), (0, 1), 2));
    assert_eq!((2, 4), reconciled((4, 6), (0, 2), 0));
    // 2. Remote edit right of the range: unchanged.
    assert_eq!((4, 6), reconciled((4, 6), (6, 8), 1));
    // 3. Range inside the remote edit: collapse to its start.
    assert_eq!((2, 2), reconciled((4, 6), (2, 8), 0));
    // 4. Remote edit inside the range: shift the end.
    assert_eq!((4, 7), reconciled((4, 6), (5, 5), 1));
    // 5. Remote edit over the start of the range.
    assert_eq!((2, 3), reconciled((4, 6), (2, 5), 0));
    // 6. Remote edit over the end of the range.
    assert_eq!((4, 5), reconciled((4, 6), (5, 8), 0));
}

#[test]
fn reconciles_only_undo_edits_of_the_same_text() {
    let mut history = History::default();
    let other_text = TimeTicket::new(1, 1, "000000000000000000000000");
    let mut not_undo = undo_edit(4, 6);
    not_undo = EditOperation::new(
        not_undo.parent_created_at().clone(),
        not_undo.range_from().clone(),
        not_undo.to_pos().clone(),
        "",
        BTreeMap::new(),
        None,
        false,
    );
    history.push_undo(edit_entry(not_undo));
    history.push_undo(edit_entry(undo_edit(4, 6)));

    history.reconcile_text_edit(&other_text, 0, 2, 0);
    history.reconcile_text_edit(&TimeTicket::initial(), 0, 2, 0);

    let offsets: Vec<_> = history
        .undo_stack()
        .iter()
        .map(|entry| edit_of(&entry[0]).range_from().relative_offset())
        .collect();
    assert_eq!(vec![4, 2], offsets);
}

// --- Two replicas (history_text_test.ts reconcile cases) ---

fn replica(n: u8) -> Document {
    let mut doc = Document::new("doc-key");
    doc.set_actor(format!("{n:024}"));
    doc
}

/// Moves the changes `doc` has not handed over yet into `target`.
fn deliver(doc: &mut Document, sent: &mut usize, target: &mut Document) -> Result<()> {
    let changes = doc.create_change_pack().changes()[*sent..].to_vec();
    *sent += changes.len();
    let pack = ChangePack::create("doc-key", Checkpoint::new(1, 0), false, changes, None, None);
    target.apply_change_pack(&pack)
}

struct Pair {
    d1: Document,
    d2: Document,
    sent1: usize,
    sent2: usize,
}

impl Pair {
    fn with_text(content: &str) -> Result<Self> {
        let mut pair = Self {
            d1: replica(1),
            d2: replica(2),
            sent1: 0,
            sent2: 0,
        };
        pair.d1
            .update(|root| root.set_text("t")?.edit(0, 0, content, None).map(|_| ()))?;
        pair.sync()?;
        Ok(pair)
    }

    fn sync(&mut self) -> Result<()> {
        deliver(&mut self.d1, &mut self.sent1, &mut self.d2)?;
        deliver(&mut self.d2, &mut self.sent2, &mut self.d1)?;
        assert_eq!(self.d1.to_sorted_json(), self.d2.to_sorted_json());
        Ok(())
    }

    fn edit(doc: &mut Document, from: usize, to: usize, content: &str) -> Result<()> {
        doc.update(|root| {
            root.get_text_mut("t")?
                .edit(from, to, content, None)
                .map(|_| ())
        })
    }
}

/// The reconcile cases of `history_text_test.ts`: concurrent edits, both
/// undo, both redo, and the replicas converge after each step.
#[test]
fn converges_after_undo_and_redo_of_concurrent_text_edits() -> Result<()> {
    let cases = [
        ("left", (6, 8, ""), (2, 2, "XX")),
        ("right", (2, 4, ""), (8, 8, "YY")),
        ("contained_by", (4, 6, ""), (2, 8, "")),
        ("contains", (2, 8, ""), (5, 5, "ZZ")),
        ("overlap_start", (4, 8, ""), (2, 6, "")),
        ("overlap_end", (2, 6, ""), (4, 8, "")),
        ("adjacent", (4, 6, ""), (6, 6, "AA")),
    ];
    for (name, (f1, t1, c1), (f2, t2, c2)) in cases {
        let mut pair = Pair::with_text("0123456789")?;
        Pair::edit(&mut pair.d1, f1, t1, c1)?;
        Pair::edit(&mut pair.d2, f2, t2, c2)?;
        pair.sync()?;

        pair.d1.history_mut().undo()?;
        pair.d2.history_mut().undo()?;
        pair.sync().map_err(|err| panic!("{name}: undo: {err}"))?;

        pair.d1.history_mut().redo()?;
        pair.d2.history_mut().redo()?;
        pair.sync().map_err(|err| panic!("{name}: redo: {err}"))?;
    }
    Ok(())
}

/// An undo of a deletion that a remote insertion shifted right lands where
/// the deleted text was, on both replicas.
#[test]
fn restores_a_deletion_shifted_by_a_remote_insertion() -> Result<()> {
    let mut pair = Pair::with_text("0123456789")?;
    Pair::edit(&mut pair.d1, 6, 8, "")?;
    Pair::edit(&mut pair.d2, 2, 2, "XX")?;
    pair.sync()?;
    assert_eq!("01XX234589", content_of(&pair.d1));

    pair.d1.history_mut().undo()?;
    pair.sync()?;

    assert_eq!("01XX23456789", content_of(&pair.d1));
    Ok(())
}

fn content_of(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(crate::JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected text at t, got {other:?}"),
    }
}
