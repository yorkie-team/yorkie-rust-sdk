use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::element::DataSize;
use crate::crdt::rga_tree_split::{
    RestoreSpan, RgaTreeSplitNode, RgaTreeSplitNodeId, RgaTreeSplitPos, ValueChange,
};
use crate::crdt::root::CrdtRoot;
use crate::crdt::text::TextValue;
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};
use std::collections::BTreeMap;

/// The direction of an identity edit, the undo and redo of a Text edit by
/// the identities of the characters it touched (JS `RestoreMode`).
/// `Restore` revives `restore_spans` and re-removes `retombstone_spans`;
/// `Retombstone`, the redo, does the opposite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreMode {
    Restore,
    Retombstone,
}

impl RestoreMode {
    pub(crate) fn flipped(self) -> Self {
        match self {
            Self::Restore => Self::Retombstone,
            Self::Retombstone => Self::Restore,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditOperation {
    meta: OperationMeta,
    from_pos: RgaTreeSplitPos,
    to_pos: RgaTreeSplitPos,
    content: String,
    attributes: BTreeMap<String, String>,
    is_undo_op: bool,
    /// The content the reversed edit removed, by identity (JS
    /// `restoreSpans`). JS tells an absent list from an empty one nowhere
    /// it reads them, so an empty `Vec` stands for both.
    restore_spans: Vec<RestoreSpan<TextValue>>,
    /// Set on an identity edit, which then runs by `restore_spans` and
    /// `retombstone_spans` alone; its positions only anchor a recreated
    /// piece when every piece of its insertion is gone (JS `restoreMode`).
    restore_mode: Option<RestoreMode>,
    /// The content the reversed edit inserted, by identity (JS
    /// `retombstoneSpans`).
    retombstone_spans: Vec<RestoreSpan<TextValue>>,
}

impl EditOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        from_pos: RgaTreeSplitPos,
        to_pos: RgaTreeSplitPos,
        content: impl Into<String>,
        attributes: BTreeMap<String, String>,
        executed_at: Option<TimeTicket>,
        is_undo_op: bool,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            from_pos,
            to_pos,
            content: content.into(),
            attributes,
            is_undo_op,
            restore_spans: Vec::new(),
            restore_mode: None,
            retombstone_spans: Vec::new(),
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        from_pos: RgaTreeSplitPos,
        to_pos: RgaTreeSplitPos,
        content: impl Into<String>,
        attributes: BTreeMap<String, String>,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(
            parent_created_at,
            from_pos,
            to_pos,
            content,
            attributes,
            executed_at,
            false,
        )
    }

    /// Makes this an identity edit in `restore_mode` over the given spans
    /// (the last three arguments of JS `EditOperation.create`).
    pub(crate) fn with_identity(
        mut self,
        restore_spans: Vec<RestoreSpan<TextValue>>,
        restore_mode: RestoreMode,
        retombstone_spans: Vec<RestoreSpan<TextValue>>,
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
        if root.text_by_created_at(self.parent_created_at()).is_none() {
            return Err(self.text_parent_error(root));
        }

        // As in JS, the mode picks the identity path, not the spans being
        // there: a reverse whose spans both came out empty still runs as a
        // no-op here instead of as an edit of its positions.
        if let Some(mode) = self.restore_mode {
            return self
                .execute_identity(root, source, version_vector, mode, path)
                .map(Some);
        }

        // An undo edit that inserts nothing changes nothing wherever it
        // points, but its positions can name an offset the text no longer
        // has: stacked entries are reconciled against remote positional
        // edits, not against remote undos and redos. JS runs it as a no-op
        // moved to the head; `refine_undo_positions` made the move, since
        // `execute` cannot change the operation here.
        if self.is_undo_op && self.content.is_empty() {
            return Ok(Some(ExecutionResult {
                op_infos: Vec::new(),
                reverse_op: (source != OpSource::Remote)
                    .then(|| Operation::Edit(self.empty_reverse())),
                tree_edit: None,
            }));
        }

        let executed_at = self.executed_at()?.clone();
        let (range, normalized_from) = {
            let text = root
                .text_by_created_at(self.parent_created_at())
                .ok_or_else(|| self.text_parent_error(root))?;
            let from_pos = if self.is_undo_op {
                text.refine_pos(&self.from_pos)?
            } else {
                self.from_pos.clone()
            };
            let to_pos = if self.is_undo_op {
                text.refine_pos(&self.to_pos)?
            } else {
                self.to_pos.clone()
            };
            let range = (from_pos, to_pos);
            let normalized_from = text.normalize_pos(&range.0)?;

            (range, normalized_from)
        };

        let attributes = (!self.attributes.is_empty()).then_some(self.attributes.clone());
        let result = {
            let Some(text) = root.text_by_created_at_mut(self.parent_created_at()) else {
                return Err(self.text_parent_error(root));
            };
            text.edit_by_range(
                range,
                self.content.clone(),
                attributes,
                executed_at.clone(),
                version_vector,
            )?
        };

        root.acc(result.diff);
        register_removed_text_nodes(root, result.removed_nodes);
        for (child_id, child_size, removed_at) in result.split_gc_pairs {
            root.register_gc_pair_by_id(child_id, child_size, removed_at);
        }

        // As in JS, one op info per range the edit changed: a removal reports
        // empty content, an insertion its value and attributes.
        let op_infos = result
            .changes
            .into_iter()
            .map(|change| edit_op_info(&path, change))
            .collect();

        Ok(Some(ExecutionResult {
            op_infos,
            reverse_op: Some(self.to_reverse_operation(
                result.removed_values,
                normalized_from,
                result.removed_spans,
                executed_at,
            )),
            tree_edit: None,
        }))
    }

    /// Runs an identity edit (JS `execute`, the `restoreMode` branch): the
    /// spans to re-remove go first, then the spans to revive, each by the
    /// identities the reversed edit recorded, never as fresh copies, so the
    /// relative order of the pieces holds across chained undos.
    fn execute_identity(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
        version_vector: Option<&VersionVector>,
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
        let mut op_infos = Vec::new();
        let mut total_diff = DataSize::default();

        // The spans about to be re-removed are what the reverse will have to
        // restore, and a replica that purged them rebuilds the pieces from
        // that payload alone. They were captured when the reversed edit ran,
        // so re-read them against the live pieces first: an attribute applied
        // to the run since would otherwise be missing from it. Only when the
        // reverse is kept: a remote change discards it.
        let refreshed = (source != OpSource::Remote && !to_retombstone.is_empty())
            .then(|| {
                self.text(root)
                    .map(|text| text.refresh_spans(to_retombstone))
            })
            .transpose()?;

        // 1. Re-remove what the reversed edit inserted. The reverse names
        // only the pieces the removal was recorded on: naming a piece someone
        // else removed would cancel a removal that piece never had.
        let mut recorded = None;
        if !to_retombstone.is_empty() {
            let result = self.text_mut(root)?.retombstone(
                refreshed.as_deref().unwrap_or(to_retombstone),
                &executed_at,
                version_vector,
            )?;
            add_data_size(&mut total_diff, result.diff);
            for (child_id, child_size, removed_at) in result.gc_pairs {
                root.register_gc_pair_by_id(child_id, child_size, removed_at);
            }
            op_infos.extend(
                result
                    .changes
                    .into_iter()
                    .map(|change| edit_op_info(&path, change)),
            );
            recorded = Some(result.recorded);
        }

        // 2. Revive what the reversed edit removed.
        if !to_restore.is_empty() {
            let (result, holds_cancels) = {
                let text = self.text_mut(root)?;
                let result = text.restore(to_restore, &executed_at, Some(&self.from_pos))?;
                (result, text.has_cancelled_removals())
            };
            if holds_cancels {
                root.register_ledger_holder(self.parent_created_at());
            }
            // Register first: a pending piece that ended up revived was born
            // by splitting a larger tombstone and never registered under its
            // own ID, so unregistering it below finds it only once it is.
            for (child_id, child_size, removed_at) in result.pending_gc_pairs {
                root.register_gc_pair_by_id(child_id, child_size, removed_at);
            }
            for node in &result.revived {
                root.unregister_gc_pair_by_id(&node.id_string(), node.data_size());
            }
            add_data_size(&mut total_diff, result.live_diff);
            op_infos.extend(
                result
                    .changes
                    .into_iter()
                    .map(|change| edit_op_info(&path, change)),
            );
        }

        root.acc(total_diff);

        // The reverse keeps the spans and flips the direction. The spans it
        // re-removes are the pieces this run recorded the removal on, read
        // against the live attributes.
        let (restore_spans, retombstone_spans) = match (is_retombstone, recorded) {
            (true, Some(recorded)) => (recorded, self.retombstone_spans.clone()),
            (false, Some(recorded)) => (self.restore_spans.clone(), recorded),
            (_, None) => (self.restore_spans.clone(), self.retombstone_spans.clone()),
        };
        Ok(ExecutionResult {
            op_infos,
            reverse_op: Some(Operation::Edit(self.identity_reverse(
                restore_spans,
                mode.flipped(),
                retombstone_spans,
            ))),
            tree_edit: None,
        })
    }

    pub(crate) fn parent_created_at(&self) -> &TimeTicket {
        self.meta.parent_created_at()
    }

    pub(crate) fn executed_at(&self) -> Result<&TimeTicket> {
        self.meta.executed_at()
    }

    pub(crate) fn range_from(&self) -> &RgaTreeSplitPos {
        &self.from_pos
    }

    pub(crate) fn to_pos(&self) -> &RgaTreeSplitPos {
        &self.to_pos
    }

    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    pub(crate) fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    /// Returns the content the reversed edit removed, by identity (JS
    /// `getRestoreSpans`).
    pub(crate) fn restore_spans(&self) -> &[RestoreSpan<TextValue>] {
        &self.restore_spans
    }

    /// Returns the direction of an identity edit (JS `getRestoreMode`).
    pub(crate) fn restore_mode(&self) -> Option<RestoreMode> {
        self.restore_mode
    }

    /// Returns the content the reversed edit inserted, by identity (JS
    /// `getRetombstoneSpans`).
    pub(crate) fn retombstone_spans(&self) -> &[RestoreSpan<TextValue>] {
        &self.retombstone_spans
    }

    pub(crate) fn set_executed_at(&mut self, executed_at: TimeTicket) {
        self.meta.set_executed_at(executed_at);
    }

    /// Remaps an undo edit's positions onto the text's current split chain.
    ///
    /// JS does this inside `execute` and keeps the result on the operation, so
    /// the change that carries an undo sends the refined positions to other
    /// replicas. `execute` takes `&self` here, so `Change` calls this first.
    ///
    /// As in JS, an identity edit is left alone: its positions only anchor a
    /// recreated piece. An undo edit that inserts nothing moves to the head
    /// instead of refining, before any refine can fail on a stale offset, so
    /// it runs as a no-op and its copy resolves on every replica.
    pub(crate) fn refine_undo_positions(&mut self, root: &CrdtRoot) -> Result<()> {
        if !self.is_undo_op || self.restore_mode.is_some() {
            return Ok(());
        }
        let text = self.text(root)?;
        if self.content.is_empty() {
            self.from_pos = head_pos();
            self.to_pos = head_pos();
            return Ok(());
        }
        self.from_pos = text.refine_pos(&self.from_pos)?;
        self.to_pos = text.refine_pos(&self.to_pos)?;
        Ok(())
    }

    /// Returns the UTF-16 indexes of this edit's range on the text as it is
    /// now (JS `normalizePos`).
    pub(crate) fn normalize_pos(&self, root: &CrdtRoot) -> Result<(usize, usize)> {
        let text = self.text(root)?;
        Ok((
            text.normalize_pos(&self.from_pos)?.relative_offset(),
            text.normalize_pos(&self.to_pos)?.relative_offset(),
        ))
    }

    /// Returns the length of the inserted content in UTF-16 code units, the
    /// unit of text indexes (JS `content.length`).
    pub(crate) fn content_len(&self) -> usize {
        self.content.encode_utf16().count()
    }

    /// Shifts an undo edit's range past a remote edit that replaced
    /// `remote_from..remote_to` with `content_len` code units (JS
    /// `reconcileOperation`). Only the relative offsets change: an undo edit's
    /// positions are normalized to the head of the text.
    ///
    /// An identity edit is shifted too: its spans locate what it changes,
    /// but `from_pos` is the anchor a recreated piece falls back to when
    /// every piece of its insertion is gone, so it has to follow remote
    /// edits like any undo position. The spans are never touched.
    pub(crate) fn reconcile_operation(
        &mut self,
        remote_from: usize,
        remote_to: usize,
        content_len: usize,
    ) {
        if !self.is_undo_op || remote_from > remote_to {
            return;
        }

        let remote_len = remote_to - remote_from;
        let local_from = self.from_pos.relative_offset();
        let local_to = self.to_pos.relative_offset();
        // JS clamps a negative result to zero.
        let shift = |offset: usize| (offset + content_len).saturating_sub(remote_len);

        let (from, to) = if remote_to <= local_from {
            // The remote edit is left of the range.
            (shift(local_from), shift(local_to))
        } else if local_to <= remote_from {
            // The remote edit is right of the range.
            return;
        } else if remote_from <= local_from && local_to <= remote_to && remote_from != remote_to {
            // The range lies inside the remote edit.
            (remote_from, remote_from)
        } else if local_from <= remote_from && remote_to <= local_to && local_from != local_to {
            // The remote edit lies inside the range.
            (local_from, shift(local_to))
        } else if remote_from < local_from && local_from < remote_to && remote_to < local_to {
            // The remote edit overlaps the start of the range.
            (remote_from, remote_from + (local_to - remote_to))
        } else if local_from < remote_from && remote_from < local_to && local_to < remote_to {
            // The remote edit overlaps the end of the range.
            (local_from, remote_from)
        } else {
            return;
        };

        self.from_pos = RgaTreeSplitPos::new(self.from_pos.id().clone(), from);
        self.to_pos = RgaTreeSplitPos::new(self.to_pos.id().clone(), to);
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        self.meta.set_actor(actor_id);
    }

    /// The JS `toTestString` form, which leaves the spans out.
    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.EDIT({},{},{})",
            self.parent_created_at().to_test_string(),
            self.from_pos.to_test_string(),
            self.to_pos.to_test_string(),
            self.content
        )
    }

    /// Builds the reverse of a positional edit (JS `toReverseOperation`).
    ///
    /// An edit that removed or inserted anything reverses by identity: the
    /// reverse revives what it removed and re-removes what it inserted, both
    /// under their original IDs. A copy inserted again would sort ahead of a
    /// neighbour not yet revived and break the order across chained undos,
    /// and a positional delete of the inserted range lands on the wrong
    /// node under a concurrent remote edit. Only an edit that changed
    /// nothing keeps the positional reverse.
    fn to_reverse_operation(
        &self,
        removed_values: Vec<TextValue>,
        from_pos: RgaTreeSplitPos,
        removed_spans: Vec<RestoreSpan<TextValue>>,
        executed_at: TimeTicket,
    ) -> Operation {
        if !removed_spans.is_empty() || !self.content.is_empty() {
            let mut inserted_spans = Vec::new();
            if !self.content.is_empty() {
                // The attributes the edit applied, stamped with the ticket the
                // insertion used. The undo only re-removes this span, but the
                // redo restores it, and a replica that purged the piece
                // rebuilds it from this value alone.
                let mut value = TextValue::create(self.content.clone());
                for (key, attr_value) in &self.attributes {
                    value.set_attr(key.clone(), attr_value.clone(), executed_at.clone());
                }
                inserted_spans.push(RestoreSpan {
                    created_at: executed_at,
                    start: 0,
                    end: self.content_len(),
                    value,
                });
            }
            return Operation::Edit(
                Self::new(
                    self.parent_created_at().clone(),
                    from_pos.clone(),
                    from_pos,
                    "",
                    BTreeMap::new(),
                    None,
                    true,
                )
                .with_identity(removed_spans, RestoreMode::Restore, inserted_spans),
            );
        }

        let content = removed_values
            .iter()
            .map(TextValue::content)
            .collect::<Vec<_>>()
            .join("");
        let attributes = if removed_values.len() == 1 {
            removed_values[0].get_attributes()
        } else {
            BTreeMap::new()
        };
        let to_pos = RgaTreeSplitPos::new(
            from_pos.id().clone(),
            from_pos.relative_offset() + self.content_len(),
        );

        Operation::Edit(Self::new(
            self.parent_created_at().clone(),
            from_pos,
            to_pos,
            content,
            attributes,
            None,
            true,
        ))
    }

    /// Builds the reverse of an identity edit, or, when both span lists
    /// came out empty, an empty undo edit at the head (JS
    /// `identityReverse`).
    ///
    /// A reverse with nothing to revive or re-remove still has to exist, or
    /// the undo stack would end one entry short of the redo stack. It cannot
    /// stay an identity edit either: the wire carries a restore mode only
    /// with spans, so it would go out as a positional edit of its stale
    /// range.
    fn identity_reverse(
        &self,
        restore_spans: Vec<RestoreSpan<TextValue>>,
        restore_mode: RestoreMode,
        retombstone_spans: Vec<RestoreSpan<TextValue>>,
    ) -> Self {
        if restore_spans.is_empty() && retombstone_spans.is_empty() {
            return self.empty_reverse();
        }
        Self::new(
            self.parent_created_at().clone(),
            self.from_pos.clone(),
            self.to_pos.clone(),
            "",
            BTreeMap::new(),
            None,
            true,
        )
        .with_identity(restore_spans, restore_mode, retombstone_spans)
    }

    /// Returns an empty undo edit at the head of this edit's text, the
    /// reverse of an edit that changes nothing (JS `emptyReverse`).
    fn empty_reverse(&self) -> Self {
        Self::new(
            self.parent_created_at().clone(),
            head_pos(),
            head_pos(),
            "",
            BTreeMap::new(),
            None,
            true,
        )
    }

    fn text<'a>(&self, root: &'a CrdtRoot) -> Result<&'a crate::crdt::text::CrdtText> {
        root.text_by_created_at(self.parent_created_at())
            .ok_or_else(|| self.text_parent_error(root))
    }

    fn text_mut<'a>(&self, root: &'a mut CrdtRoot) -> Result<&'a mut crate::crdt::text::CrdtText> {
        if root.text_by_created_at(self.parent_created_at()).is_none() {
            return Err(self.text_parent_error(root));
        }
        root.text_by_created_at_mut(self.parent_created_at())
            .ok_or_else(|| YorkieError::MissingCrdtElement(self.parent_created_at().to_id_string()))
    }

    fn text_parent_error(&self, root: &CrdtRoot) -> YorkieError {
        if root.find_by_created_at(self.parent_created_at()).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: self.parent_created_at().to_id_string(),
                expected: "text",
            };
        }

        YorkieError::MissingCrdtElement(self.parent_created_at().to_id_string())
    }
}

