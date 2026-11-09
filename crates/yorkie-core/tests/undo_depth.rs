//! The undo depth option and `Document::clear_history`.
//!
//! JS reference: `test/unit/document/undo_depth_test.ts`, `describe('Document
//! maxUndoDepth')` (54-115). Its `History depth` cases (117-160) are unit
//! tests beside `src/history.rs`, since they read the stacks.
//!
//! JS rejects five `maxUndoDepth` values (95): `0`, `-1`, `1.5`, `NaN` and
//! `Infinity`. Rust takes `Option<usize>`, so only `0` is representable; the
//! other four cannot be written.

use yorkie_core::{Document, DocumentOptions, JsonValue, Result, YorkieError};

/// JS `MaxUndoRedoStackDepth`.
const DEFAULT_DEPTH: usize = 50;

/// JS `typed` (29-40): types `count` characters, one `update` each, after an
/// initial Text whose creation `clear_history` takes off the undo stack.
fn typed(count: usize, max_undo_depth: Option<usize>) -> Result<Document> {
    let mut options = DocumentOptions::default();
    options.max_undo_depth = max_undo_depth;
    let mut doc = Document::with_options("undo-depth", options)?;
    doc.update(|root| root.set_text("t").map(|_| ()))?;
    doc.clear_history();
    for i in 0..count {
        doc.update(|root| root.get_text_mut("t")?.edit(i, i, "a", None).map(|_| ()))?;
    }
    Ok(doc)
}

/// JS `undoAll` (43-51): undoes until `can_undo` is false and returns how many
/// steps it took.
fn undo_all(doc: &mut Document) -> Result<usize> {
    let mut steps = 0;
    while doc.history().can_undo() {
        doc.history_mut().undo()?;
        steps += 1;
    }
    Ok(steps)
}

fn content_of(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected text at t, got {other:?}"),
    }
}

/// JS "keeps the default depth when the option is not given".
#[test]
fn keeps_the_default_depth_when_the_option_is_not_given() -> Result<()> {
    let mut doc = typed(DEFAULT_DEPTH + 10, None)?;

    assert_eq!(DEFAULT_DEPTH, undo_all(&mut doc)?);
    assert_eq!("a".repeat(10), content_of(&doc));
    Ok(())
}

/// JS "keeps as many undo entries as the option allows".
#[test]
fn keeps_as_many_undo_entries_as_the_option_allows() -> Result<()> {
    let mut doc = typed(120, Some(100))?;

    assert_eq!(100, undo_all(&mut doc)?);
    assert_eq!("a".repeat(20), content_of(&doc));
    Ok(())
}

/// JS "drops the oldest entries first when the depth is small".
#[test]
fn drops_the_oldest_entries_first_when_the_depth_is_small() -> Result<()> {
    let mut doc = typed(5, Some(2))?;

    assert_eq!(2, undo_all(&mut doc)?);
    assert_eq!("aaa", content_of(&doc));
    Ok(())
}

/// JS "bounds the redo stack by the same depth, not the default one".
///
/// The depth is above the default: undoing all 60 changes pushes 60 redo
/// entries, so a redo stack still bounded by the default would have dropped
/// the 10 oldest of them.
#[test]
fn bounds_the_redo_stack_by_the_same_depth_not_the_default_one() -> Result<()> {
    let mut doc = typed(60, Some(60))?;

    assert_eq!(60, undo_all(&mut doc)?);
    assert_eq!("", content_of(&doc));

    let mut redone = 0;
    while doc.history().can_redo() {
        doc.history_mut().redo()?;
        redone += 1;
    }

    assert_eq!(60, redone);
    assert_eq!("a".repeat(60), content_of(&doc));
    Ok(())
}

/// JS "rejects a maxUndoDepth of 0" (the loop at 95, the one representable
/// value).
#[test]
fn rejects_a_max_undo_depth_of_0() {
    let mut options = DocumentOptions::default();
    options.max_undo_depth = Some(0);

    let err = Document::with_options("undo-depth", options).unwrap_err();
    assert_eq!(
        YorkieError::InvalidArgument("maxUndoDepth must be a positive integer: 0".to_owned()),
        err
    );
}

/// JS `clearHistory` (`src/document/document.ts` 2898-2907) drops both
/// stacks and leaves the document, its local changes and its presence alone.
#[test]
fn clear_history_drops_both_stacks_only() -> Result<()> {
    let mut doc = typed(3, None)?;
    doc.history_mut().undo()?;
    assert!(doc.history().can_undo() && doc.history().can_redo());
    let json = doc.to_sorted_json();
    let presence = doc.my_presence();
    let has_local_changes = doc.has_local_changes();

    doc.clear_history();

    assert!(!doc.history().can_undo());
    assert!(!doc.history().can_redo());
    assert_eq!(json, doc.to_sorted_json());
    assert_eq!(presence, doc.my_presence());
    assert_eq!(has_local_changes, doc.has_local_changes());
    Ok(())
}
