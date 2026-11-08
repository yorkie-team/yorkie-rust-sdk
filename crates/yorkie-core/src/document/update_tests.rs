//! Tests that a rejected update or undo leaves the document untouched.
//!
//! JS reference: `src/document/document.ts` `update` and `executeUndoRedo`.
//! JS runs a change on its clone root first and drops the clone when that
//! fails, so the root never holds part of a change.

use super::Document;
use crate::history::HistoryOperation;
use crate::json::lock_recorder;
use crate::operation::{Operation, RemoveOperation};
use crate::{Result, TimeTicket, YorkieError};

/// Everything a rejected update or undo must leave as it was.
#[derive(Debug, PartialEq)]
struct Observed {
    json: String,
    crdt_json: String,
    local_changes: usize,
    undo_stack: Vec<Vec<HistoryOperation>>,
    redo_stack: Vec<Vec<HistoryOperation>>,
    doc_size: crate::crdt::root::DocSize,
    change_id: crate::change::ChangeId,
}

fn observe(doc: &Document) -> Observed {
    Observed {
        json: doc.to_sorted_json(),
        crdt_json: doc.crdt_root.to_sorted_json(),
        local_changes: doc.local_changes.len(),
        undo_stack: doc.undo_stack_for_test().to_vec(),
        redo_stack: doc.redo_stack_for_test().to_vec(),
        doc_size: doc.crdt_root.doc_size(),
        change_id: doc.change_id.clone(),
    }
}

/// A remove whose parent does not exist, so executing it always fails.
fn missing_parent_remove() -> RemoveOperation {
    RemoveOperation::new(
        TimeTicket::new(99, 0, "ghost"),
        TimeTicket::new(99, 1, "ghost"),
        Some(TimeTicket::new(99, 2, "ghost")),
    )
}

fn doc_with_a_and_b() -> Result<Document> {
    let mut doc = Document::new("doc");
    doc.update(|root| root.set("a", 1).map(|_| ()))?;
    doc.update(|root| root.set("b", 2).map(|_| ()))?;
    Ok(doc)
}

#[test]
fn operation_failing_partway_rolls_back_the_update() -> Result<()> {
    let mut doc = doc_with_a_and_b()?;
    let before = observe(&doc);

    // The first operation applies; the second names a parent that does not
    // exist and fails when the change runs.
    let err = doc
        .update(|root| {
            root.set("c", 3)?;
            let recorder = root.recorder_for_test().expect("update recorder");
            let ghost = missing_parent_remove();
            lock_recorder(&recorder).record_object_remove(
                ghost.parent_created_at(),
                "ghost",
                ghost.created_at(),
            );
            Ok(())
        })
        .unwrap_err();

    assert!(matches!(err, YorkieError::MissingCrdtElement(_)), "{err:?}");
    assert_eq!(before, observe(&doc));

    // The next change takes the ID the rejected one would have had.
    doc.update(|root| root.set("c", 3).map(|_| ()))?;
    assert_eq!(r#"{"a":1,"b":2,"c":3}"#, doc.to_sorted_json());
    assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
    assert_eq!(
        before.change_id.next(false).lamport(),
        doc.local_changes.last().unwrap().id().lamport()
    );
    Ok(())
}

#[test]
fn operation_failing_partway_rolls_back_the_undo() -> Result<()> {
    let mut doc = doc_with_a_and_b()?;

    // The top entry undoes the set of "b"; append an operation that fails
    // after that reverse has run.
    let mut entry = doc.history.pop_undo().unwrap();
    entry.push(HistoryOperation::Operation(Box::new(Operation::Remove(
        missing_parent_remove(),
    ))));
    doc.history.push_undo(entry);
    let before = observe(&doc);

    let err = doc.history_mut().undo().unwrap_err();

    assert!(matches!(err, YorkieError::MissingCrdtElement(_)), "{err:?}");
    assert_eq!(before, observe(&doc));
    assert!(doc.history().can_undo());
    Ok(())
}
