//! Options for creating a document (JS `DocumentOptions`).

/// The options to create a document with `Document::with_options` (JS
/// `DocumentOptions`).
///
/// Non-exhaustive, so more JS options can be added later. Start from the
/// default and set the fields to change:
///
/// ```
/// use yorkie_core::{Document, DocumentOptions};
///
/// let mut options = DocumentOptions::default();
/// options.max_undo_depth = Some(5);
/// let doc = Document::with_options("doc", options).unwrap();
/// # let _ = doc;
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DocumentOptions {
    /// How many changes `history_mut().undo()` can go back, and likewise for
    /// `redo()` (JS `maxUndoDepth`). Each `Document::update` that changes the
    /// document is one entry; the oldest is dropped when the stack is full.
    /// `None` means 50 (JS `MaxUndoRedoStackDepth`). Must be positive. A
    /// reverse entry keeps a copy of the content it would restore, so a
    /// larger depth holds more memory.
    pub max_undo_depth: Option<usize>,
}
