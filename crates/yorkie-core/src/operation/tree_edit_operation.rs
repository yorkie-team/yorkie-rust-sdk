use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta, RestoreMode};
use crate::crdt::element::DataSize;
use crate::crdt::root::CrdtRoot;
use crate::crdt::tree::{
    fallback_split_tickets, replace_tree_node_id, CrdtTree, TreeEditChange, TreeEditResult,
    TreeNode, TreeNodeId, TreePos, TreeRestoreSpan,
};
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};
use std::collections::BTreeSet;

/// What one run of a Tree edit measured, for the undo stack (JS keeps these
/// on the operation, which its `execute` mutates). `Change` attaches them to
/// the copy of the operation it records; they are never encoded and take no
/// part in the operation's equality.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TreeEditExecution {
    /// The visible index of the start of the range before the edit (JS
    /// `lastFromIdx`). Unset for an identity edit, which reports
    /// `executed_ranges` instead.
    pub(crate) last_from_idx: Option<usize>,
    /// `last_from_idx` plus the padded size of the nodes the range marked for
    /// removal and of the subtrees the merge stranded, read after the edit
    /// (JS `lastToIdx`).
    pub(crate) last_to_idx: Option<usize>,
    /// The size of the content the tree accepted (JS `insertedContentSize`).
    pub(crate) inserted_content_size: Option<usize>,
    /// The visible size of the boundaries the split opened (JS `splitSize`).
    pub(crate) split_size: usize,
    /// What an identity edit changed, one `(from, to, inserted size)` per
    /// node that left or came back, in the order it did (JS
    /// `executedRanges`).
    pub(crate) executed_ranges: Option<Vec<(usize, usize, usize)>>,
    /// The `(removed element, its replacement)` pairs the split re-created
    /// (JS `splitRecreatedIDs`).
    pub(crate) split_recreated_ids: Vec<(TreeNodeId, TreeNodeId)>,
    /// How many of the carried split tickets the split took (JS
    /// `consumedSplitTickets`).
    pub(crate) consumed_split_tickets: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct TreeEditOperation {
    meta: OperationMeta,
    from_pos: TreePos,
    to_pos: TreePos,
    contents: Option<Vec<TreeNode>>,
    split_level: usize,
    /// The tickets the editing replica issued for the elements its split
    /// created, in issue order (JS `splitTickets`). Empty for a change
    /// written before the field existed, which falls back to rebuilding
    /// them from `executed_at`.
    split_tickets: Vec<TimeTicket>,
    /// Set on the reverse of an edit, which an undo or a redo runs (JS
    /// `isUndoOp`). Local only.
    is_undo_op: bool,
    /// The visible range of an undo edit, which reconciliation shifts and
    /// which takes precedence over the positions when it runs (JS
    /// `fromIdx`/`toIdx`). Local only.
    from_idx: Option<usize>,
    to_idx: Option<usize>,
    /// The nodes the reversed edit removed, by identity (JS
    /// `restoreSpans`). JS tells an absent list from an empty one nowhere it
    /// reads them, so an empty `Vec` stands for both.
    restore_spans: Vec<TreeRestoreSpan>,
    /// Set on an identity edit, which then runs by `restore_spans` and
    /// `retombstone_spans` alone (JS `restoreMode`).
    restore_mode: Option<RestoreMode>,
    /// The nodes the reversed edit inserted, by identity (JS
    /// `retombstoneSpans`).
    retombstone_spans: Vec<TreeRestoreSpan>,
    /// Set on the boundary deletion that undoes a split, so its own reverse
    /// splits again rather than reinserting the boundary nodes as content
    /// (JS `redoSplitLevel`). Local only.
    redo_split_level: Option<usize>,
    /// The elements a merge removed that this split re-creates, innermost
    /// first, the order the split mints their replacements in (JS
    /// `replacedIDs`). Local only.
    replaced_ids: Vec<TreeNodeId>,
    /// What the run that recorded this copy measured. Local only.
    execution: Option<TreeEditExecution>,
}

/// Compares the fields the wire carries only: the local undo state and the
/// execution facts stay off the wire, so a decoded operation still equals
/// the one recorded. JS writes a restore mode only with spans, so a mode
/// without them is not compared either.
impl PartialEq for TreeEditOperation {
    fn eq(&self, other: &Self) -> bool {
        self.meta == other.meta
            && self.from_pos == other.from_pos
            && self.to_pos == other.to_pos
            && self.contents == other.contents
            && self.split_level == other.split_level
            && self.split_tickets == other.split_tickets
            && self.restore_spans == other.restore_spans
            && self.retombstone_spans == other.retombstone_spans
            && self.encoded_restore_mode() == other.encoded_restore_mode()
    }
}

