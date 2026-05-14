use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::root::CrdtRoot;
use crate::crdt::tree::{TreeEditChange, TreeEditResult, TreeNode, TreeNodeId, TreePos};
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeEditOperation {
    meta: OperationMeta,
    from_pos: TreePos,
    to_pos: TreePos,
    contents: Option<Vec<TreeNode>>,
    split_level: usize,
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

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        _source: OpSource,
        version_vector: Option<&VersionVector>,
    ) -> Result<Option<ExecutionResult>> {
        let path = root.create_path(self.parent_created_at())?;
        let executed_at = self.executed_at()?.clone();
        let range = (self.from_pos.clone(), self.to_pos.clone());

        let (result, reverse_op) = {
            let Some(tree) = root.tree_by_created_at_mut(self.parent_created_at()) else {
                return Err(self.tree_parent_error(root));
            };

            let result = tree.edit_by_range_with_changes(
                range,
                self.contents.clone(),
                self.split_level,
                executed_at,
                version_vector,
            )?;
            let reverse_op = self.to_reverse_operation(tree, &result)?;
            (result, reverse_op)
        };

        root.acc(result.diff);
        for (child_id, child_size, removed_at) in &result.gc_pairs {
            root.register_gc_pair_by_id(child_id.clone(), *child_size, removed_at.clone());
        }

        Ok(Some(ExecutionResult {
            op_infos: changes_to_op_infos(path, result.changes),
            reverse_op,
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
            "{}.TREE_EDIT({},{},split={})",
            self.parent_created_at().to_test_string(),
            self.from_pos.to_test_string(),
            self.to_pos.to_test_string(),
            self.split_level
        )
    }

    fn to_reverse_operation(
        &self,
        tree: &crate::crdt::tree::CrdtTree,
        result: &TreeEditResult,
    ) -> Result<Option<Operation>> {
        if result.inserted_size > 0 {
            let reverse_from = tree.find_pos(result.from_idx, true)?;
            let reverse_to = tree.find_pos(result.from_idx + result.inserted_size, true)?;
            return Ok(Some(Operation::TreeEdit(Self::create(
                self.parent_created_at().clone(),
                reverse_from,
                reverse_to,
                None,
                0,
                None,
            ))));
        }

        let mut contents = top_level_removed_nodes(&result.removed_nodes);
        if contents.is_empty() {
            return Ok(None);
        }

        for node in &mut contents {
            node.clear_removed_recursively();
        }

        let reverse_from = tree.find_pos(result.from_idx, true)?;
        Ok(Some(Operation::TreeEdit(Self::create(
            self.parent_created_at().clone(),
            reverse_from.clone(),
            reverse_from,
            Some(contents),
            0,
            None,
        ))))
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

fn changes_to_op_infos(path: String, changes: Vec<TreeEditChange>) -> Vec<OpInfo> {
    changes
        .into_iter()
        .map(|change| OpInfo::TreeEdit {
            path: path.clone(),
            from: change.from,
            to: change.to,
            from_path: change.from_path,
            to_path: change.to_path,
            value: change.value,
            split_level: change.split_level,
        })
        .collect()
}

fn top_level_removed_nodes(nodes: &[TreeNode]) -> Vec<TreeNode> {
    let mut top_level = Vec::new();

    for (index, node) in nodes.iter().enumerate() {
        let nested_in_removed_node = nodes.iter().enumerate().any(|(other_index, other)| {
            other_index != index && tree_node_contains_id(other, node.id())
        });

        if !nested_in_removed_node {
            top_level.push(node.clone());
        }
    }

    top_level
}

fn tree_node_contains_id(node: &TreeNode, id: &TreeNodeId) -> bool {
    node.all_children()
        .any(|child| child.id() == id || tree_node_contains_id(child, id))
}

#[cfg(test)]
mod tests {
    use super::TreeEditOperation;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::operation::{OpInfo, OpSource, Operation};
    use crate::TimeTicket;

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
                value: Some(vec![
                    r#"{"type":"p","children":[{"type":"text","value":"hello"}]}"#.to_owned()
                ]),
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
        let range = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .path_to_pos_range(&[0])?;

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
                to: 1,
                from_path: vec![0],
                to_path: vec![0, 0],
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
                assert_eq!(
                    Some(vec![r#"{"type":"text","value":"X"}"#.to_owned()]),
                    *value
                );
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
