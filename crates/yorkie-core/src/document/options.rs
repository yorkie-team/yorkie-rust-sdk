//! Options for creating a document (JS `DocumentOptions`).

/// The options to create a document with `Document::with_options` (JS
/// `DocumentOptions`).
///
/// Like `AttachOptions`, set the fields to change and take the rest from the
/// default:
///
/// ```
/// use yorkie_core::{Document, DocumentOptions};
///
/// let options = DocumentOptions {
///     max_undo_depth: Some(5),
///     ..Default::default()
/// };
/// let doc = Document::with_options("doc", options).unwrap();
/// # let _ = doc;
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentOptions {
    /// How many changes `history_mut().undo()` can go back, and likewise for
    /// `redo()` (JS `maxUndoDepth`). Each `Document::update` that changes the
    /// document is one entry; the oldest is dropped when the stack is full.
    /// `None` means 50 (JS `MaxUndoRedoStackDepth`). Must be positive. A
    /// reverse entry keeps a copy of the content it would restore, so a
    /// larger depth holds more memory.
    pub max_undo_depth: Option<usize>,
    /// Turns garbage collection off for this document (JS `disableGC`):
    /// `Document::garbage_collect` purges nothing and returns 0, so removed
    /// elements stay as tombstones. It is local only; declaring an attachment
    /// GC-free to the server is `AttachOptions::disable_gc` in the client,
    /// and neither implies the other.
    pub disable_gc: bool,
}
