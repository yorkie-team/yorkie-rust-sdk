use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::element::DataSize;
use crate::crdt::rga_tree_split::RgaTreeSplitPos;
use crate::crdt::rht::RhtNode;
use crate::crdt::root::CrdtRoot;
use crate::crdt::text::TextStyleChange;
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StyleOperation {
    meta: OperationMeta,
    from_pos: RgaTreeSplitPos,
    to_pos: RgaTreeSplitPos,
    attributes: BTreeMap<String, String>,
    attributes_to_remove: Vec<String>,
}

impl StyleOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        from_pos: RgaTreeSplitPos,
        to_pos: RgaTreeSplitPos,
        attributes: BTreeMap<String, String>,
        attributes_to_remove: Vec<String>,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            from_pos,
            to_pos,
            attributes,
            attributes_to_remove,
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        from_pos: RgaTreeSplitPos,
        to_pos: RgaTreeSplitPos,
        attributes: BTreeMap<String, String>,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(
            parent_created_at,
            from_pos,
            to_pos,
            attributes,
            Vec::new(),
            executed_at,
        )
    }

    pub(crate) fn create_remove_style_operation(
        parent_created_at: TimeTicket,
        from_pos: RgaTreeSplitPos,
        to_pos: RgaTreeSplitPos,
        attributes_to_remove: Vec<String>,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(
            parent_created_at,
            from_pos,
            to_pos,
            BTreeMap::new(),
            attributes_to_remove,
            executed_at,
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
        let range = (self.from_pos.clone(), self.to_pos.clone());

        let (changes, gc_nodes, diff, reverse_attributes, reverse_attributes_to_remove) = {
            let Some(text) = root.text_by_created_at_mut(self.parent_created_at()) else {
                return Err(self.text_parent_error(root));
            };

            let mut changes = Vec::new();
            let mut gc_nodes = Vec::new();
            let mut diff = DataSize::default();
            let mut reverse_attributes = BTreeMap::new();
            let mut reverse_attributes_to_remove = Vec::new();

            if !self.attributes_to_remove.is_empty() {
                let (removed_nodes, remove_diff, remove_changes, previous_attributes) = text
                    .remove_style_by_range_with_changes(
                        range.clone(),
                        &self.attributes_to_remove,
                        executed_at.clone(),
                        version_vector,
                    )?;
                add_data_size(&mut diff, remove_diff);
                gc_nodes.extend(removed_nodes);
                changes.extend(remove_changes);
                reverse_attributes.extend(previous_attributes);
            }

            if !self.attributes.is_empty() {
                let (
                    removed_nodes,
                    style_diff,
                    style_changes,
                    previous_attributes,
                    attributes_to_remove,
                ) = text.set_style_by_range_with_changes(
                    range,
                    self.attributes.clone(),
                    executed_at,
                    version_vector,
                )?;
                add_data_size(&mut diff, style_diff);
                gc_nodes.extend(removed_nodes);
                changes.extend(style_changes);
                reverse_attributes.extend(previous_attributes);
                reverse_attributes_to_remove.extend(attributes_to_remove);
            }

            (
                changes,
                gc_nodes,
                diff,
                reverse_attributes,
                reverse_attributes_to_remove,
            )
        };

        root.acc(diff);
        register_removed_attr_nodes(root, gc_nodes);

        let reverse_op =
            self.to_reverse_operation(reverse_attributes, reverse_attributes_to_remove);
        let op_infos = changes_to_op_infos(path, changes);

        Ok(Some(ExecutionResult {
            op_infos,
            reverse_op,
        }))
    }

    pub(crate) fn parent_created_at(&self) -> &TimeTicket {
        self.meta.parent_created_at()
    }

    pub(crate) fn executed_at(&self) -> Result<&TimeTicket> {
        self.meta.executed_at()
    }

    pub(crate) fn from_pos(&self) -> &RgaTreeSplitPos {
        &self.from_pos
    }

    pub(crate) fn to_pos(&self) -> &RgaTreeSplitPos {
        &self.to_pos
    }

    pub(crate) fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    pub(crate) fn attributes_to_remove(&self) -> &[String] {
        &self.attributes_to_remove
    }

    pub(crate) fn set_executed_at(&mut self, executed_at: TimeTicket) {
        self.meta.set_executed_at(executed_at);
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        self.meta.set_actor(actor_id);
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.STYL({},{},{:?})",
            self.parent_created_at().to_test_string(),
            self.from_pos.to_test_string(),
            self.to_pos.to_test_string(),
            self.attributes
        )
    }

    fn to_reverse_operation(
        &self,
        attributes: BTreeMap<String, String>,
        attributes_to_remove: Vec<String>,
    ) -> Option<Operation> {
        if attributes.is_empty() && attributes_to_remove.is_empty() {
            return None;
        }

        let operation = if attributes.is_empty() {
            Self::create_remove_style_operation(
                self.parent_created_at().clone(),
                self.from_pos.clone(),
                self.to_pos.clone(),
                attributes_to_remove,
                None,
            )
        } else if attributes_to_remove.is_empty() {
            Self::create(
                self.parent_created_at().clone(),
                self.from_pos.clone(),
                self.to_pos.clone(),
                attributes,
                None,
            )
        } else {
            Self::new(
                self.parent_created_at().clone(),
                self.from_pos.clone(),
                self.to_pos.clone(),
                attributes,
                attributes_to_remove,
                None,
            )
        };

        Some(Operation::Style(operation))
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

fn changes_to_op_infos(path: String, changes: Vec<TextStyleChange>) -> Vec<OpInfo> {
    changes
        .into_iter()
        .map(|change| OpInfo::Style {
            path: path.clone(),
            from: change.from,
            to: change.to,
            attributes: change.attributes,
            attributes_to_remove: change.attributes_to_remove,
        })
        .collect()
}

fn register_removed_attr_nodes(root: &mut CrdtRoot, nodes: Vec<RhtNode>) {
    for node in nodes {
        if let Some(removed_at) = node.removed_at() {
            root.register_gc_pair_by_id(node.id_string(), node.data_size(), removed_at.clone());
        }
    }
}

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

#[cfg(test)]
mod tests {
    use super::StyleOperation;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::text::CrdtText;
    use crate::operation::{EditOperation, OpInfo, OpSource, Operation, SetOperation};
    use crate::{TimeTicket, VersionVector};
    use std::collections::BTreeMap;

    #[test]
    fn applies_style_to_text_range() -> crate::Result<()> {
        let mut root = root_with_text()?;
        let text_at = ticket(1);
        insert_text(&mut root, &text_at, "Hello World", ticket(2))?;
        let range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 5)?;
        let attrs = BTreeMap::from([("b".to_owned(), "true".to_owned())]);

        let result =
            StyleOperation::create(text_at, range.0, range.1, attrs.clone(), Some(ticket(3)))
                .execute(&mut root, OpSource::Remote, None)?
                .unwrap();

        assert_eq!(
            r#"{"text":[{"attrs":{"b":true},"val":"Hello"},{"val":" World"}]}"#,
            root.to_json()
        );
        assert_eq!(
            vec![OpInfo::Style {
                path: "$.text".to_owned(),
                from: 0,
                to: 5,
                attributes: attrs,
                attributes_to_remove: Vec::new(),
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::Style(_))));
        Ok(())
    }

    #[test]
    fn removes_style_and_registers_gc_pairs() -> crate::Result<()> {
        let mut root = root_with_text()?;
        let text_at = ticket(1);
        insert_text(&mut root, &text_at, "Hello", ticket(2))?;
        let range = root
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 5)?;
        StyleOperation::create(
            text_at.clone(),
            range.0.clone(),
            range.1.clone(),
            BTreeMap::from([("b".to_owned(), "true".to_owned())]),
            Some(ticket(3)),
        )
        .execute(&mut root, OpSource::Remote, None)?;

        let result = StyleOperation::create_remove_style_operation(
            text_at,
            range.0,
            range.1,
            vec!["b".to_owned()],
            Some(ticket(4)),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(r#"{"text":[{"val":"Hello"}]}"#, root.to_json());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            vec![OpInfo::Style {
                path: "$.text".to_owned(),
                from: 0,
                to: 5,
                attributes: BTreeMap::new(),
                attributes_to_remove: vec!["b".to_owned()],
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::Style(_))));
        Ok(())
    }

    #[test]
    fn keeps_concurrent_insertions_unstyled_when_format_did_not_see_them() -> crate::Result<()> {
        let text_at = ticket_actor(1, "a");
        let mut left = root_with_text_at(text_at.clone())?;
        insert_text(&mut left, &text_at, "The fox jumped.", ticket_actor(2, "a"))?;
        let mut right = left.deepcopy();

        let style_range = left
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 15)?;
        let insert_range = left
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(4, 4)?;
        let style_at = ticket_actor(3, "a");
        let insert_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);
        let bold = BTreeMap::from([("bold".to_owned(), "true".to_owned())]);

        StyleOperation::create(
            text_at.clone(),
            style_range.0.clone(),
            style_range.1.clone(),
            bold.clone(),
            Some(style_at.clone()),
        )
        .execute(&mut left, OpSource::Remote, None)?;
        EditOperation::create(
            text_at.clone(),
            insert_range.0.clone(),
            insert_range.1.clone(),
            "brown ",
            BTreeMap::new(),
            Some(insert_at.clone()),
        )
        .execute(&mut right, OpSource::Remote, None)?;

        EditOperation::create(
            text_at.clone(),
            insert_range.0,
            insert_range.1,
            "brown ",
            BTreeMap::new(),
            Some(insert_at),
        )
        .execute(&mut left, OpSource::Remote, Some(&seen_base))?;
        StyleOperation::create(text_at, style_range.0, style_range.1, bold, Some(style_at))
            .execute(&mut right, OpSource::Remote, Some(&seen_base))?;

        assert_eq!(
            r#"{"text":[{"attrs":{"bold":true},"val":"The "},{"val":"brown "},{"attrs":{"bold":true},"val":"fox jumped."}]}"#,
            left.to_json()
        );
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
    }

    #[test]
    fn resolves_conflicting_overlapping_styles_by_lww() -> crate::Result<()> {
        let text_at = ticket_actor(1, "a");
        let mut left = root_with_text_at(text_at.clone())?;
        insert_text(&mut left, &text_at, "The fox jumped.", ticket_actor(2, "a"))?;
        let mut right = left.deepcopy();

        let left_range = left
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(0, 7)?;
        let right_range = left
            .text_by_created_at(&text_at)
            .unwrap()
            .index_range_to_pos_range(4, 15)?;
        let left_style_at = ticket_actor(3, "a");
        let right_style_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);
        let red = BTreeMap::from([("highlight".to_owned(), "\"red\"".to_owned())]);
        let blue = BTreeMap::from([("highlight".to_owned(), "\"blue\"".to_owned())]);

        StyleOperation::create(
            text_at.clone(),
            left_range.0.clone(),
            left_range.1.clone(),
            red.clone(),
            Some(left_style_at.clone()),
        )
        .execute(&mut left, OpSource::Remote, None)?;
        StyleOperation::create(
            text_at.clone(),
            right_range.0.clone(),
            right_range.1.clone(),
            blue.clone(),
            Some(right_style_at.clone()),
        )
        .execute(&mut right, OpSource::Remote, None)?;

        StyleOperation::create(
            text_at.clone(),
            right_range.0,
            right_range.1,
            blue,
            Some(right_style_at),
        )
        .execute(&mut left, OpSource::Remote, Some(&seen_base))?;
        StyleOperation::create(
            text_at,
            left_range.0,
            left_range.1,
            red,
            Some(left_style_at),
        )
        .execute(&mut right, OpSource::Remote, Some(&seen_base))?;

        assert_eq!(
            r#"{"text":[{"attrs":{"highlight":"red"},"val":"The "},{"attrs":{"highlight":"blue"},"val":"fox"},{"attrs":{"highlight":"blue"},"val":" jumped."}]}"#,
            left.to_json()
        );
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
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