impl TreeEditOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        from_pos: TreePos,
        to_pos: TreePos,
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            from_pos,
            to_pos,
            contents,
            split_level,
            split_tickets: Vec::new(),
            is_undo_op: false,
            from_idx: None,
            to_idx: None,
            restore_spans: Vec::new(),
            restore_mode: None,
            retombstone_spans: Vec::new(),
            redo_split_level: None,
            replaced_ids: Vec::new(),
            execution: None,
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        from_pos: TreePos,
        to_pos: TreePos,
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(
            parent_created_at,
            from_pos,
            to_pos,
            contents,
            split_level,
            executed_at,
        )
    }

    /// Makes this the reverse of an edit, over the visible range
    /// `from_idx..to_idx` (the `isUndoOp`, `fromIdx` and `toIdx` arguments
    /// of JS `TreeEditOperation.create`).
    pub(crate) fn into_undo(mut self, from_idx: usize, to_idx: usize) -> Self {
        self.is_undo_op = true;
        self.from_idx = Some(from_idx);
        self.to_idx = Some(to_idx);
        self
    }

    /// Makes this an identity edit in `restore_mode` over the given spans
    /// (the last three arguments of JS `TreeEditOperation.create`).
    pub(crate) fn with_identity(
        mut self,
        restore_spans: Vec<TreeRestoreSpan>,
        restore_mode: RestoreMode,
        retombstone_spans: Vec<TreeRestoreSpan>,
    ) -> Self {
        self.restore_spans = restore_spans;
        self.restore_mode = Some(restore_mode);
        self.retombstone_spans = retombstone_spans;
        self
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
        version_vector: Option<&VersionVector>,
    ) -> Result<Option<ExecutionResult>> {
        let path = root.create_path(self.parent_created_at())?;
        if root.tree_by_created_at(self.parent_created_at()).is_none() {
            return Err(self.tree_parent_error(root));
        }

        // As in JS, an identity edit runs by its spans alone and never
        // resolves its indexes.
        if let Some(mode) = self.restore_mode {
            return self.execute_identity(root, mode, path).map(Some);
        }

        let executed_at = self.executed_at()?.clone();
        let (result, consumed_split_tickets, facts, reverse_op) = {
            let tree = self.tree_mut(root)?;
            let range = self.undo_range(tree)?;

            // The split takes the carried tickets in order, then the ones a
            // replica rebuilds from the edit and its top-level contents.
            let mut consumed_split_tickets = 0;
            let result = {
                let mut split_tickets = self.split_tickets.iter().cloned();
                let mut fallback =
                    fallback_split_tickets(&executed_at, self.contents.as_ref().map(Vec::len));
                let mut issue_split_ticket = || match split_tickets.next() {
                    Some(ticket) => {
                        consumed_split_tickets += 1;
                        ticket
                    }
                    None => fallback(),
                };
                tree.edit(
                    range.clone(),
                    self.contents.clone(),
                    self.split_level,
                    executed_at,
                    &mut issue_split_ticket,
                    version_vector,
                )?
            };

            // Both are read off the tree the edit left, and both are kept
            // back until the root has booked what the edit changed, so a
            // failure leaves the document's sizes right.
            let facts = removed_size(tree, &result);
            let reverse_op = self.reverse_of(tree, &range, &result);
            (result, consumed_split_tickets, facts, reverse_op)
        };

        root.acc(result.diff);
        for (child_id, child_size, removed_at) in &result.gc_pairs {
            root.register_gc_pair_by_id(child_id.clone(), *child_size, removed_at.clone());
        }

        let removed_size = facts?;
        // JS has no catch here: a reverse that cannot be built fails the
        // edit, so a local edit never loses its undo entry without notice.
        // A remote change is the exception. Its reverse is measured on this
        // replica's tree from indexes taken before the edit, which a
        // concurrent merge can leave outside it, and is thrown away anyway;
        // it is dropped rather than refusing a change the caller never
        // controlled (see the porting gaps).
        let reverse_op = match reverse_op {
            Ok(reverse_op) => reverse_op,
            Err(_) if source == OpSource::Remote => None,
            Err(error) => return Err(error),
        };

        Ok(Some(ExecutionResult {
            op_infos: changes_to_op_infos(&path, result.changes, None),
            reverse_op,
            tree_edit: Some(TreeEditExecution {
                last_from_idx: Some(result.from_idx),
                last_to_idx: Some(result.from_idx + removed_size),
                inserted_content_size: Some(result.inserted_size),
                split_size: result.split_size,
                executed_ranges: None,
                split_recreated_ids: Vec::new(),
                consumed_split_tickets,
            }),
        }))
    }

    /// Runs an identity edit (JS `execute`, the restore-mode branch): the
    /// spans to re-remove go first, then the spans to revive, each by the
    /// identities the reversed edit recorded, never as fresh copies, so
    /// concurrent undos of one deletion converge rather than duplicate.
    fn execute_identity(
        &self,
        root: &mut CrdtRoot,
        mode: RestoreMode,
        path: String,
    ) -> Result<ExecutionResult> {
        let executed_at = self.executed_at()?.clone();
        let is_retombstone = mode == RestoreMode::Retombstone;
        let (to_restore, to_retombstone) = if is_retombstone {
            (&self.retombstone_spans, &self.restore_spans)
        } else {
            (&self.restore_spans, &self.retombstone_spans)
        };
        let mut diff = DataSize::default();

        // 1. Re-remove by identity. Isolating a straddling piece splits it,
        // and the split's overhead goes to live.
        let retombstoned = self
            .tree_mut(root)?
            .retombstone(to_retombstone, &executed_at)?;
        add_data_size(&mut diff, retombstoned.diff);
        for (child_id, child_size, removed_at) in retombstoned.gc_pairs {
            root.register_gc_pair_by_id(child_id, child_size, removed_at);
        }

        // 2. Revive by identity. The pending pairs go first: a revived piece
        // may be one split off a tombstone, registered only now, and the
        // unregister below finds it only once it is. Recreated nodes are
        // new, so their size goes to live.
        //
        // JS `unregisterGCPair` also moves the revived size back to live. A
        // Tree removal here never took it out of live (see
        // `CrdtRoot::unregister_gc_pair_by_id`), so live keeps it already.
        let restored = self.tree_mut(root)?.restore(to_restore)?;
        for (child_id, child_size, removed_at) in restored.gc_pairs {
            root.register_gc_pair_by_id(child_id, child_size, removed_at);
        }
        for node in &restored.untombstoned {
            root.unregister_gc_pair_by_id(&node.id.to_id_string(), node.size);
        }
        add_data_size(&mut diff, restored.diff);
        for node in &restored.recreated {
            add_data_size(&mut diff, node.size);
        }
        root.acc(diff);

        // One op info per node that left or came back, in the order it did,
        // so an editor can apply them one after another. Where they landed
        // is also what the undo stack is reconciled against: the stored
        // indexes describe the edit this one reverses, which never moves.
        let edits = retombstoned
            .changes
            .into_iter()
            .chain(restored.changes)
            .collect::<Vec<_>>();
        let executed_ranges = edits
            .iter()
            .map(|edit| (edit.change.from, edit.change.to, edit.inserted_size))
            .collect();
        let op_infos = changes_to_op_infos(
            &path,
            edits.into_iter().map(|edit| edit.change).collect(),
            Some(0),
        );

        // The reverse keeps the spans and flips the direction.
        let mut reverse = Self::create(
            self.parent_created_at().clone(),
            self.from_pos.clone(),
            self.to_pos.clone(),
            None,
            0,
            None,
        )
        .with_identity(
            self.restore_spans.clone(),
            mode.flipped(),
            self.retombstone_spans.clone(),
        );
        reverse.is_undo_op = true;
        reverse.from_idx = self.from_idx;
        reverse.to_idx = self.to_idx;

        Ok(ExecutionResult {
            op_infos,
            reverse_op: Some(Operation::TreeEdit(reverse)),
            tree_edit: Some(TreeEditExecution {
                executed_ranges: Some(executed_ranges),
                ..TreeEditExecution::default()
            }),
        })
    }

    pub(crate) fn parent_created_at(&self) -> &TimeTicket {
        self.meta.parent_created_at()
    }

    pub(crate) fn executed_at(&self) -> Result<&TimeTicket> {
        self.meta.executed_at()
    }

    pub(crate) fn range_from(&self) -> &TreePos {
        &self.from_pos
    }

    pub(crate) fn to_pos(&self) -> &TreePos {
        &self.to_pos
    }

    pub(crate) fn contents(&self) -> Option<&[TreeNode]> {
        self.contents.as_deref()
    }

    pub(crate) fn split_level(&self) -> usize {
        self.split_level
    }

    /// Returns the tickets issued for the elements the split created (JS
    /// `getSplitTickets`).
    pub(crate) fn split_tickets(&self) -> &[TimeTicket] {
        &self.split_tickets
    }

    /// Records the tickets issued for the elements the split created (JS
    /// `setSplitTickets`).
    pub(crate) fn set_split_tickets(&mut self, split_tickets: Vec<TimeTicket>) {
        self.split_tickets = split_tickets;
    }

    pub(crate) fn is_undo_op(&self) -> bool {
        self.is_undo_op
    }

    /// Returns the stored visible range of an undo edit (JS
    /// `fromIdx`/`toIdx`).
    pub(crate) fn undo_indexes(&self) -> Option<(usize, usize)> {
        self.from_idx.zip(self.to_idx)
    }

    /// Returns the nodes the reversed edit removed, by identity (JS
    /// `getRestoreSpans`).
    pub(crate) fn restore_spans(&self) -> &[TreeRestoreSpan] {
        &self.restore_spans
    }

    /// Returns the direction of an identity edit (JS `getRestoreMode`).
    pub(crate) fn restore_mode(&self) -> Option<RestoreMode> {
        self.restore_mode
    }

    /// Returns the nodes the reversed edit inserted, by identity (JS
    /// `getRetombstoneSpans`).
    pub(crate) fn retombstone_spans(&self) -> &[TreeRestoreSpan] {
        &self.retombstone_spans
    }

    /// Returns the split level the redo of this boundary deletion splits
    /// at (JS `redoSplitLevel`).
    pub(crate) fn redo_split_level(&self) -> Option<usize> {
        self.redo_split_level
    }

    /// Returns the elements this split re-creates, innermost first (JS
    /// `getReplacedIDs`). Empty unless it is the redo of a split or the
    /// undo of a merge.
    pub(crate) fn replaced_ids(&self) -> &[TreeNodeId] {
        &self.replaced_ids
    }

    /// Returns what the run that recorded this copy measured.
    pub(crate) fn execution(&self) -> Option<&TreeEditExecution> {
        self.execution.as_ref()
    }

    /// Keeps what a run measured on the copy `Change` records.
    pub(crate) fn set_execution(&mut self, execution: TreeEditExecution) {
        self.execution = Some(execution);
    }

    /// Returns the `(removed element, its replacement)` pairs the last run's
    /// split re-created (JS `getSplitRecreatedIDs`).
    pub(crate) fn split_recreated_ids(&self) -> &[(TreeNodeId, TreeNodeId)] {
        self.execution
            .as_ref()
            .map_or(&[], |execution| &execution.split_recreated_ids)
    }

    /// Returns how many carried split tickets the last run took (JS
    /// `getConsumedSplitTicketCount`), zero when it never ran.
    pub(crate) fn consumed_split_tickets(&self) -> usize {
        self.execution
            .as_ref()
            .map_or(0, |execution| execution.consumed_split_tickets)
    }

    pub(crate) fn set_executed_at(&mut self, executed_at: TimeTicket) {
        self.meta.set_executed_at(executed_at);
    }

    /// Sets the actor of this operation and of the tickets its split issued,
    /// which carry the actor of the change (JS `setActor`).
    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();
        for ticket in &mut self.split_tickets {
            *ticket = ticket.set_actor(actor_id.clone());
        }
        self.meta.set_actor(actor_id);
    }

    /// Resolves an undo edit's stored indexes into the positions it runs
    /// at, and keeps them, so the change carries them to other replicas.
    ///
    /// JS does this inside `execute` and keeps the result on the operation.
    /// `execute` takes `&self` here, so `Change` calls this first. These are
    /// the document's own indexes, which reconciliation and the edits since
    /// can move inside a surrogate pair, so they resolve without the pair
    /// check. An identity edit is left alone, as in JS.
    pub(crate) fn resolve_undo_positions(&mut self, root: &CrdtRoot) -> Result<()> {
        if self.restore_mode.is_some() {
            return Ok(());
        }
        let tree = root
            .tree_by_created_at(self.parent_created_at())
            .ok_or_else(|| self.tree_parent_error(root))?;
        (self.from_pos, self.to_pos) = self.undo_range(tree)?;
        Ok(())
    }

    /// Returns the visible range of this edit (JS `normalizePos`): the
    /// stored, possibly reconciled, indexes of an undo edit, else the range
    /// the last run measured before it edited.
    pub(crate) fn normalize_pos(&self) -> (usize, usize) {
        if let Some(indexes) = self.is_undo_op.then(|| self.undo_indexes()).flatten() {
            return indexes;
        }
        self.execution
            .as_ref()
            .and_then(|execution| execution.last_from_idx.zip(execution.last_to_idx))
            .unwrap_or((0, 0))
    }

    /// Returns the visible size this edit adds (JS `getContentSize`): once
    /// it ran, the content the tree accepted plus the boundaries its split
    /// opened, since content reusing an ID is dropped and counting it would
    /// shift the undo stack past content never inserted.
    pub(crate) fn content_size(&self) -> usize {
        if let Some(execution) = &self.execution {
            if let Some(inserted) = execution.inserted_content_size {
                return inserted + execution.split_size;
            }
        }
        self.contents
            .iter()
            .flatten()
            .map(|node| node.padded_size(false))
            .sum()
    }

    /// Returns the visible ranges this run replaced, each with the size it
    /// inserted there, in the order they applied (JS `getExecutedRanges`):
    /// one per node an identity edit moved, else the single normalized
    /// range.
    pub(crate) fn executed_ranges(&self) -> Vec<(usize, usize, usize)> {
        if let Some(ranges) = self
            .execution
            .as_ref()
            .and_then(|execution| execution.executed_ranges.as_ref())
        {
            return ranges.clone();
        }
        let (from, to) = self.normalize_pos();
        vec![(from, to, self.content_size())]
    }

    /// Shifts an undo edit's stored range past a remote edit that replaced
    /// `remote_from..remote_to` with `content_len` (JS
    /// `reconcileOperation`), by the six overlap cases Text uses. An
    /// identity edit locates its nodes by ID, so it is left alone, as is an
    /// edit that is not an undo or has no stored range.
    pub(crate) fn reconcile_operation(
        &mut self,
        remote_from: usize,
        remote_to: usize,
        content_len: usize,
    ) {
        if !self.is_undo_op || self.restore_mode.is_some() || remote_from > remote_to {
            return;
        }
        let Some((local_from, local_to)) = self.undo_indexes() else {
            return;
        };

        let remote_len = remote_to - remote_from;
        // JS clamps a negative result to zero.
        let shift = |index: usize| (index + content_len).saturating_sub(remote_len);

        let (from, to) = if remote_to <= local_from {
            // 1. The remote edit is left of the range.
            (shift(local_from), shift(local_to))
        } else if local_to <= remote_from {
            // 2. The remote edit is right of the range.
            return;
        } else if remote_from <= local_from && local_to <= remote_to && remote_from != remote_to {
            // 3. The range lies inside the remote edit.
            (remote_from, remote_from)
        } else if local_from <= remote_from && remote_to <= local_to && local_from != local_to {
            // 4. The remote edit lies inside the range.
            (local_from, shift(local_to))
        } else if remote_from < local_from && local_from < remote_to && remote_to < local_to {
            // 5. The remote edit overlaps the start of the range.
            (remote_from, remote_from + (local_to - remote_to))
        } else if local_from < remote_from && remote_from < local_to && local_to < remote_to {
            // 6. The remote edit overlaps the end of the range.
            (local_from, remote_from)
        } else {
            return;
        };

        self.from_idx = Some(from);
        self.to_idx = Some(to);
    }

    /// Gives every node this edit inserts a fresh identity, and returns the
    /// `(old, new)` pairs (JS `reissueContentIDs`).
    ///
    /// A copy-reinsert reverse carries the original IDs of the nodes it
    /// copied, and inserting it would put two nodes under one ID. The fresh
    /// IDs come from the change the undo creates, in JS's post-order. An
    /// identity edit revives nodes by identity by design and is left alone.
    /// A splitting edit is refused: its split tickets, rebuilt from
    /// `executed_at` when none are carried, would overlap the reissued ones.
    pub(crate) fn reissue_content_ids(
        &mut self,
        issue_time_ticket: &mut dyn FnMut() -> TimeTicket,
    ) -> Result<Vec<(TreeNodeId, TreeNodeId)>> {
        let mut reissued = Vec::new();
        if self.restore_mode.is_some() {
            return Ok(reissued);
        }
        let Some(contents) = &mut self.contents else {
            return Ok(reissued);
        };
        if self.split_level != 0 {
            return Err(YorkieError::Refused(
                "cannot reissue content ids on a splitting edit".to_owned(),
            ));
        }

        for content in contents {
            content.for_each_post_order_mut(&mut |node| {
                let prev = node.id().clone();
                let curr = TreeNodeId::new(issue_time_ticket(), 0);
                node.set_id(curr.clone());
                reissued.push((prev, curr));
                // A fresh identity has to be fresh in every field that
                // names a node: the split chain and merge lineage of the
                // copied node would splice this one into a chain it never
                // belonged to.
                node.set_ins_prev_id(None);
                node.set_ins_next_id(None);
                node.set_merged_from(None);
                node.set_merged_at(None);
                node.set_merged_into(None);
            });
        }
        Ok(reissued)
    }

    /// Points this edit at `curr` wherever it named `prev` (JS
    /// `reconcileNodeID`): its range, every ID its spans carry (the span's
    /// own ID too, which restore looks the node up and recreates it by), and
    /// the elements it re-creates.
    pub(crate) fn reconcile_node_id(&mut self, prev: &TreeNodeId, curr: &TreeNodeId) {
        self.from_pos = self.from_pos.replace_node_id(prev, curr);
        self.to_pos = self.to_pos.replace_node_id(prev, curr);

        let replace = |id: &TreeNodeId| replace_tree_node_id(id, prev, curr);
        for span in self
            .restore_spans
            .iter_mut()
            .chain(self.retombstone_spans.iter_mut())
        {
            span.id = replace(&span.id);
            span.parent_id = span.parent_id.as_ref().map(replace);
            span.left_sibling_id = span.left_sibling_id.as_ref().map(replace);
            span.right_sibling_id = span.right_sibling_id.as_ref().map(replace);
        }
        for id in &mut self.replaced_ids {
            *id = replace(id);
        }
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.TREE_EDIT({},{},split={})",
            self.parent_created_at().to_test_string(),
            self.from_pos.to_test_string(),
            self.to_pos.to_test_string(),
            self.split_level
        )
    }

    /// The restore mode the wire carries: JS writes it only with spans.
    fn encoded_restore_mode(&self) -> Option<RestoreMode> {
        self.restore_mode
            .filter(|_| !self.restore_spans.is_empty() || !self.retombstone_spans.is_empty())
    }

    /// Returns the range this edit runs at: an undo edit's stored indexes,
    /// resolved without the surrogate pair check, else its positions (JS
    /// `execute`, before the edit).
    fn undo_range(&self, tree: &CrdtTree) -> Result<(TreePos, TreePos)> {
        let Some((from_idx, to_idx)) = self.is_undo_op.then(|| self.undo_indexes()).flatten()
        else {
            return Ok((self.from_pos.clone(), self.to_pos.clone()));
        };
        let from_pos = tree.find_pos_unchecked(from_idx, true)?;
        let to_pos = if from_idx == to_idx {
            from_pos.clone()
        } else {
            tree.find_pos_unchecked(to_idx, true)?
        };
        Ok((from_pos, to_pos))
    }

    /// Builds the reverse of a positional edit (JS `execute`, after the
    /// edit): an edit that does not split reverses through
    /// `to_reverse_operation`, a pure split through
    /// `to_split_reverse_operation`, and any other split has none.
    ///
    /// As in JS, a pure split carries no content and the range it was
    /// given marked nothing for removal; nodes already removed count.
    fn reverse_of(
        &self,
        tree: &CrdtTree,
        range: &(TreePos, TreePos),
        result: &TreeEditResult,
    ) -> Result<Option<Operation>> {
        if self.split_level == 0 {
            return self.to_reverse_operation(tree, range, result);
        }
        let is_pure_split =
            self.contents.as_ref().is_none_or(Vec::is_empty) && result.removal_targets.is_empty();
        if is_pure_split {
            return self.to_split_reverse_operation(tree, result.from_idx, result.split_size);
        }
        Ok(None)
    }

    /// Builds the reverse of an edit that does not split, on the tree after
    /// it, from the index its range started at before it (JS
    /// `toReverseOperation`). In order:
    ///
    /// 1. An edit the captured spans describe whole reverses by identity,
    ///    unless it is the boundary deletion undoing a split.
    /// 2. That boundary deletion reverses by splitting again.
    /// 3. A merge reverses by a split re-creating the merged elements.
    /// 4. Otherwise a copy of what it removed is inserted again over what it
    ///    inserted, unless what it inserted runs past the tree, which marks
    ///    an edit that had no visible effect.
    ///
    /// The reverse is built for remote changes too, from indexes on this
    /// replica's tree, so it skips the surrogate pair check (JS
    /// `findPosUnchecked`).
    fn to_reverse_operation(
        &self,
        tree: &CrdtTree,
        range: &(TreePos, TreePos),
        result: &TreeEditResult,
    ) -> Result<Option<Operation>> {
        let from_idx = result.from_idx;
        if self.redo_split_level.is_none()
            && (!result.removed_spans.is_empty() || !result.inserted_spans.is_empty())
        {
            return Ok(Some(Operation::TreeEdit(
                self.reverse_create(range.0.clone(), range.1.clone(), None, 0)
                    .into_undo(from_idx, from_idx)
                    .with_identity(
                        result.removed_spans.clone(),
                        RestoreMode::Restore,
                        result.inserted_spans.clone(),
                    ),
            )));
        }

        let split_level = match self.redo_split_level.filter(|level| *level > 0) {
            Some(level) => level,
            None => result.merge_level,
        };
        if split_level > 0 {
            let pos = tree.find_pos_unchecked(from_idx, true)?;
            let mut split = self
                .reverse_create(pos.clone(), pos, None, split_level)
                .into_undo(from_idx, from_idx);
            split.replaced_ids = merged_away_ids(tree, &result.merged_nodes)?;
            return Ok(Some(Operation::TreeEdit(split)));
        }

        // The size the tree accepted, measured before the insert: content
        // inserted under a removed parent is born removed and reads back as
        // nothing, so a range ending past the tree marks an edit that had no
        // visible effect.
        let inserted_size = result.inserted_size;
        let to_idx = from_idx + inserted_size;
        if to_idx > tree.root().len() {
            return Ok(None);
        }

        // The removed nodes whose parent was not removed with them, copied
        // as the edit left them, without the nodes removed before it: those
        // record an earlier delete an undo of this one must not revive.
        let marked = result.removal_targets.iter().collect::<BTreeSet<_>>();
        let contents = result
            .removal_targets
            .iter()
            .filter(|id| !result.pre_tombstoned.contains(&id.to_id_string()))
            .filter(|id| {
                tree.parent_id(id)
                    .is_none_or(|parent| !marked.contains(&parent))
            })
            .map(|id| {
                let mut node = tree.node(id)?.clone();
                node.retain_descendants(&|child| {
                    !result.pre_tombstoned.contains(&child.id_string())
                });
                node.clear_removed_recursively();
                Ok(node)
            })
            .collect::<Result<Vec<_>>>()?;

        let from_pos = tree.find_pos_unchecked(from_idx, true)?;
        let to_pos = if inserted_size > 0 {
            tree.find_pos_unchecked(to_idx, true)?
        } else {
            from_pos.clone()
        };
        Ok(Some(Operation::TreeEdit(
            self.reverse_create(
                from_pos,
                to_pos,
                (!contents.is_empty()).then_some(contents),
                0,
            )
            .into_undo(from_idx, to_idx),
        )))
    }

    /// Builds the reverse of a pure split, the deletion of the boundaries
    /// it opened (JS `toSplitReverseOperation`). `boundary_size` is what
    /// the split opened, not twice its level: a level the tree had no room
    /// for opened nothing, and sizing the range by it would delete live
    /// content past the boundary. A split that opened nothing, or a range
    /// running past the tree, has no reverse.
    fn to_split_reverse_operation(
        &self,
        tree: &CrdtTree,
        from_idx: usize,
        boundary_size: usize,
    ) -> Result<Option<Operation>> {
        if boundary_size == 0 {
            return Ok(None);
        }
        let to_idx = from_idx + boundary_size;
        if to_idx > tree.root().len() {
            return Ok(None);
        }

        let from_pos = tree.find_pos_unchecked(from_idx, true)?;
        let to_pos = tree.find_pos_unchecked(to_idx, true)?;
        let mut deletion = self
            .reverse_create(from_pos, to_pos, None, 0)
            .into_undo(from_idx, to_idx);
        // Its own reverse, the redo, splits again rather than inserting the
        // removed boundary nodes back as content.
        deletion.redo_split_level = Some(self.split_level);
        Ok(Some(Operation::TreeEdit(deletion)))
    }

    /// Creates a reverse of this edit, its ticket assigned when it runs.
    fn reverse_create(
        &self,
        from_pos: TreePos,
        to_pos: TreePos,
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
    ) -> Self {
        Self::create(
            self.parent_created_at().clone(),
            from_pos,
            to_pos,
            contents,
            split_level,
            None,
        )
    }

    fn tree_mut<'a>(&self, root: &'a mut CrdtRoot) -> Result<&'a mut CrdtTree> {
        if root.tree_by_created_at(self.parent_created_at()).is_none() {
            return Err(self.tree_parent_error(root));
        }
        Ok(root
            .tree_by_created_at_mut(self.parent_created_at())
            .expect("tree existence checked above"))
    }

    fn tree_parent_error(&self, root: &CrdtRoot) -> YorkieError {
        if root.find_by_created_at(self.parent_created_at()).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: self.parent_created_at().to_id_string(),
                expected: "tree",
            };
        }

        YorkieError::MissingCrdtElement(self.parent_created_at().to_id_string())
    }
}

