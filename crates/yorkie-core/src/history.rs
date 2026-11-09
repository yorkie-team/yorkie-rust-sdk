//! Undo and redo, mirroring JS `document/history.ts` and the `history`
//! object of `document/document.ts`.
//!
//! `History` holds the stacks. Each entry is the reverse of one local change:
//! its reverse operations, newest first, plus the previous values of the
//! presence keys the change set with `PresenceProxy::set_with_history`.
//! `Document::history` and `Document::history_mut` hand out the handles that
//! JS exposes as `doc.history`.
//!
//! A Text edit reverses by identity, as in JS: its undo revives the removed
//! pieces and re-removes the inserted one under their original IDs (see
//! `EditOperation`). A Tree edit reverses by identity where its restore spans
//! describe it whole, and otherwise by a split, a boundary deletion or a copy
//! (see `TreeEditOperation`); a Tree style reverses to the previous values.

use crate::crdt::tree::TreeNodeId;
use crate::operation::Operation;
use crate::presence::ReversePresence;
use crate::{Document, Result, TimeTicket};

/// How many entries each of the undo and redo stacks keeps before dropping
/// the oldest, unless the document sets `DocumentOptions::max_undo_depth`
/// (JS `MaxUndoRedoStackDepth`).
pub(crate) const MAX_UNDO_REDO_STACK_DEPTH: usize = 50;

/// One element of a history entry (JS `HistoryOperation`): a reverse
/// operation, or the presence values to restore.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HistoryOperation {
    /// Boxed: a tree edit makes `Operation` large, and an entry holds many.
    Operation(Box<Operation>),
    Presence(ReversePresence),
}

impl HistoryOperation {
    /// Builds a history entry from a change's reverse operations and reverse
    /// presence, as JS `Document.update` does: every reverse, newest first,
    /// then the presence values.
    pub(crate) fn entry(
        reverse_ops: Vec<Operation>,
        reverse_presence: Option<ReversePresence>,
    ) -> Vec<Self> {
        reverse_ops
            .into_iter()
            .map(|op| Self::Operation(Box::new(op)))
            .chain(reverse_presence.map(Self::Presence))
            .collect()
    }

    /// Returns the JS `toStringHistoryOp` form of this element.
    #[cfg(test)]
    pub(crate) fn to_test_string(&self) -> String {
        match self {
            Self::Operation(op) => op.to_test_string(),
            Self::Presence(value) => format!("presence:{value:?}"),
        }
    }
}

/// The undo and redo stacks of a document (JS `History`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct History {
    undo_stack: Vec<Vec<HistoryOperation>>,
    redo_stack: Vec<Vec<HistoryOperation>>,
    /// How many entries each stack keeps before dropping the oldest.
    max_depth: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(MAX_UNDO_REDO_STACK_DEPTH)
    }
}

