use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::rht::RhtNode;
use crate::crdt::root::CrdtRoot;
use crate::crdt::tree::{TreePos, TreeStyleChange};
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeStyleOperation {
    meta: OperationMeta,
    from_pos: TreePos,
    to_pos: TreePos,
    attributes: BTreeMap<String, String>,
    attributes_to_remove: Vec<String>,
}

impl TreeStyleOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        from_pos: TreePos,
        to_pos: TreePos,
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
        from_pos: TreePos,
        to_pos: TreePos,
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

    pub(crate) fn create_tree_remove_style_operation(
        parent_created_at: TimeTicket,
        from_pos: TreePos,
        to_pos: TreePos,
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
            let Some(tree) = root.tree_by_created_at_mut(self.parent_created_at()) else {
                return Err(self.tree_parent_error(root));
            };

            if !self.attributes_to_remove.is_empty() {
                let (removed_nodes, remove_diff, remove_changes, previous_attributes) = tree
                    .remove_style_by_range_with_changes(
                        range,
                        &self.attributes_to_remove,
                        executed_at,
                        version_vector,
                    )?;
                (
                    remove_changes,
                    removed_nodes,
                    remove_diff,
                    previous_attributes,
                    Vec::new(),
                )
            } else {
                let (
                    removed_nodes,
                    style_diff,
                    style_changes,
                    previous_attributes,
                    attributes_to_remove,
                ) = tree.style_by_range_with_changes(
                    range,
                    self.attributes.clone(),
                    executed_at,
                    version_vector,
                )?;
                (
                    style_changes,
                    removed_nodes,
                    style_diff,
                    previous_attributes,
                    attributes_to_remove,
                )
            }
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

    pub(crate) fn from_pos(&self) -> &TreePos {
        &self.from_pos
    }

    pub(crate) fn to_pos(&self) -> &TreePos {
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
            "{}.TREE_STYLE({},{},{:?})",
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
            Self::create_tree_remove_style_operation(
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

        Some(Operation::TreeStyle(operation))
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

fn changes_to_op_infos(path: String, changes: Vec<TreeStyleChange>) -> Vec<OpInfo> {
    changes
        .into_iter()
        .map(|change| OpInfo::TreeStyle {
            path: path.clone(),
            from: change.from,
            to: change.to,
            from_path: change.from_path,
            to_path: change.to_path,
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

#[cfg(test)]
mod tests {
    use super::TreeStyleOperation;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::operation::{OpInfo, OpSource, Operation};
    use crate::TimeTicket;
    use std::collections::BTreeMap;

    #[test]
    fn styles_tree_element_range() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone());
        let range = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .path_to_pos_range(&[0])?;

        let result = TreeStyleOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            BTreeMap::from([("bold".to_owned(), "true".to_owned())]),
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<root><p bold="true">hello</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(
            vec![OpInfo::TreeStyle {
                path: "$.body".to_owned(),
                from: 0,
                to: 1,
                from_path: vec![0],
                to_path: vec![0, 0],
                attributes: BTreeMap::from([("bold".to_owned(), "true".to_owned())]),
                attributes_to_remove: Vec::new(),
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::TreeStyle(_))));
        Ok(())
    }

    #[test]
    fn styles_text_only_range_by_splitting_boundaries() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone());
        let range = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (tree.find_pos(2, true)?, tree.find_pos(4, true)?)
        };

        let result = TreeStyleOperation::create(
            tree_at.clone(),
            range.0,
            range.1,
            BTreeMap::from([("bold".to_owned(), "true".to_owned())]),
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        let tree = root.tree_by_created_at(&tree_at).unwrap();
        assert_eq!(r#"<root><p>hello</p></root>"#, tree.to_xml());
        assert_eq!(5, tree.node_map_len());
        assert!(result.op_infos.is_empty());
        assert!(result.reverse_op.is_none());
        Ok(())
    }

    #[test]
    fn removes_tree_style_and_registers_gc() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut root = seeded_root(tree_at.clone());
        let range = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .path_to_pos_range(&[0])?;

        TreeStyleOperation::create(
            tree_at.clone(),
            range.0.clone(),
            range.1.clone(),
            BTreeMap::from([("bold".to_owned(), "true".to_owned())]),
            Some(ticket(10, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?;

        let result = TreeStyleOperation::create_tree_remove_style_operation(
            tree_at.clone(),
            range.0,
            range.1,
            vec!["bold".to_owned()],
            Some(ticket(11, "a")),
        )
        .execute(&mut root, OpSource::Remote, None)?
        .unwrap();

        assert_eq!(
            r#"<root><p>hello</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            vec![OpInfo::TreeStyle {
                path: "$.body".to_owned(),
                from: 0,
                to: 1,
                from_path: vec![0],
                to_path: vec![0, 0],
                attributes: BTreeMap::new(),
                attributes_to_remove: vec!["bold".to_owned()],
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::TreeStyle(_))));
        Ok(())
    }

    fn seeded_root(tree_at: TimeTicket) -> CrdtRoot {
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(tree_at.clone(), 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    TreeNodeId::new(ticket(2, "a"), 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(
                        TreeNodeId::new(ticket(3, "a"), 0),
                        "hello",
                    )],
                )],
            ),
            tree_at,
        );

        CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("body", CrdtElement::tree(tree))],
        ))
    }

    fn ticket(lamport: i64, actor: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor)
    }
}
