//! Tests that a rejected update or undo leaves the document untouched (a
//! failed undo or redo only drops its entry, as in JS), and
//! that local updates are checked against schema rules and the maximum
//! document size.
//!
//! JS references: `src/document/document.ts` `update` (its validation
//! block) and `executeUndoRedo`; `test/integration/document_schema_test.ts`
//! and `document_limit_test.ts`. JS runs a change on its clone root first
//! and drops the clone when that fails or is rejected, so the root never
//! holds part of a change.

use super::{DocStatus, Document};
use crate::history::HistoryOperation;
use crate::json::lock_recorder;
use crate::operation::{Operation, RemoveOperation};
use crate::{
    ActorId, DataSize, DocSize, JsonText, Presence, Result, SchemaRule, TimeTicket, YorkieError,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Everything a rejected update must leave as it was.
#[derive(Debug, PartialEq)]
struct Observed {
    json: String,
    crdt_json: String,
    local_changes: usize,
    undo_stack: Vec<Vec<HistoryOperation>>,
    redo_stack: Vec<Vec<HistoryOperation>>,
    doc_size: crate::crdt::root::DocSize,
    change_id: crate::change::ChangeId,
    presences: BTreeMap<ActorId, Presence>,
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
        presences: doc.presences.clone(),
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

    // As in JS, the failed entry is dropped; nothing else changes.
    assert!(matches!(err, YorkieError::MissingCrdtElement(_)), "{err:?}");
    let mut expected = before;
    expected.undo_stack.pop();
    assert_eq!(expected, observe(&doc));
    assert_eq!(1, doc.undo_stack_for_test().len());
    Ok(())
}

fn presence_of(name: &str) -> Presence {
    let mut presence = Presence::new();
    presence.insert("name".to_owned(), serde_json::json!(name));
    presence
}

fn string_title_rule() -> Vec<SchemaRule> {
    vec![SchemaRule::new("$.title", "string", Vec::new())]
}

/// A document whose undo and redo stacks both hold an entry, so a rejected
/// update that cleared the redo stack would show.
fn doc_with_history() -> Result<Document> {
    let mut doc = Document::new("doc");
    doc.update(|root| root.set("title", "hello").map(|_| ()))?;
    doc.update(|root| root.set("body", "world").map(|_| ()))?;
    doc.history_mut().undo()?;
    assert!(doc.history().can_undo() && doc.history().can_redo());
    Ok(doc)
}

#[test]
fn rejected_update_rolls_back() -> Result<()> {
    // A schema failure.
    let mut doc = doc_with_history()?;
    doc.set_schema_rules(string_title_rule());
    let before = observe(&doc);

    let err = doc
        .update(|root| root.set("title", 1).map(|_| ()))
        .unwrap_err();

    assert_eq!(
        YorkieError::SchemaValidationFailed(vec!["expected string at path $.title".to_owned()]),
        err
    );
    assert_eq!(
        "schema validation failed: expected string at path $.title",
        err.to_string()
    );
    assert_eq!(before, observe(&doc));
    assert!(doc.history().can_undo() && doc.history().can_redo());

    // A size failure.
    let mut doc = doc_with_history()?;
    let limit = doc.doc_size().total() + 10;
    doc.set_max_size_per_document(limit);
    let before = observe(&doc);

    let err = doc
        .update(|root| root.set("title", "a much longer title").map(|_| ()))
        .unwrap_err();

    let YorkieError::DocumentSizeExceedsLimit { size, limit: got } = err else {
        panic!("expected a size error, got {err:?}");
    };
    assert_eq!(limit, got);
    assert!(size > limit);
    assert_eq!(before, observe(&doc));
    assert!(doc.history().can_undo() && doc.history().can_redo());

    // The next change takes the ID the rejected one would have had.
    doc.set_max_size_per_document(0);
    doc.update(|root| root.set("title", "hi").map(|_| ()))?;
    assert_eq!(
        before.change_id.next(false).lamport(),
        doc.local_changes.last().unwrap().id().lamport()
    );

    // A failure in an update that also sets presence, watched by a
    // subscriber: the presence stays and no event fires.
    let mut doc = doc_with_history()?;
    doc.set_actor("000000000000000000000001");
    doc.apply_status(DocStatus::Attached);
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of("before"));
        Ok(())
    })?;
    doc.set_schema_rules(string_title_rule());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    doc.subscribe(move |event| sink.lock().unwrap().push(format!("{event:?}")));
    let before_presence = observe(&doc);

    let err = doc
        .update_with_presence(|root, presence| {
            presence.set(presence_of("after"));
            root.set("title", 1).map(|_| ())
        })
        .unwrap_err();

    assert!(
        matches!(err, YorkieError::SchemaValidationFailed(_)),
        "{err:?}"
    );
    assert_eq!(before_presence, observe(&doc));
    assert_eq!(presence_of("before"), doc.my_presence());
    assert!(events.lock().unwrap().is_empty(), "{events:?}");
    Ok(())
}