impl History {
    /// Creates empty stacks that each keep `max_depth` entries (JS
    /// `new History(maxDepth)`). The document validates the depth.
    pub(crate) fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth,
        }
    }

    /// Returns how many entries each of the undo and redo stacks keeps
    /// before dropping the oldest (JS `getMaxDepth`).
    #[cfg(test)]
    pub(crate) fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub(crate) fn has_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub(crate) fn has_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Pushes an undo entry, dropping the oldest one at the depth limit.
    pub(crate) fn push_undo(&mut self, ops: Vec<HistoryOperation>) {
        push_bounded(&mut self.undo_stack, ops, self.max_depth);
    }

    pub(crate) fn pop_undo(&mut self) -> Option<Vec<HistoryOperation>> {
        self.undo_stack.pop()
    }

    /// Pushes a redo entry, dropping the oldest one at the depth limit.
    pub(crate) fn push_redo(&mut self, ops: Vec<HistoryOperation>) {
        push_bounded(&mut self.redo_stack, ops, self.max_depth);
    }

    pub(crate) fn pop_redo(&mut self) -> Option<Vec<HistoryOperation>> {
        self.redo_stack.pop()
    }

    pub(crate) fn clear_redo(&mut self) {
        self.redo_stack.clear();
    }

    pub(crate) fn clear_undo(&mut self) {
        self.undo_stack.clear();
    }

    #[cfg(test)]
    pub(crate) fn undo_stack(&self) -> &[Vec<HistoryOperation>] {
        &self.undo_stack
    }

    #[cfg(test)]
    pub(crate) fn redo_stack(&self) -> &[Vec<HistoryOperation>] {
        &self.redo_stack
    }

    /// Re-points the entries of both stacks that name `prev` to `curr` (JS
    /// `reconcileCreatedAt`).
    ///
    /// An array set, or an undo that adds a removed element back, gives the
    /// element a new creation time. Entries recorded against the old one have
    /// to follow it.
    pub(crate) fn reconcile_created_at(&mut self, prev: &TimeTicket, curr: &TimeTicket) {
        for op in self.operations_mut() {
            match op {
                Operation::ArraySet(op) if op.created_at() == prev => {
                    op.set_created_at(curr.clone());
                }
                Operation::Remove(op) if op.created_at() == prev => {
                    op.set_created_at(curr.clone());
                }
                Operation::Add(op) if op.prev_created_at() == prev => {
                    op.set_prev_created_at(curr.clone());
                }
                Operation::Move(op) => {
                    if op.created_at() == prev {
                        op.set_created_at(curr.clone());
                    }
                    if op.prev_created_at() == prev {
                        op.set_prev_created_at(curr.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Shifts the undo text edits of both stacks on the text `parent_created_at`
    /// past a remote edit that replaced `from..to` with `content_len` UTF-16
    /// code units (JS `reconcileTextEdit`).
    pub(crate) fn reconcile_text_edit(
        &mut self,
        parent_created_at: &TimeTicket,
        from: usize,
        to: usize,
        content_len: usize,
    ) {
        for op in self.operations_mut() {
            if let Operation::Edit(op) = op {
                if op.parent_created_at() == parent_created_at {
                    op.reconcile_operation(from, to, content_len);
                }
            }
        }
    }

    /// Shifts the undo tree edits of both stacks on the tree
    /// `parent_created_at` past an edit that replaced `from..to` with
    /// `content_size` (JS `reconcileTreeEdit`). Identity edits locate their
    /// nodes by ID and are left alone (see
    /// `TreeEditOperation::reconcile_operation`).
    pub(crate) fn reconcile_tree_edit(
        &mut self,
        parent_created_at: &TimeTicket,
        from: usize,
        to: usize,
        content_size: usize,
    ) {
        for op in self.operations_mut() {
            if let Operation::TreeEdit(op) = op {
                if op.parent_created_at() == parent_created_at {
                    op.reconcile_operation(from, to, content_size);
                }
            }
        }
    }

    /// Re-points the tree edits and styles of both stacks on the tree
    /// `parent_created_at` from the node `prev` to the node `curr` (JS
    /// `reconcileTreeNodeID`), for a split that re-created a merged-away
    /// element under a new ID.
    ///
    /// Scoped to one tree, as `reconcile_tree_edit` is: a node ID is unique
    /// only within its own tree, and the pairs come from peers' splits as
    /// well as this replica's, so an unscoped sweep would let a split in one
    /// tree re-address entries recorded against another.
    pub(crate) fn reconcile_tree_node_id(
        &mut self,
        parent_created_at: &TimeTicket,
        prev: &TreeNodeId,
        curr: &TreeNodeId,
    ) {
        for op in self.operations_mut() {
            match op {
                Operation::TreeEdit(op) if op.parent_created_at() == parent_created_at => {
                    op.reconcile_node_id(prev, curr);
                }
                Operation::TreeStyle(op) if op.parent_created_at() == parent_created_at => {
                    op.reconcile_node_id(prev, curr);
                }
                _ => {}
            }
        }
    }

    fn operations_mut(&mut self) -> impl Iterator<Item = &mut Operation> {
        self.undo_stack
            .iter_mut()
            .chain(self.redo_stack.iter_mut())
            .flatten()
            .filter_map(|op| match op {
                HistoryOperation::Operation(op) => Some(op.as_mut()),
                HistoryOperation::Presence(_) => None,
            })
    }
}

fn push_bounded(
    stack: &mut Vec<Vec<HistoryOperation>>,
    ops: Vec<HistoryOperation>,
    max_depth: usize,
) {
    if stack.len() >= max_depth {
        stack.remove(0);
    }
    stack.push(ops);
}

/// Read-only view of a document's history, from `Document::history` (JS
/// `doc.history.canUndo`/`canRedo`).
#[derive(Debug, Clone, Copy)]
pub struct HistoryRef<'a> {
    doc: &'a Document,
}

impl<'a> HistoryRef<'a> {
    pub(crate) fn new(doc: &'a Document) -> Self {
        Self { doc }
    }

    /// Returns whether there is a change to undo (JS `canUndo`).
    pub fn can_undo(&self) -> bool {
        self.doc.history_stacks().has_undo()
    }

    /// Returns whether there is an undone change to redo (JS `canRedo`).
    pub fn can_redo(&self) -> bool {
        self.doc.history_stacks().has_redo()
    }
}

/// Mutable handle on a document's history, from `Document::history_mut`
/// (JS `doc.history`).
#[derive(Debug)]
pub struct HistoryMut<'a> {
    doc: &'a mut Document,
}

impl<'a> HistoryMut<'a> {
    pub(crate) fn new(doc: &'a mut Document) -> Self {
        Self { doc }
    }

    /// Returns whether there is a change to undo (JS `canUndo`).
    pub fn can_undo(&self) -> bool {
        self.doc.history_stacks().has_undo()
    }

    /// Returns whether there is an undone change to redo (JS `canRedo`).
    pub fn can_redo(&self) -> bool {
        self.doc.history_stacks().has_redo()
    }

    /// Reverts the latest local change that is not undone yet (JS `undo`).
    ///
    /// The undo is a new local change: it is queued for the server and
    /// publishes a `LocalChange` event like any other. With nothing to undo it
    /// does nothing, as in JS.
    pub fn undo(&mut self) -> Result<()> {
        self.doc.execute_undo_redo(true)
    }

    /// Reapplies the latest undone change (JS `redo`). Like `undo`, it is a
    /// new local change, and does nothing when there is nothing to redo.
    pub fn redo(&mut self) -> Result<()> {
        self.doc.execute_undo_redo(false)
    }
}

#[cfg(test)]
mod depth_tests;
#[cfg(test)]
mod tests;