/// The position at the start of the head node, the one node of a text GC
/// never purges (JS `headPos`).
fn head_pos() -> RgaTreeSplitPos {
    RgaTreeSplitPos::new(RgaTreeSplitNodeId::new(TimeTicket::initial(), 0), 0)
}

/// As in JS, one op info per range an edit changed: a removal reports empty
/// content, an insertion its value and attributes.
fn edit_op_info(path: &str, change: ValueChange<TextValue>) -> OpInfo {
    let (content, attributes) = change
        .value
        .map(|value| (value.content().to_owned(), value.get_attributes()))
        .unwrap_or_default();
    OpInfo::Edit {
        path: path.to_owned(),
        from: change.from,
        to: change.to,
        content,
        attributes,
    }
}

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

fn register_removed_text_nodes(root: &mut CrdtRoot, nodes: Vec<RgaTreeSplitNode<TextValue>>) {
    for node in nodes {
        if let Some(removed_at) = node.removed_at() {
            root.register_gc_pair_by_id(node.id_string(), node.data_size(), removed_at.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{head_pos, EditOperation, RestoreMode};
    use crate::crdt::element::CrdtElement;
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::text::CrdtText;
    use crate::operation::{OpInfo, OpSource, Operation, SetOperation};
    use crate::{TimeTicket, VersionVector};
    use std::collections::BTreeMap;

    #[test]
    fn edits_text_element() -> crate::Result<()> {
        let mut root = root_with_text()?;
        let text_at = ticket(1);
        let range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 0)?;

        let result = EditOperation::create(
            text_at.clone(),
            range.0,
            range.1,
            "Hello World",
            BTreeMap::new(),
            Some(ticket(2)),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(r#"{"text":[{"val":"Hello World"}]}"#, root.to_json());
        assert_eq!(
            vec![OpInfo::Edit {
                path: "$.text".to_owned(),
                from: 0,
                to: 0,
                content: "Hello World".to_owned(),
                attributes: BTreeMap::new(),
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::Edit(_))));
        Ok(())
    }

    #[test]
    fn registers_removed_text_nodes_as_gc_pairs() -> crate::Result<()> {
        let mut root = root_with_text()?;
        let text_at = ticket(1);
        let insert_range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 0)?;
        EditOperation::create(
            text_at.clone(),
            insert_range.0,
            insert_range.1,
            "Hello World",
            BTreeMap::new(),
            Some(ticket(2)),
        )
        .execute(&mut root, OpSource::Remote, None)?;

        let replace_range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(6, 11)?;
        EditOperation::create(
            text_at,
            replace_range.0,
            replace_range.1,
            "Yorkie",
            BTreeMap::new(),
            Some(ticket(3)),
        )
        .execute(&mut root, OpSource::Remote, None)?;

        assert_eq!(
            r#"{"text":[{"val":"Hello "},{"val":"Yorkie"}]}"#,
            root.to_json()
        );
        assert_eq!(1, root.get_garbage_len());
        Ok(())
    }

    #[test]
    fn applies_remote_insert_and_delete_with_original_positions() -> crate::Result<()> {
        let text_at = ticket_actor(1, "a");
        let mut left = root_with_text_at(text_at.clone())?;
        insert_text(&mut left, &text_at, "AB", ticket_actor(2, "a"))?;
        let mut right = left.deepcopy();

        let delete_range = left
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 2)?;
        let insert_range = left
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(1, 1)?;
        let left_delete_at = ticket_actor(3, "a");
        let right_insert_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);

        EditOperation::create(
            text_at.clone(),
            delete_range.0.clone(),
            delete_range.1.clone(),
            "",
            BTreeMap::new(),
            Some(left_delete_at.clone()),
        )
        .execute(&mut left, OpSource::Remote, None)?;
        EditOperation::create(
            text_at.clone(),
            insert_range.0.clone(),
            insert_range.1.clone(),
            "C",
            BTreeMap::new(),
            Some(right_insert_at.clone()),
        )
        .execute(&mut right, OpSource::Remote, None)?;

        EditOperation::create(
            text_at.clone(),
            insert_range.0,
            insert_range.1,
            "C",
            BTreeMap::new(),
            Some(right_insert_at),
        )
        .execute(&mut left, OpSource::Remote, Some(&seen_base))?;
        EditOperation::create(
            text_at,
            delete_range.0,
            delete_range.1,
            "",
            BTreeMap::new(),
            Some(left_delete_at),
        )
        .execute(&mut right, OpSource::Remote, Some(&seen_base))?;

        assert_eq!(r#"{"text":[{"val":"C"}]}"#, left.to_json());
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
    }

    #[test]
    fn handles_composition_replacements() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        let commands = [
            (0, 0, "ㅎ", r#"{"text":[{"val":"ㅎ"}]}"#),
            (0, 1, "하", r#"{"text":[{"val":"하"}]}"#),
            (0, 1, "한", r#"{"text":[{"val":"한"}]}"#),
            (0, 1, "하", r#"{"text":[{"val":"하"}]}"#),
            (1, 1, "느", r#"{"text":[{"val":"하"},{"val":"느"}]}"#),
            (1, 2, "늘", r#"{"text":[{"val":"하"},{"val":"늘"}]}"#),
        ];

        for (idx, (from, to, content, expected)) in commands.into_iter().enumerate() {
            let range = root
                .text_by_created_at(&text_at)
                .unwrap()
                .index_range_to_pos_range(from, to)?;
            EditOperation::create(
                text_at.clone(),
                range.0,
                range.1,
                content,
                BTreeMap::new(),
                Some(ticket((idx + 2) as i64)),
            )
            .execute(&mut root, OpSource::Remote, None)?;

            assert_eq!(expected, root.to_json());
        }

        Ok(())
    }

    /// JS `text_removal_ledger_test.ts` `EditOperation empty undo` `runs as
    /// a no-op wherever it points`: an undo edit that inserts nothing, at an
    /// offset past the end of the text, moves to the head before any refine
    /// and changes nothing. Seed 152 of Go's `TestTextUndoFuzz` failed here.
    #[test]
    fn runs_an_empty_undo_as_a_no_op_wherever_it_points() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        insert_text(&mut root, &text_at, "0123456789", ticket(2))?;
        let stale = super::RgaTreeSplitPos::new(head_pos().id().clone(), 50);
        let mut undo = EditOperation::new(
            text_at.clone(),
            stale.clone(),
            stale,
            "",
            BTreeMap::new(),
            Some(ticket(100)),
            true,
        );

        undo.refine_undo_positions(&root)?;
        let result = undo.execute(&mut root, OpSource::UndoRedo, None)?.unwrap();

        assert!(result.op_infos.is_empty());
        assert_eq!(r#"{"text":[{"val":"0123456789"}]}"#, root.to_json());
        for pos in [undo.range_from(), undo.to_pos()] {
            assert_eq!(&head_pos(), pos);
        }
        let Some(Operation::Edit(reverse)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(0, reverse.range_from().relative_offset());
        assert_eq!(None, reverse.restore_mode());
        Ok(())
    }

    /// JS: a remote copy of an empty undo edit keeps no reverse.
    #[test]
    fn keeps_no_reverse_of_a_remote_empty_undo() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        let undo = EditOperation::new(
            text_at,
            head_pos(),
            head_pos(),
            "",
            BTreeMap::new(),
            Some(ticket(2)),
            true,
        );
        let result = undo.execute(&mut root, OpSource::Remote, None)?.unwrap();
        assert!(result.reverse_op.is_none());
        Ok(())
    }

    /// JS `toReverseOperation`: a deletion reverses to an identity edit
    /// that restores the removed pieces, anchored where they were.
    #[test]
    fn reverses_a_deletion_by_identity() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        insert_text(&mut root, &text_at, "abcd", ticket(2))?;

        let reverse = delete(&mut root, &text_at, 1, 3, ticket(3))?;

        assert_eq!(Some(RestoreMode::Restore), reverse.restore_mode());
        assert_eq!(
            vec![(ticket(2), 1, 3, "bc".to_owned())],
            spans_of(reverse.restore_spans())
        );
        assert!(reverse.retombstone_spans().is_empty());
        assert_eq!("", reverse.content());
        assert_eq!(reverse.range_from(), reverse.to_pos());
        assert_eq!(1, reverse.range_from().relative_offset());
        Ok(())
    }

    /// JS `toReverseOperation`: an insertion reverses to an identity edit
    /// that re-removes the inserted piece. The span carries the attributes
    /// the edit applied, stamped with its ticket, as a replica that purged
    /// the piece rebuilds it from them on a redo.
    #[test]
    fn reverses_an_insertion_by_identity_with_its_attributes() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        let range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 0)?;
        let attributes = BTreeMap::from([("b".to_owned(), "1".to_owned())]);
        let result = EditOperation::create(
            text_at,
            range.0,
            range.1,
            "hi",
            attributes.clone(),
            Some(ticket(2)),
        )
        .execute(&mut root, OpSource::Local, None)?
        .unwrap();

        let Some(Operation::Edit(reverse)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(Some(RestoreMode::Restore), reverse.restore_mode());
        assert!(reverse.restore_spans().is_empty());
        let span = &reverse.retombstone_spans()[0];
        assert_eq!(
            (ticket(2), 0, 2),
            (span.created_at.clone(), span.start, span.end)
        );
        assert_eq!("hi", span.value.content());
        assert_eq!(attributes, span.value.get_attributes());
        let attr = span
            .value
            .attributes()
            .iter()
            .next()
            .expect("the attribute");
        assert_eq!(&ticket(2), attr.updated_at());
        Ok(())
    }

    /// JS `toReverseOperation`: only an edit that removed and inserted
    /// nothing keeps the positional reverse.
    #[test]
    fn keeps_the_positional_reverse_of_an_edit_that_changed_nothing() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        insert_text(&mut root, &text_at, "abcd", ticket(2))?;
        let range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(2, 2)?;
        let result = EditOperation::create(
            text_at,
            range.0,
            range.1,
            "",
            BTreeMap::new(),
            Some(ticket(3)),
        )
        .execute(&mut root, OpSource::Local, None)?
        .unwrap();

        let Some(Operation::Edit(reverse)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(None, reverse.restore_mode());
        Ok(())
    }

    /// JS `execute`, the identity branch: the undo restores the pieces and
    /// reverses to a redo that re-removes them; the redo's reverse names
    /// only the pieces its removal was recorded on, here `b`, as `c` was
    /// removed by someone else meanwhile.
    #[test]
    fn names_only_the_recorded_pieces_in_a_redos_reverse() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        insert_text(&mut root, &text_at, "abcd", ticket(2))?;
        let mut undo = delete(&mut root, &text_at, 1, 3, ticket(3))?;
        undo.set_executed_at(ticket(4));
        let result = undo.execute(&mut root, OpSource::UndoRedo, None)?.unwrap();
        assert_eq!(
            r#"{"text":[{"val":"a"},{"val":"bc"},{"val":"d"}]}"#,
            root.to_json()
        );
        let Some(Operation::Edit(mut redo)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(Some(RestoreMode::Retombstone), redo.restore_mode());
        assert_eq!(undo.restore_spans(), redo.restore_spans());

        // B removes `c`, having seen it live.
        let c = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(2, 3)?;
        EditOperation::create(
            text_at.clone(),
            c.0,
            c.1,
            "",
            BTreeMap::new(),
            Some(ticket_actor(5, "b")),
        )
        .execute(
            &mut root,
            OpSource::Remote,
            Some(&vector([("a", 4), ("b", 5)])),
        )?;

        redo.set_executed_at(ticket(6));
        let result = redo.execute(&mut root, OpSource::UndoRedo, None)?.unwrap();
        assert_eq!(r#"{"text":[{"val":"a"},{"val":"d"}]}"#, root.to_json());
        let Some(Operation::Edit(reverse)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(Some(RestoreMode::Restore), reverse.restore_mode());
        assert_eq!(
            vec![(ticket(2), 1, 2, "b".to_owned())],
            spans_of(reverse.restore_spans())
        );
        Ok(())
    }

    /// JS `execute`, the identity branch: the undo of an insertion re-removes
    /// the inserted run, and its reverse, the redo, names only the pieces the
    /// removal was recorded on: here `x`, as B removed `y` meanwhile, having
    /// seen it live.
    #[test]
    fn names_only_the_recorded_pieces_in_an_insertion_undos_reverse() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        let range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 0)?;
        let result = EditOperation::create(
            text_at.clone(),
            range.0,
            range.1,
            "xy",
            BTreeMap::new(),
            Some(ticket(2)),
        )
        .execute(&mut root, OpSource::Local, None)?
        .unwrap();
        let Some(Operation::Edit(mut undo)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(
            vec![(ticket(2), 0, 2, "xy".to_owned())],
            spans_of(undo.retombstone_spans())
        );

        // B removes `y`, having seen it live.
        let y = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(1, 2)?;
        EditOperation::create(
            text_at,
            y.0,
            y.1,
            "",
            BTreeMap::new(),
            Some(ticket_actor(3, "b")),
        )
        .execute(
            &mut root,
            OpSource::Remote,
            Some(&vector([("a", 2), ("b", 3)])),
        )?;

        undo.set_executed_at(ticket(4));
        let result = undo.execute(&mut root, OpSource::UndoRedo, None)?.unwrap();
        assert_eq!(r#"{"text":[]}"#, root.to_json());
        let Some(Operation::Edit(redo)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(Some(RestoreMode::Retombstone), redo.restore_mode());
        assert!(redo.restore_spans().is_empty());
        assert_eq!(
            vec![(ticket(2), 0, 1, "x".to_owned())],
            spans_of(redo.retombstone_spans())
        );
        Ok(())
    }

    /// JS `identityReverse`: an identity edit whose spans are both empty
    /// reverses to an empty undo edit at the head, so the stacks stay
    /// paired and the reverse never goes out as an edit of a stale range.
    #[test]
    fn degrades_a_reverse_without_spans_to_an_empty_undo() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        insert_text(&mut root, &text_at, "abcd", ticket(2))?;
        let at_three = super::RgaTreeSplitPos::new(head_pos().id().clone(), 3);
        let identity = EditOperation::new(
            text_at,
            at_three.clone(),
            at_three,
            "",
            BTreeMap::new(),
            Some(ticket(3)),
            true,
        )
        .with_identity(Vec::new(), RestoreMode::Restore, Vec::new());

        let result = identity
            .execute(&mut root, OpSource::UndoRedo, None)?
            .unwrap();

        assert!(result.op_infos.is_empty());
        assert_eq!(r#"{"text":[{"val":"abcd"}]}"#, root.to_json());
        let Some(Operation::Edit(reverse)) = result.reverse_op else {
            panic!("expected an edit reverse");
        };
        assert_eq!(None, reverse.restore_mode());
        assert_eq!(&head_pos(), reverse.range_from());
        assert_eq!(&head_pos(), reverse.to_pos());
        Ok(())
    }

    /// JS `execute`: the spans a local identity edit re-removes are read
    /// against the live pieces first, so the reverse carries an attribute
    /// applied since; a remote run skips that, as its reverse is dropped.
    #[test]
    fn refreshes_the_spans_it_re_removes_only_when_not_remote() -> crate::Result<()> {
        for (source, styled) in [(OpSource::UndoRedo, true), (OpSource::Remote, false)] {
            let text_at = ticket(1);
            let mut root = root_with_text_at(text_at.clone())?;
            insert_text(&mut root, &text_at, "abcd", ticket(2))?;
            let mut undo = delete(&mut root, &text_at, 1, 3, ticket(3))?;
            undo.set_executed_at(ticket(4));
            let Some(Operation::Edit(mut redo)) = undo
                .execute(&mut root, OpSource::UndoRedo, None)?
                .unwrap()
                .reverse_op
            else {
                panic!("expected an edit reverse");
            };
            root.text_by_created_at_mut(&text_at)
                .unwrap()
                .set_style_by_index(
                    1,
                    3,
                    BTreeMap::from([("b".to_owned(), "1".to_owned())]),
                    ticket(5),
                    None,
                )?;

            redo.set_executed_at(ticket(6));
            let Some(Operation::Edit(reverse)) =
                redo.execute(&mut root, source, None)?.unwrap().reverse_op
            else {
                panic!("expected an edit reverse");
            };
            let attributes = reverse.restore_spans()[0].value.get_attributes();
            assert_eq!(styled, attributes.contains_key("b"), "{source:?}");
        }
        Ok(())
    }

    /// JS `execute`, the identity branch: a restore that revives part of a
    /// tombstone splits it, and the piece it revives was born by that split.
    /// The split pieces are registered before the revived ones are
    /// unregistered, or the revived piece would stay charged to gc. Here A
    /// undoes the removal of `cd` only (Go `TestTextRestore` "partial
    /// overlap" shape), and gc matches a root rebuilt from the content.
    #[test]
    fn registers_split_pieces_before_unregistering_revived_ones() -> crate::Result<()> {
        let text_at = ticket(1);
        let mut root = root_with_text_at(text_at.clone())?;
        insert_text(&mut root, &text_at, "abcdef", ticket(2))?;
        delete(&mut root, &text_at, 0, 6, ticket(3))?;
        let span = crate::crdt::rga_tree_split::RestoreSpan {
            created_at: ticket(2),
            start: 2,
            end: 4,
            value: crate::crdt::text::TextValue::create("cd"),
        };
        EditOperation::new(
            text_at,
            head_pos(),
            head_pos(),
            "",
            BTreeMap::new(),
            Some(ticket(4)),
            true,
        )
        .with_identity(vec![span], RestoreMode::Restore, Vec::new())
        .execute(&mut root, OpSource::UndoRedo, None)?;

        assert_eq!(r#"{"text":[{"val":"cd"}]}"#, root.to_json());
        let rebuilt = CrdtRoot::new(root.object().deepcopy());
        assert_eq!(rebuilt.doc_size().gc, root.doc_size().gc);
        assert_eq!(rebuilt.get_garbage_len(), root.get_garbage_len());
        Ok(())
    }

    /// Deletes `from..to` of the text at `text_at` and returns the reverse.
    fn delete(
        root: &mut CrdtRoot,
        text_at: &TimeTicket,
        from: usize,
        to: usize,
        at: TimeTicket,
    ) -> crate::Result<EditOperation> {
        let range = root
            .text_by_created_at(text_at)
            .unwrap()
            .index_range_to_pos_range(from, to)?;
        let result = EditOperation::create(
            text_at.clone(),
            range.0,
            range.1,
            "",
            BTreeMap::new(),
            Some(at),
        )
        .execute(root, OpSource::Local, None)?
        .unwrap();
        match result.reverse_op {
            Some(Operation::Edit(reverse)) => Ok(reverse),
            other => panic!("expected an edit reverse, got {other:?}"),
        }
    }

    fn spans_of(
        spans: &[crate::crdt::rga_tree_split::RestoreSpan<crate::crdt::text::TextValue>],
    ) -> Vec<(TimeTicket, usize, usize, String)> {
        spans
            .iter()
            .map(|span| {
                (
                    span.created_at.clone(),
                    span.start,
                    span.end,
                    span.value.content().to_owned(),
                )
            })
            .collect()
    }

    fn insert_text(
        root: &mut CrdtRoot,
        text_at: &TimeTicket,
        content: &str,
        edited_at: TimeTicket,
    ) -> crate::Result<()> {
        let range = root
            .text_by_created_at(text_at)
            .unwrap()
            .index_range_to_pos_range(0, 0)?;
        EditOperation::create(
            text_at.clone(),
            range.0,
            range.1,
            content,
            BTreeMap::new(),
            Some(edited_at),
        )
        .execute(root, OpSource::Remote, None)?;
        Ok(())
    }

    fn root_with_text() -> crate::Result<CrdtRoot> {
        root_with_text_at(ticket(1))
    }

    fn root_with_text_at(text_at: TimeTicket) -> crate::Result<CrdtRoot> {
        let mut root = CrdtRoot::create();
        SetOperation::create(
            "text",
            CrdtElement::text(CrdtText::create(text_at.clone())),
            TimeTicket::initial(),
            Some(text_at),
        )
        .execute(&mut root, OpSource::Remote)?;
        Ok(root)
    }

    fn ticket(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, "a")
    }

    fn ticket_actor(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }

    fn vector(entries: impl IntoIterator<Item = (&'static str, i64)>) -> VersionVector {
        let mut vector = VersionVector::new();
        for (actor_id, lamport) in entries {
            vector.set(actor_id, lamport);
        }
        vector
    }
}