/// JS `document_schema_test.ts`: `should reject local update that violates
/// schema`.
#[test]
fn rejects_local_update_that_violates_schema() -> Result<()> {
    let mut doc = Document::new("doc");
    doc.set_schema_rules(string_title_rule());

    let err = doc
        .update(|root| root.set("title", 123).map(|_| ()))
        .unwrap_err();
    assert_eq!(
        "schema validation failed: expected string at path $.title",
        err.to_string()
    );
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| root.set("title", "hello").map(|_| ()))?;
    assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
    Ok(())
}

#[test]
fn joins_every_schema_violation_into_one_error() -> Result<()> {
    let mut doc = Document::new("doc");
    doc.set_schema_rules(vec![
        SchemaRule::new("$.title", "string", Vec::new()),
        SchemaRule::new("$.count", "integer", Vec::new()),
    ]);

    let err = doc
        .update(|root| {
            root.set("title", 1)?;
            root.set("count", "one").map(|_| ())
        })
        .unwrap_err();
    assert_eq!(
        "schema validation failed: expected string at path $.title, \
         expected integer at path $.count",
        err.to_string()
    );
    Ok(())
}

/// JS validates the whole document after each local update, but skips a
/// change that only touches presence.
#[test]
fn checks_the_whole_document_but_not_presence_only_changes() -> Result<()> {
    let mut doc = Document::new("doc");
    doc.update(|root| root.set("title", 1).map(|_| ()))?;
    doc.set_schema_rules(string_title_rule());

    doc.update_with_presence(|_, presence| {
        let mut value = Presence::new();
        value.insert("name".to_owned(), serde_json::json!("a"));
        presence.set(value);
        Ok(())
    })?;

    let err = doc
        .update(|root| root.set("other", "x").map(|_| ()))
        .unwrap_err();
    assert!(
        matches!(err, YorkieError::SchemaValidationFailed(_)),
        "{err:?}"
    );
    assert_eq!(r#"{"title":1}"#, doc.to_sorted_json());
    Ok(())
}

/// JS `document_limit_test.ts`: `should reject local update that exceeds
/// document size limit`.
#[test]
fn rejects_local_update_that_exceeds_document_size_limit() -> Result<()> {
    let mut doc = Document::new("doc");
    doc.set_max_size_per_document(100);
    doc.update(|root| root.set("text", JsonText::new()).map(|_| ()))?;
    assert_eq!(DataSize { data: 0, meta: 72 }, doc.doc_size().live);

    let err = doc
        .update(|root| {
            root.get_text_mut("text")?
                .edit(0, 0, "helloworld", None)
                .map(|_| ())
        })
        .unwrap_err();

    assert_eq!(
        YorkieError::DocumentSizeExceedsLimit {
            size: 116,
            limit: 100
        },
        err
    );
    assert_eq!("document size exceeded: 116 > 100", err.to_string());
    assert_eq!(72, doc.doc_size().total());
    assert_eq!(
        DocSize {
            live: DataSize { data: 0, meta: 72 },
            gc: DataSize::default(),
        },
        doc.doc_size()
    );
    Ok(())
}

/// JS checks the limit only for local updates: undo and redo are not
/// checked.
#[test]
fn undo_is_not_checked_against_the_limit() -> Result<()> {
    let mut doc = Document::new("doc");
    doc.update(|root| root.set("title", "a long title to restore").map(|_| ()))?;
    doc.update(|root| root.set("title", "x").map(|_| ()))?;
    doc.set_max_size_per_document(doc.doc_size().total());

    doc.history_mut().undo()?;
    assert_eq!(
        r#"{"title":"a long title to restore"}"#,
        doc.to_sorted_json()
    );
    assert!(doc.doc_size().total() > doc.max_size_per_document());
    Ok(())
}
