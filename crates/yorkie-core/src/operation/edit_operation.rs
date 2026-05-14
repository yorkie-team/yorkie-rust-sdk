use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::rga_tree_split::{RgaTreeSplitNode, RgaTreeSplitPos};
use crate::crdt::root::CrdtRoot;
use crate::crdt::text::TextValue;
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditOperation {
    meta: OperationMeta,
    from_pos: RgaTreeSplitPos,
    to_pos: RgaTreeSplitPos,
    content: String,
    attributes: BTreeMap<String, String>,
    is_undo_op: bool,
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

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        _source: OpSource,
        version_vector: Option<&VersionVector>,
    ) -> Result<Option<ExecutionResult>> {
        let path = root.create_path(self.parent_created_at())?;
        let executed_at = self.executed_at()?.clone();

        let (range, normalized_from, from_idx, to_idx) = {
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
            let (from_idx, to_idx) = text.find_indexes_from_range(&range)?;

            (range, normalized_from, from_idx, to_idx)
        };

        let attributes = (!self.attributes.is_empty()).then_some(self.attributes.clone());
        let (removed_nodes, diff, removed_values) = {
            let Some(text) = root.text_by_created_at_mut(self.parent_created_at()) else {
                return Err(self.text_parent_error(root));
            };
            text.edit_by_range(
                range,
                self.content.clone(),
                attributes,
                executed_at,
                version_vector,
            )?
        };

        root.acc(diff);
        register_removed_text_nodes(root, removed_nodes);

        Ok(Some(ExecutionResult {
            op_infos: vec![OpInfo::Edit {
                path,
                from: from_idx,
                to: to_idx,
                content: self.content.clone(),
                attributes: self.attributes.clone(),
            }],
            reverse_op: Some(self.to_reverse_operation(removed_values, normalized_from)),
        }))
    }

    pub(crate) fn parent_created_at(&self) -> &TimeTicket {
        self.meta.parent_created_at()
    }

    pub(crate) fn executed_at(&self) -> Result<&TimeTicket> {
        self.meta.executed_at()
    }

    pub(crate) fn set_executed_at(&mut self, executed_at: TimeTicket) {
        self.meta.set_executed_at(executed_at);
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        self.meta.set_actor(actor_id);
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.EDIT({},{},{})",
            self.parent_created_at().to_test_string(),
            self.from_pos.to_test_string(),
            self.to_pos.to_test_string(),
            self.content
        )
    }

    fn to_reverse_operation(
        &self,
        removed_values: Vec<TextValue>,
        from_pos: RgaTreeSplitPos,
    ) -> Operation {
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
            from_pos.relative_offset() + self.content.encode_utf16().count(),
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

fn register_removed_text_nodes(root: &mut CrdtRoot, nodes: Vec<RgaTreeSplitNode<TextValue>>) {
    for node in nodes {
        if let Some(removed_at) = node.removed_at() {
            root.register_gc_pair_by_id(node.id_string(), node.data_size(), removed_at.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EditOperation;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::text::CrdtText;
    use crate::operation::{OpInfo, OpSource, Operation, SetOperation};
    use crate::TimeTicket;
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

    fn root_with_text() -> crate::Result<CrdtRoot> {
        let mut root = CrdtRoot::create();
        let text_at = ticket(1);
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
}