/// Returns the padded size, on the tree the edit left, of every node its
/// range marked for removal and of every subtree its merge stranded (JS
/// `execute`'s `removedSize`). As in JS, a node is measured without its own
/// tombstone, and one marked twice counts twice.
fn removed_size(tree: &CrdtTree, result: &TreeEditResult) -> Result<usize> {
    result
        .removal_targets
        .iter()
        .chain(result.stranded_nodes.iter().map(TreeNode::id))
        .map(|id| Ok(tree.node(id)?.padded_size(false)))
        .sum()
}

/// Returns the IDs of the elements a merge removed, innermost first, the
/// order a split re-creating them issues tickets in (JS `mergedAwayIDs`).
/// The depth is read off the tree, since `merged_nodes` are copies.
fn merged_away_ids(tree: &CrdtTree, merged_nodes: &[TreeNode]) -> Result<Vec<TreeNodeId>> {
    let mut by_depth = merged_nodes
        .iter()
        .filter(|node| !node.is_text())
        .map(|node| {
            let depth = tree.depth_of(node.id()).ok_or_else(|| {
                YorkieError::InvalidTreePosition(format!(
                    "merged node {} not found",
                    node.id().to_test_string()
                ))
            })?;
            Ok((depth, node.id().clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    // Stable, as JS `Array.prototype.sort` is: equal depths keep their order.
    by_depth.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
    Ok(by_depth.into_iter().map(|(_, id)| id).collect())
}

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

fn changes_to_op_infos(
    path: &str,
    changes: Vec<TreeEditChange>,
    split_level: Option<usize>,
) -> Vec<OpInfo> {
    changes
        .into_iter()
        .map(|change| OpInfo::TreeEdit {
            path: path.to_owned(),
            from: change.from,
            to: change.to,
            from_path: change.from_path,
            to_path: change.to_path,
            value: change.value,
            // JS `TreeEditOpInfo.splitLevel` exists, but no change of a
            // positional edit sets it: a split is reported as an empty change
            // where it opened. An identity edit reports zero.
            split_level,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{TreeEditExecution, TreeEditOperation};
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::operation::{ExecutionResult, OpInfo, OpSource, Operation, RestoreMode};
    use crate::{TimeTicket, VersionVector};

    #[test]
    fn inserts_tree_element_content() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), Vec::new());
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(0, true)?;

        let result = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            Some(vec![paragraph_node()]),
            0,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<root><p>hello</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(
            vec![OpInfo::TreeEdit {
                path: "$.body".to_owned(),
                from: 0,
                to: 0,
                from_path: vec![0],
                to_path: vec![0],
                value: Some(vec![crate::TreeNodeInput::element(
                    "p",
                    vec![crate::TreeNodeInput::text("hello")]
                )]),
                split_level: None,
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::TreeEdit(_))));
        Ok(())
    }

    #[test]
    fn removes_tree_element_content() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let tree = root.tree_by_created_at(&tree_at).unwrap();
        let range = (tree.path_to_pos(&[0])?, tree.path_to_pos(&[1])?);

        let result = TreeEditOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            None,
            0,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<root></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(2, root.get_garbage_len());
        assert_eq!(
            vec![OpInfo::TreeEdit {
                path: "$.body".to_owned(),
                from: 0,
                to: 7,
                from_path: vec![0],
                to_path: vec![1],
                value: None,
                split_level: None,
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::TreeEdit(_))));
        Ok(())
    }

    #[test]
    fn inserts_tree_content_inside_text() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(3, true)?;

        let result = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(4, "a"), 0),
                "X",
            )]),
            0,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<root><p>heXllo</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert!(matches!(result.reverse_op, Some(Operation::TreeEdit(_))));
        assert_eq!(1, result.op_infos.len());
        match &result.op_infos[0] {
            OpInfo::TreeEdit {
                path,
                from,
                to,
                value,
                ..
            } => {
                assert_eq!("$.body", path);
                assert_eq!(3, *from);
                assert_eq!(3, *to);
                assert_eq!(Some(vec![crate::TreeNodeInput::text("X")]), *value);
            }
            other => panic!("unexpected op info: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn removes_tree_text_range_with_splits() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.find_pos(2, true)?, tree.find_pos(5, true)?)
        };

        let result = TreeEditOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            None,
            0,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<root><p>ho</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(1, root.get_garbage_len());
        assert!(matches!(result.reverse_op, Some(Operation::TreeEdit(_))));
        assert_eq!(1, result.op_infos.len());
        match &result.op_infos[0] {
            OpInfo::TreeEdit {
                path,
                from,
                to,
                value,
                ..
            } => {
                assert_eq!("$.body", path);
                assert_eq!(2, *from);
                assert_eq!(5, *to);
                assert!(value.is_none());
            }
            other => panic!("unexpected op info: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn splits_tree_element_content() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(3, true)?;

        let result = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            None,
            1,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        let tree = root.tree_by_created_at(&tree_at).unwrap();
        assert_eq!(r#"<root><p>he</p><p>llo</p></root>"#, tree.to_xml());
        assert_eq!(5, tree.node_map_len());
        execute_tree_edit_reverse(&mut root, result.reverse_op, ticket(11, "a"))?;
        assert_eq!(
            r#"<root><p>hello</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(1, result.op_infos.len());
        match &result.op_infos[0] {
            OpInfo::TreeEdit {
                path,
                from,
                to,
                value,
                split_level,
                ..
            } => {
                assert_eq!("$.body", path);
                assert_eq!(3, *from);
                assert_eq!(3, *to);
                assert!(value.is_none());
                // As in JS, no change carries the split level.
                assert_eq!(None, *split_level);
            }
            other => panic!("unexpected op info: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn splits_tree_content_across_multiple_levels() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_nested_root(tree_at.clone());
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .path_to_pos(&[0, 0, 0, 2])?;

        let result = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            None,
            2,
            Some(ticket(20, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        execute_tree_edit_reverse(&mut root, result.reverse_op, ticket(21, "a"))?;
        assert_eq!(
            r#"<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(1, result.op_infos.len());
        match &result.op_infos[0] {
            OpInfo::TreeEdit { split_level, .. } => {
                assert_eq!(None, *split_level);
            }
            other => panic!("unexpected op info: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn merges_tree_content_between_paths() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_nested_root(tree_at.clone());
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.path_to_pos(&[0, 0, 1])?, tree.path_to_pos(&[0, 1, 0])?)
        };

        let result = TreeEditOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            None,
            0,
            Some(ticket(20, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<doc><tc><p><tn>1234</tn><tn>5678</tn></p></tc></doc>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        execute_tree_edit_reverse(&mut root, result.reverse_op.clone(), ticket(21, "a"))?;
        assert_eq!(
            r#"<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(1, result.op_infos.len());
        match &result.op_infos[0] {
            OpInfo::TreeEdit { value, .. } => {
                assert!(value.is_none());
            }
            other => panic!("unexpected op info: {other:?}"),
        }
        Ok(())
    }

    /// JS `TreeEditOperation.execute`: the elements a split creates take the
    /// tickets the operation carries, in order, then fall back to the
    /// tickets that follow the edit and its top-level contents.
    #[test]
    fn splits_with_the_tickets_the_operation_carries() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let product_ids = |split_tickets: Vec<TimeTicket>| -> crate::Result<Vec<String>> {
            let mut root = seeded_nested_root(tree_at.clone());
            let pos = root
                .tree_by_created_at(&tree_at)
                .unwrap()
                .path_to_pos(&[0, 0, 0, 2])?;
            let mut operation = TreeEditOperation::create(
                tree_at.clone(),
                pos.clone(),
                pos,
                None,
                2,
                Some(TimeTicket::new(20, 3, "b")),
            );
            operation.set_split_tickets(split_tickets);
            operation.execute(&mut root, OpSource::Remote, None)?;

            let tree = root.tree_by_created_at(&tree_at).unwrap();
            assert_eq!(
                r#"<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>"#,
                tree.to_xml()
            );
            Ok(tree
                .nodes()
                .filter(|node| node.id().created_at().lamport() == 20)
                .map(|node| node.id().created_at().to_test_string())
                .collect())
        };

        assert_eq!(
            vec!["20:b:7", "20:b:9"],
            product_ids(vec![
                TimeTicket::new(20, 9, "b"),
                TimeTicket::new(20, 7, "b")
            ])?
        );
        // A change written before the field existed carries none.
        assert_eq!(vec!["20:b:5", "20:b:4"], product_ids(Vec::new())?);
        // The fallback starts past the edit, whatever was consumed.
        assert_eq!(
            vec!["20:b:4", "20:b:9"],
            product_ids(vec![TimeTicket::new(20, 9, "b")])?
        );
        Ok(())
    }

    /// JS `tree_concurrency_test.ts` "intersect-element(merge,replaceText)":
    /// the replacement applies after a merge it did not know. Its range was
    /// measured on the tree before the merge, so a reverse built from it
    /// does not fit the tree after it, and JS's `maxNeededIdx` guard builds
    /// none rather than failing: the edit applies, with no reverse.
    #[test]
    fn applies_an_edit_whose_reverse_does_not_fit_the_tree() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let paragraph = |lamport: i64, value: &str| {
            TreeNode::create_element(
                TreeNodeId::new(ticket(lamport, "a"), 0),
                "p",
                None,
                vec![TreeNode::create_text(
                    TreeNodeId::new(ticket(lamport + 1, "a"), 0),
                    value,
                )],
            )
        };
        let mut root = seeded_root(
            tree_at.clone(),
            vec![
                paragraph(2, "abc"),
                paragraph(4, "def"),
                paragraph(6, "ghi"),
            ],
        );
        let (merge, replace) = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (
                (tree.find_pos(4, true)?, tree.find_pos(6, true)?),
                (tree.find_pos(5, true)?, tree.find_pos(15, true)?),
            )
        };

        let mut known = VersionVector::new();
        known.set("a", 9);
        known.set("b", 10);
        TreeEditOperation::create(
            tree_at.clone(),
            merge.0,
            merge.1,
            None,
            0,
            Some(ticket(10, "b")),
        )
        .execute(&mut root, OpSource::Remote, Some(&known))?;
        assert_eq!(
            "<root><p>abcdef</p><p>ghi</p></root>",
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );

        let mut known = VersionVector::new();
        known.set("a", 9);
        known.set("c", 10);
        let result = TreeEditOperation::create(
            tree_at.clone(),
            replace.0,
            replace.1,
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "c"), 0),
                "B",
            )]),
            0,
            Some(ticket(10, "c")),
        )
        .execute(&mut root, OpSource::Remote, Some(&known))?
        .unwrap();
        assert_eq!(None, result.reverse_op);
        assert_eq!(
            "<root><p>abc</p>B</root>",
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        Ok(())
    }

    fn run(
        root: &mut CrdtRoot,
        operation: &TreeEditOperation,
        source: OpSource,
    ) -> crate::Result<ExecutionResult> {
        Ok(operation.execute(root, source, None)?.unwrap())
    }

    fn tree_reverse(result: &ExecutionResult) -> TreeEditOperation {
        match &result.reverse_op {
            Some(Operation::TreeEdit(operation)) => operation.clone(),
            other => panic!("expected a tree edit reverse, got {other:?}"),
        }
    }

    /// Runs `reverse` as an undo or a redo runs it: under a fresh ticket,
    /// with copied content under fresh IDs, at its stored indexes resolved
    /// on the tree as it stands.
    fn run_reverse(
        root: &mut CrdtRoot,
        reverse: &TreeEditOperation,
        executed_at: TimeTicket,
    ) -> crate::Result<ExecutionResult> {
        let mut reverse = reverse.clone();
        let mut delimiter = executed_at.delimiter();
        reverse.reissue_content_ids(&mut || {
            delimiter += 1;
            TimeTicket::new(
                executed_at.lamport(),
                delimiter,
                executed_at.actor_id().clone(),
            )
        })?;
        reverse.set_executed_at(executed_at);
        reverse.resolve_undo_positions(root)?;
        run(root, &reverse, OpSource::UndoRedo)
    }

    fn xml(root: &CrdtRoot, tree_at: &TimeTicket) -> String {
        root.tree_by_created_at(tree_at).unwrap().to_xml()
    }

    fn span_ids(spans: &[crate::crdt::tree::TreeRestoreSpan]) -> Vec<TreeNodeId> {
        spans.iter().map(|span| span.id.clone()).collect()
    }

    fn node_id(lamport: i64) -> TreeNodeId {
        TreeNodeId::new(ticket(lamport, "a"), 0)
    }

    /// JS `TreeEditOperation.toReverseOperation`, the identity outcome: an
    /// insertion reverses by re-removing what it inserted, by identity, and
    /// the redo revives it; each identity run reverses into the other mode
    /// over the same spans and keeps the stored indexes.
    #[test]
    fn reverses_an_insertion_by_identity() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), Vec::new());
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(0, true)?;
        let insert = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            Some(vec![paragraph_node()]),
            0,
            Some(ticket(10, "a")),
        );

        let result = run(&mut root, &insert, OpSource::Local)?;
        assert_eq!(
            Some(TreeEditExecution {
                last_from_idx: Some(0),
                last_to_idx: Some(0),
                inserted_content_size: Some(7),
                ..TreeEditExecution::default()
            }),
            result.tree_edit
        );
        let undo = tree_reverse(&result);
        assert!(undo.is_undo_op());
        assert_eq!(Some((0, 0)), undo.undo_indexes());
        assert_eq!(Some(RestoreMode::Restore), undo.restore_mode());
        assert!(undo.restore_spans().is_empty());
        // Parents before children, which recreating a purged subtree needs.
        assert_eq!(
            vec![node_id(2), node_id(3)],
            span_ids(undo.retombstone_spans())
        );
        assert!(undo.contents().is_none());
        assert_eq!(0, undo.split_level());

        let undone = run_reverse(&mut root, &undo, ticket(11, "a"))?;
        assert_eq!("<root></root>", xml(&root, &tree_at));
        // One op info per node that left: the <p>; its text left with it.
        assert_eq!(
            vec![OpInfo::TreeEdit {
                path: "$.body".to_owned(),
                from: 0,
                to: 7,
                from_path: vec![0],
                to_path: vec![1],
                value: None,
                split_level: Some(0),
            }],
            undone.op_infos
        );
        assert_eq!(
            Some(vec![(0, 7, 0)]),
            undone.tree_edit.clone().unwrap().executed_ranges
        );
        let redo = tree_reverse(&undone);
        assert_eq!(Some(RestoreMode::Retombstone), redo.restore_mode());
        assert_eq!(undo.restore_spans(), redo.restore_spans());
        assert_eq!(undo.retombstone_spans(), redo.retombstone_spans());
        assert_eq!(Some((0, 0)), redo.undo_indexes());

        let redone = run_reverse(&mut root, &redo, ticket(12, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        assert_eq!(
            // Parent first, each measured right after it came back.
            Some(vec![(0, 0, 2), (1, 1, 5)]),
            redone.tree_edit.clone().unwrap().executed_ranges
        );
        assert_eq!(
            Some(RestoreMode::Restore),
            tree_reverse(&redone).restore_mode()
        );
        Ok(())
    }

    /// JS `TreeEditOperation.toReverseOperation`, the identity outcome for a
    /// deletion: the reverse revives the removed nodes by identity.
    #[test]
    fn reverses_a_deletion_by_identity() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.find_pos(0, true)?, tree.find_pos(7, true)?)
        };
        let delete = TreeEditOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            None,
            0,
            Some(ticket(10, "a")),
        );

        let result = run(&mut root, &delete, OpSource::Local)?;
        let facts = result.tree_edit.clone().unwrap();
        assert_eq!((Some(0), Some(7)), (facts.last_from_idx, facts.last_to_idx));
        let undo = tree_reverse(&result);
        assert_eq!(Some(RestoreMode::Restore), undo.restore_mode());
        assert_eq!(vec![node_id(2), node_id(3)], span_ids(undo.restore_spans()));
        assert!(undo.retombstone_spans().is_empty());
        assert_eq!(Some((0, 0)), undo.undo_indexes());

        run_reverse(&mut root, &undo, ticket(11, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        Ok(())
    }

    /// JS `TreeEditOperation.toReverseOperation`, the identity outcome for a
    /// replacement: the reverse carries both span lists.
    #[test]
    fn reverses_a_replacement_by_identity() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.find_pos(2, true)?, tree.find_pos(4, true)?)
        };
        let replace = TreeEditOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            Some(vec![TreeNode::create_text(node_id(10), "X")]),
            0,
            Some(ticket(10, "a")),
        );

        let result = run(&mut root, &replace, OpSource::Local)?;
        assert_eq!("<root><p>hXlo</p></root>", xml(&root, &tree_at));
        let undo = tree_reverse(&result);
        assert_eq!(Some(RestoreMode::Restore), undo.restore_mode());
        assert_eq!(1, undo.restore_spans().len());
        assert_eq!(vec![node_id(10)], span_ids(undo.retombstone_spans()));
        assert_eq!(Some((2, 2)), undo.undo_indexes());

        let undone = run_reverse(&mut root, &undo, ticket(11, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        run_reverse(&mut root, &tree_reverse(&undone), ticket(12, "a"))?;
        assert_eq!("<root><p>hXlo</p></root>", xml(&root, &tree_at));
        Ok(())
    }

    /// JS `TreeEditOperation.toReverseOperation`, the merge outcome: a merge
    /// reverses by a split at the index the range started at, as deep as
    /// the merge went, naming the elements it removed innermost first (JS
    /// `mergedAwayIDs`).
    #[test]
    fn reverses_a_merge_by_a_split_naming_the_merged_elements() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        for (from, to, level, replaced) in [
            (vec![0, 0, 1], vec![0, 1, 0], 1, vec![node_id(6)]),
            (
                vec![0, 0, 0, 4],
                vec![0, 1, 0, 0],
                2,
                vec![node_id(7), node_id(6)],
            ),
        ] {
            let mut root = seeded_nested_root(tree_at.clone());
            let range = {
                let tree = root.tree_by_created_at(&tree_at).unwrap();
                (tree.path_to_pos(&from)?, tree.path_to_pos(&to)?)
            };
            let merge = TreeEditOperation::create(
                tree_at.clone(),
                range.0,
                range.1,
                None,
                0,
                Some(ticket(20, "a")),
            );

            let result = run(&mut root, &merge, OpSource::Local)?;
            let undo = tree_reverse(&result);
            let from_idx = result.tree_edit.unwrap().last_from_idx.unwrap();
            assert_eq!(level, undo.split_level(), "merge {from:?}");
            assert_eq!(replaced, undo.replaced_ids(), "merge {from:?}");
            assert_eq!(Some((from_idx, from_idx)), undo.undo_indexes());
            assert!(undo.contents().is_none());
            assert_eq!(None, undo.restore_mode());

            run_reverse(&mut root, &undo, ticket(21, "a"))?;
            assert_eq!(
                "<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>",
                xml(&root, &tree_at),
                "merge {from:?}"
            );
        }
        Ok(())
    }

    /// JS `TreeEditOperation.toSplitReverseOperation`: a pure split reverses
    /// by deleting the boundaries it opened, tagged so its own reverse, the
    /// redo, splits again and names the element the deletion merged away.
    #[test]
    fn reverses_a_pure_split_by_a_boundary_deletion() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(3, true)?;
        let split = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            None,
            1,
            Some(ticket(10, "a")),
        );

        let result = run(&mut root, &split, OpSource::Local)?;
        assert_eq!("<root><p>he</p><p>llo</p></root>", xml(&root, &tree_at));
        assert_eq!(2, result.tree_edit.as_ref().unwrap().split_size);
        let undo = tree_reverse(&result);
        assert_eq!(0, undo.split_level());
        assert_eq!(Some((3, 5)), undo.undo_indexes());
        assert_eq!(Some(1), undo.redo_split_level());
        assert_eq!(None, undo.restore_mode());

        let undone = run_reverse(&mut root, &undo, ticket(11, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        // The boundary deletion merged the split's <p> away; spans would
        // describe it, but the tag keeps the redo a split.
        let redo = tree_reverse(&undone);
        assert_eq!(1, redo.split_level());
        assert_eq!(None, redo.restore_mode());
        assert_eq!(Some((3, 3)), redo.undo_indexes());
        assert_eq!(1, redo.replaced_ids().len());

        run_reverse(&mut root, &redo, ticket(12, "a"))?;
        assert_eq!("<root><p>he</p><p>llo</p></root>", xml(&root, &tree_at));
        Ok(())
    }

    /// JS `toSplitReverseOperation` and `toReverseOperation`'s
    /// `redoSplitLevel` branch: a split at the end of an element opens an
    /// empty one; the undo deletes the boundary between the two, and the
    /// redo splits there again.
    #[test]
    fn re_splits_on_the_redo_of_a_split_at_the_end_of_an_element() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(6, true)?;
        let split = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            None,
            1,
            Some(ticket(10, "a")),
        );

        let result = run(&mut root, &split, OpSource::Local)?;
        assert_eq!("<root><p>hello</p><p></p></root>", xml(&root, &tree_at));
        let undo = tree_reverse(&result);
        assert_eq!(Some((6, 8)), undo.undo_indexes());

        let undone = run_reverse(&mut root, &undo, ticket(11, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        let redo = tree_reverse(&undone);
        assert_eq!(None, redo.restore_mode());
        assert_eq!(1, redo.split_level());

        run_reverse(&mut root, &redo, ticket(12, "a"))?;
        assert_eq!("<root><p>hello</p><p></p></root>", xml(&root, &tree_at));
        Ok(())
    }

    /// JS `toSplitReverseOperation`: a split deeper than the tree opens only
    /// the levels it has, and the boundary deletion is sized by what it
    /// opened (`splitSize`), not by twice the level it asked for, which here
    /// would still fit the tree and delete past the boundary.
    #[test]
    fn sizes_the_split_reverse_by_what_the_split_opened() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(3, true)?;
        let split = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            None,
            3,
            Some(ticket(10, "a")),
        );

        let result = run(&mut root, &split, OpSource::Local)?;
        assert_eq!("<root><p>he</p><p>llo</p></root>", xml(&root, &tree_at));
        assert_eq!(9, root.tree_by_created_at(&tree_at).unwrap().root().len());
        let undo = tree_reverse(&result);
        assert_eq!(Some((3, 5)), undo.undo_indexes());
        assert_eq!(Some(3), undo.redo_split_level());

        run_reverse(&mut root, &undo, ticket(11, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        Ok(())
    }

    /// JS `toReverseOperation`'s `maxNeededIdx` guard: content inserted
    /// into a parent removed concurrently is born removed, so the inserted
    /// range runs past the tree and the edit, which had no visible effect,
    /// has no reverse.
    #[test]
    fn has_no_reverse_for_an_insertion_under_a_removed_parent() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let (insert, delete) = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (
                tree.find_pos(3, true)?,
                (tree.find_pos(0, true)?, tree.find_pos(7, true)?),
            )
        };
        let mut known = VersionVector::new();
        known.set("a", 9);
        TreeEditOperation::create(
            tree_at.clone(),
            delete.0,
            delete.1,
            None,
            0,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, Some(&known))?;

        let mut known = VersionVector::new();
        known.set("a", 9);
        known.set("b", 10);
        let result = TreeEditOperation::create(
            tree_at.clone(),
            insert.clone(),
            insert,
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "b"), 0),
                "X",
            )]),
            0,
            Some(ticket(10, "b")),
        )
        .execute(&mut root, OpSource::Local, Some(&known))?
        .unwrap();

        assert_eq!("<root></root>", xml(&root, &tree_at));
        assert_eq!(None, result.reverse_op);
        assert_eq!(
            Some(1),
            result.tree_edit.unwrap().inserted_content_size,
            "measured before the insert, as JS does"
        );
        Ok(())
    }

    /// JS `toReverseOperation`, the copy-reinsert outcome: an edit the spans
    /// cannot describe whole (here it splits a piece off a removed node)
    /// reverses by inserting a copy of what it removed, without the nodes
    /// removed before it (JS `cloneAndDropPreTombstoned`). As in JS,
    /// `last_to_idx` counts every node the range marked, the one removed
    /// before included.
    #[test]
    fn reverses_by_a_copy_without_the_nodes_removed_before() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(
            tree_at.clone(),
            vec![TreeNode::create_element(
                node_id(2),
                "p",
                None,
                vec![TreeNode::create_text(node_id(3), "abcd")],
            )],
        );
        let (first, second) = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (
                (tree.find_pos(2, true)?, tree.find_pos(4, true)?),
                (tree.find_pos(1, true)?, tree.find_pos(3, true)?),
            )
        };
        let mut known = VersionVector::new();
        known.set("a", 9);
        TreeEditOperation::create(
            tree_at.clone(),
            first.0,
            first.1,
            None,
            0,
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, Some(&known))?;
        assert_eq!("<root><p>ad</p></root>", xml(&root, &tree_at));

        // Concurrent with the first: it removes "a" and reaches "b", which
        // the first removed, and its end splits "c" off that tombstone.
        let mut known = VersionVector::new();
        known.set("a", 9);
        known.set("b", 10);
        let result = TreeEditOperation::create(
            tree_at.clone(),
            second.0,
            second.1,
            None,
            0,
            Some(ticket(11, "b")),
        )
        .execute(&mut root, OpSource::Local, Some(&known))?
        .unwrap();
        assert_eq!("<root><p>d</p></root>", xml(&root, &tree_at));

        let facts = result.tree_edit.clone().unwrap();
        assert_eq!((Some(1), Some(3)), (facts.last_from_idx, facts.last_to_idx));
        let undo = tree_reverse(&result);
        assert_eq!(None, undo.restore_mode());
        assert_eq!(Some((1, 1)), undo.undo_indexes());
        let contents = undo.contents().unwrap();
        assert_eq!(1, contents.len());
        assert_eq!("a", contents[0].value());
        assert!(!contents[0].is_removed());

        run_reverse(&mut root, &undo, ticket(12, "a"))?;
        assert_eq!("<root><p>ad</p></root>", xml(&root, &tree_at));
        Ok(())
    }

    /// Review Focus 9: building a reverse reports what it cannot build
    /// instead of swallowing it, so `execute` can fail a local edit rather
    /// than lose its undo entry without notice (it drops the error for a
    /// remote change only). Here the redo of a boundary deletion would
    /// re-split at an index the tree does not have.
    #[test]
    fn reports_a_reverse_it_cannot_build() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(3, true)?;
        let mut deletion = TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos.clone(),
            None,
            0,
            Some(ticket(10, "a")),
        );
        deletion.redo_split_level = Some(1);
        let result = crate::crdt::tree::TreeEditResult {
            from_idx: 100,
            ..Default::default()
        };

        let tree = root.tree_by_created_at(&tree_at).unwrap();
        assert!(deletion
            .reverse_of(tree, &(pos.clone(), pos), &result)
            .is_err());
        Ok(())
    }

    /// JS `undo_content_identity_test.ts` "gives copied content a fresh
    /// identity": every node of the copy, in post-order, takes a new ID, and
    /// loses the split chain and merge lineage of the node it copied.
    #[test]
    fn gives_copied_content_a_fresh_identity() -> crate::Result<()> {
        let pos = crate::crdt::tree::TreePos::new(node_id(1), node_id(1));
        let mut paragraph = TreeNode::create_element(
            node_id(2),
            "p",
            None,
            vec![
                TreeNode::create_text(node_id(3), "a"),
                TreeNode::create_text(node_id(4), "b"),
            ],
        );
        paragraph.set_ins_next_id(Some(node_id(9)));
        paragraph.set_merged_from(Some(node_id(8)));
        let mut op = TreeEditOperation::create(
            ticket(1, "a"),
            pos.clone(),
            pos,
            Some(vec![paragraph]),
            0,
            Some(ticket(5, "a")),
        )
        .into_undo(0, 0);

        let mut delimiter = 0;
        let mut issue = || {
            delimiter += 1;
            TimeTicket::new(20, delimiter, "a")
        };
        let pairs = op.reissue_content_ids(&mut issue)?;

        // The count is stated rather than derived from the same traversal.
        assert_eq!(3, pairs.len(), "the <p> and its two texts");
        assert_eq!(
            vec![node_id(3), node_id(4), node_id(2)],
            pairs.iter().map(|(old, _)| old.clone()).collect::<Vec<_>>()
        );
        let contents = op.contents().unwrap();
        let after = std::iter::once(contents[0].id().clone())
            .chain(contents[0].all_children().map(|node| node.id().clone()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(3, after.len(), "every node gets its own id");
        assert!(after.iter().all(|id| id.created_at().lamport() == 20));
        assert_eq!(None, contents[0].ins_next_id());
        assert_eq!(None, contents[0].merged_from());
        Ok(())
    }

    /// JS `undo_content_identity_test.ts` "leaves a restore-mode reverse
    /// alone": an identity reverse revives nodes by identity and keeps it.
    #[test]
    fn leaves_a_restore_mode_reverse_alone() -> crate::Result<()> {
        let pos = crate::crdt::tree::TreePos::new(node_id(1), node_id(1));
        let mut op = TreeEditOperation::create(
            ticket(1, "a"),
            pos.clone(),
            pos,
            Some(vec![TreeNode::create_text(node_id(3), "a")]),
            0,
            Some(ticket(5, "a")),
        )
        .into_undo(0, 0)
        .with_identity(Vec::new(), RestoreMode::Restore, Vec::new());

        let pairs = op.reissue_content_ids(&mut || ticket(20, "a"))?;

        assert!(pairs.is_empty());
        assert_eq!(&node_id(3), op.contents().unwrap()[0].id());
        Ok(())
    }

    /// JS `undo_content_identity_test.ts` "assigns ids that a later reissue
    /// does not repeat".
    #[test]
    fn assigns_ids_that_a_later_reissue_does_not_repeat() -> crate::Result<()> {
        let pos = crate::crdt::tree::TreePos::new(node_id(1), node_id(1));
        let op = |id| {
            TreeEditOperation::create(
                ticket(1, "a"),
                pos.clone(),
                pos.clone(),
                Some(vec![TreeNode::create_text(id, "a")]),
                0,
                Some(ticket(5, "a")),
            )
            .into_undo(0, 0)
        };
        let (mut first, mut second) = (op(node_id(3)), op(node_id(4)));

        let mut delimiter = 0;
        let mut issue = || {
            delimiter += 1;
            TimeTicket::new(20, delimiter, "a")
        };
        first.reissue_content_ids(&mut issue)?;
        second.reissue_content_ids(&mut issue)?;

        assert_ne!(
            first.contents().unwrap()[0].id(),
            second.contents().unwrap()[0].id()
        );
        Ok(())
    }

    /// JS `reissueContentIDs`: a splitting edit with content is refused, as
    /// its rebuilt split tickets would overlap the reissued ones.
    #[test]
    fn refuses_to_reissue_the_content_of_a_splitting_edit() {
        let pos = crate::crdt::tree::TreePos::new(node_id(1), node_id(1));
        let mut op = TreeEditOperation::create(
            ticket(1, "a"),
            pos.clone(),
            pos,
            Some(vec![TreeNode::create_text(node_id(3), "a")]),
            1,
            Some(ticket(5, "a")),
        );

        let error = op.reissue_content_ids(&mut || ticket(20, "a")).unwrap_err();
        assert_eq!(crate::ErrorCode::ErrRefused, error.code());
    }

    /// A row of the reconcile table: its name, the undo edit, the remote
    /// `(from, to, content size)`, and the stored range expected after.
    type ReconcileRow = (
        &'static str,
        TreeEditOperation,
        (usize, usize, usize),
        Option<(usize, usize)>,
    );

    /// Go `tree_edit_reconcile_test.go` `TestTreeEditReconcileOperationCases`
    /// (JS `TreeEditOperation.reconcileOperation`): the six overlap cases,
    /// the two adjacent edges, and the guards.
    #[test]
    fn reconciles_undo_indexes_against_a_remote_edit() {
        let pos = crate::crdt::tree::TreePos::new(node_id(1), node_id(1));
        let undo = |from, to| {
            TreeEditOperation::create(ticket(1, "a"), pos.clone(), pos.clone(), None, 0, None)
                .into_undo(from, to)
        };
        let rows: [ReconcileRow; 12] = [
            (
                "guard: not an undo op",
                {
                    let mut op = undo(10, 20);
                    op.is_undo_op = false;
                    op
                },
                (0, 100, 5),
                Some((10, 20)),
            ),
            (
                "guard: an identity op",
                undo(10, 20).with_identity(Vec::new(), RestoreMode::Restore, Vec::new()),
                (0, 100, 5),
                Some((10, 20)),
            ),
            (
                "guard: an inverted remote range",
                undo(10, 20),
                (15, 5, 3),
                Some((10, 20)),
            ),
            (
                "guard: no stored range",
                {
                    let mut op = undo(10, 20);
                    op.from_idx = None;
                    op.to_idx = None;
                    op
                },
                (0, 100, 5),
                None,
            ),
            ("case 1: left", undo(10, 20), (2, 6, 1), Some((7, 17))),
            ("case 2: right", undo(10, 20), (25, 30, 2), Some((10, 20))),
            (
                "case 3: inside the remote",
                undo(10, 20),
                (5, 25, 3),
                Some((5, 5)),
            ),
            (
                "case 4: remote inside",
                undo(10, 20),
                (12, 15, 1),
                Some((10, 18)),
            ),
            (
                "case 5: overlaps the start",
                undo(10, 20),
                (5, 15, 9),
                Some((5, 10)),
            ),
            (
                "case 6: overlaps the end",
                undo(10, 20),
                (15, 25, 4),
                Some((10, 15)),
            ),
            (
                "case 7: at the start edge",
                undo(10, 20),
                (10, 10, 2),
                Some((12, 22)),
            ),
            (
                "case 7: at the end edge",
                undo(10, 20),
                (20, 20, 2),
                Some((10, 20)),
            ),
        ];
        for (name, mut op, (from, to, len), expected) in rows {
            op.reconcile_operation(from, to, len);
            assert_eq!(expected, op.from_idx.zip(op.to_idx), "{name}");
        }
    }

    /// Go `tree_edit_reconcile_test.go` "never touches restoreSpans or
    /// retombstoneSpans", and JS `reconcileNodeID`: index reconciliation
    /// leaves the spans alone, while re-pointing a node rewrites every ID
    /// they carry, the range and the re-created elements.
    #[test]
    fn re_points_spans_but_never_reconciles_them_by_index() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.find_pos(0, true)?, tree.find_pos(7, true)?)
        };
        let result = run(
            &mut root,
            &TreeEditOperation::create(
                tree_at.clone(),
                range.0,
                range.1,
                None,
                0,
                Some(ticket(10, "a")),
            ),
            OpSource::Local,
        )?;
        let mut undo = tree_reverse(&result);
        let spans = undo.restore_spans().to_vec();
        undo.reconcile_operation(0, 5, 1);
        assert_eq!(spans, undo.restore_spans());
        assert_eq!(Some((0, 0)), undo.undo_indexes());

        undo.replaced_ids = vec![node_id(2)];
        undo.reconcile_node_id(&node_id(2), &node_id(30));
        assert_eq!(
            vec![node_id(30), node_id(3)],
            span_ids(undo.restore_spans())
        );
        assert_eq!(
            Some(&node_id(30)),
            undo.restore_spans()[1].parent_id.as_ref()
        );
        assert_eq!(vec![node_id(30)], undo.replaced_ids());
        Ok(())
    }

    /// The local undo state and the execution facts never reach the wire,
    /// so they take no part in equality; the spans and their mode do.
    #[test]
    fn compares_only_the_fields_the_wire_carries() {
        let pos = crate::crdt::tree::TreePos::new(node_id(1), node_id(1));
        let plain = TreeEditOperation::create(
            ticket(1, "a"),
            pos.clone(),
            pos,
            None,
            0,
            Some(ticket(5, "a")),
        );
        let mut local = plain.clone().into_undo(3, 4);
        local.redo_split_level = Some(1);
        local.replaced_ids = vec![node_id(2)];
        local.set_execution(TreeEditExecution {
            last_from_idx: Some(1),
            ..TreeEditExecution::default()
        });
        assert_eq!(plain, local);
        // JS writes a restore mode only with spans.
        assert_eq!(
            plain,
            plain
                .clone()
                .with_identity(Vec::new(), RestoreMode::Restore, Vec::new())
        );
        let span = crate::crdt::tree::TreeRestoreSpan {
            id: node_id(2),
            node_type: "p".to_owned(),
            is_text: false,
            length: 0,
            value: None,
            attrs: None,
            parent_id: None,
            left_sibling_id: None,
            right_sibling_id: None,
        };
        assert_ne!(
            plain,
            plain
                .clone()
                .with_identity(vec![span], RestoreMode::Restore, Vec::new())
        );
    }

    /// Go `tree_restore_test.go` `TestTreeRestoreDocSizeSymmetry`, through
    /// the operation: an edit, its undo, the redo and the undo again move
    /// the sizes back and forth without drift, and the GC size is always
    /// what a rebuild of the same document books.
    ///
    /// Live is not compared with a rebuild: a Tree removal here leaves the
    /// removed size in live (see `CrdtRoot::unregister_gc_pair_by_id` and
    /// the porting gaps), so live runs ahead of a rebuild by what is
    /// removed from the moment of the edit, before any undo. The identity
    /// runs neither widen nor narrow that gap.
    #[test]
    fn keeps_the_document_size_through_undo_and_redo() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone(), vec![paragraph_node()]);
        let baseline = root.doc_size();
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.find_pos(2, true)?, tree.find_pos(4, true)?)
        };
        let replace = TreeEditOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            Some(vec![TreeNode::create_text(node_id(10), "XY")]),
            0,
            Some(ticket(10, "a")),
        );
        let gc_matches_a_rebuild = |root: &CrdtRoot| {
            assert_eq!(root.deepcopy().doc_size().gc, root.doc_size().gc);
        };

        let result = run(&mut root, &replace, OpSource::Local)?;
        let after_edit = root.doc_size();
        assert_ne!(baseline, after_edit);
        gc_matches_a_rebuild(&root);
        // The gap, there before any undo: "el" stays in live.
        assert_eq!(
            root.deepcopy().doc_size().live.data + 4,
            after_edit.live.data
        );

        let undone = run_reverse(&mut root, &tree_reverse(&result), ticket(11, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        gc_matches_a_rebuild(&root);
        let after_undo = root.doc_size();

        let redone = run_reverse(&mut root, &tree_reverse(&undone), ticket(12, "a"))?;
        assert_eq!("<root><p>hXYlo</p></root>", xml(&root, &tree_at));
        gc_matches_a_rebuild(&root);
        assert_eq!(after_edit, root.doc_size(), "the redo is the edit again");

        run_reverse(&mut root, &tree_reverse(&redone), ticket(13, "a"))?;
        assert_eq!("<root><p>hello</p></root>", xml(&root, &tree_at));
        gc_matches_a_rebuild(&root);
        assert_eq!(after_undo, root.doc_size(), "the second undo is the first");
        Ok(())
    }

    fn seeded_root(tree_at: TimeTicket, children: Vec<TreeNode>) -> CrdtRoot {
        let tree = CrdtTree::create(
            TreeNode::create_element(TreeNodeId::new(tree_at.clone(), 0), "root", None, children),
            tree_at,
        );

        CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("body", CrdtElement::tree(tree))],
        ))
    }

    fn seeded_nested_root(tree_at: TimeTicket) -> CrdtRoot {
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(tree_at.clone(), 0),
                "doc",
                None,
                vec![TreeNode::create_element(
                    TreeNodeId::new(ticket(2, "a"), 0),
                    "tc",
                    None,
                    vec![
                        TreeNode::create_element(
                            TreeNodeId::new(ticket(3, "a"), 0),
                            "p",
                            None,
                            vec![TreeNode::create_element(
                                TreeNodeId::new(ticket(4, "a"), 0),
                                "tn",
                                None,
                                vec![TreeNode::create_text(
                                    TreeNodeId::new(ticket(5, "a"), 0),
                                    "1234",
                                )],
                            )],
                        ),
                        TreeNode::create_element(
                            TreeNodeId::new(ticket(6, "a"), 0),
                            "p",
                            None,
                            vec![TreeNode::create_element(
                                TreeNodeId::new(ticket(7, "a"), 0),
                                "tn",
                                None,
                                vec![TreeNode::create_text(
                                    TreeNodeId::new(ticket(8, "a"), 0),
                                    "5678",
                                )],
                            )],
                        ),
                    ],
                )],
            ),
            tree_at,
        );

        CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("body", CrdtElement::tree(tree))],
        ))
    }

    fn execute_tree_edit_reverse(
        root: &mut CrdtRoot,
        reverse_op: Option<Operation>,
        executed_at: TimeTicket,
    ) -> crate::Result<()> {
        let Some(mut reverse_op) = reverse_op else {
            panic!("expected reverse tree edit operation");
        };
        let Operation::TreeEdit(operation) = &mut reverse_op else {
            panic!("unexpected reverse operation: {reverse_op:?}");
        };
        operation.set_executed_at(executed_at);
        reverse_op.execute(root, OpSource::Remote)?;
        Ok(())
    }

    /// JS `surrogate_index_test.ts` "builds reverse operations at indexes
    /// inside a pair": the reverse is built from the document's own indexes,
    /// for remote changes too, so it takes the unchecked lookup. Each builder
    /// is called directly with an index inside the pair: the copy reverse of
    /// an insertion ends there, the split reversing a merge and the boundary
    /// deletion reversing a split start there.
    #[test]
    fn builds_reverse_operations_at_indexes_inside_a_pair() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(tree_at.clone(), 0),
                "r",
                None,
                vec![TreeNode::create_element(
                    TreeNodeId::new(ticket(2, "a"), 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(
                        TreeNodeId::new(ticket(3, "a"), 0),
                        "😀",
                    )],
                )],
            ),
            tree_at.clone(),
        );
        assert!(tree.find_pos(2, true).is_err());

        let pos = tree.find_pos(1, true)?;
        let range = (pos.clone(), pos.clone());
        let op = |split_level| {
            TreeEditOperation::create(
                tree_at.clone(),
                pos.clone(),
                pos.clone(),
                None,
                split_level,
                Some(ticket(10, "a")),
            )
        };
        let (split, edit) = (op(1), op(0));
        let result = |from_idx, merge_level, inserted_size| crate::crdt::tree::TreeEditResult {
            merge_level,
            from_idx,
            to_idx: from_idx,
            inserted_size,
            ..Default::default()
        };

        // The copy reverse of an insertion ends inside the pair.
        assert!(edit
            .to_reverse_operation(&tree, &range, &result(1, 0, 1))?
            .is_some());
        // A merge reverse (a split) starts inside it.
        assert!(edit
            .to_reverse_operation(&tree, &range, &result(2, 1, 0))?
            .is_some());
        // A split reverse starts inside it, and one ends inside it.
        assert!(split.to_split_reverse_operation(&tree, 2, 2)?.is_some());
        assert!(split.to_split_reverse_operation(&tree, 0, 2)?.is_some());
        Ok(())
    }

    fn paragraph_node() -> TreeNode {
        TreeNode::create_element(
            TreeNodeId::new(ticket(2, "a"), 0),
            "p",
            None,
            vec![TreeNode::create_text(
                TreeNodeId::new(ticket(3, "a"), 0),
                "hello",
            )],
        )
    }

    fn ticket(lamport: i64, actor: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor)
    }
}
