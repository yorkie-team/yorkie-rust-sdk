use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::root::CrdtRoot;
use crate::crdt::tree::{
    fallback_split_tickets, TreeEditChange, TreeEditResult, TreeNode, TreeNodeId, TreePos,
};
use crate::time::ActorId;
use crate::{Result, TimeTicket, VersionVector, YorkieError};

#[derive(Debug, Clone, PartialEq)]
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

            // The split takes the carried tickets in order, then the ones a
            // replica rebuilds from the edit and its top-level contents.
            let mut split_tickets = self.split_tickets.iter().cloned();
            let mut fallback =
                fallback_split_tickets(&executed_at, self.contents.as_ref().map(Vec::len));
            let mut issue_split_ticket = || split_tickets.next().unwrap_or_else(&mut fallback);
            let result = tree.edit(
                range,
                self.contents.clone(),
                self.split_level,
                executed_at,
                &mut issue_split_ticket,
                version_vector,
            )?;
            // The reverse is measured on the tree after the edit, from
            // indexes taken before it, which a concurrent merge can leave
            // outside the tree. Tree undo is not ported yet, so a reverse
            // that cannot be built is dropped rather than failing the edit,
            // which JS applies.
            //
            // NOTE: this drops every error, local edits included, and
            // `HistoryOperation::entry` drops the reverse that is built:
            // nothing reaches the undo stack today. A Tree undo port has to
            // keep the errors of local edits, or an undo entry goes missing
            // without notice.
            let reverse_op = self.to_reverse_operation(tree, &result).ok().flatten();
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

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.TREE_EDIT({},{},split={})",
            self.parent_created_at().to_test_string(),
            self.from_pos.to_test_string(),
            self.to_pos.to_test_string(),
            self.split_level
        )
    }

    /// Builds the reverse of this edit on the tree after it. The reverse is
    /// built for remote changes too, from indexes on this replica's tree, so
    /// it skips the surrogate pair check (JS `findPosUnchecked`): refusing
    /// one would refuse a change the caller never controlled.
    fn to_reverse_operation(
        &self,
        tree: &crate::crdt::tree::CrdtTree,
        result: &TreeEditResult,
    ) -> Result<Option<Operation>> {
        if self.split_level > 0 && result.inserted_size == 0 && result.removed_nodes.is_empty() {
            let boundary_size = 2 * self.split_level;
            if result.from_idx + boundary_size > tree.root().len() {
                return Ok(None);
            }
            let reverse_from = tree.find_pos_unchecked(result.from_idx, true)?;
            let reverse_to = tree.find_pos_unchecked(result.from_idx + boundary_size, true)?;
            return Ok(Some(Operation::TreeEdit(Self::create(
                self.parent_created_at().clone(),
                reverse_from,
                reverse_to,
                None,
                0,
                None,
            ))));
        }

        if result.merge_level > 0 {
            let reverse_from = tree.find_pos_unchecked(result.from_idx, true)?;
            return Ok(Some(Operation::TreeEdit(Self::create(
                self.parent_created_at().clone(),
                reverse_from.clone(),
                reverse_from,
                None,
                result.merge_level,
                None,
            ))));
        }

        if result.inserted_size > 0 {
            let reverse_from = tree.find_pos_unchecked(result.from_idx, true)?;
            let reverse_to =
                tree.find_pos_unchecked(result.from_idx + result.inserted_size, true)?;
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

        let reverse_from = tree.find_pos_unchecked(result.from_idx, true)?;
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
            // JS `TreeEditOpInfo.splitLevel` exists, but no change sets it: a
            // split is reported as an empty change where it opened.
            split_level: None,
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
    /// measured on the tree before the merge, so a reverse built from it may
    /// not fit the tree after it; Tree undo is not ported, and the edit
    /// still applies.
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
        TreeEditOperation::create(
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
        .execute(&mut root, OpSource::Remote, Some(&known))?;
        assert_eq!(
            "<root><p>abc</p>B</root>",
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );
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
    /// for remote changes too, so it takes the unchecked lookup.
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
        // A split is reversed first, so the merge reverse needs an edit that
        // does not split.
        let (split, merge) = (op(1), op(0));
        let result = |from_idx, merge_level, inserted_size| crate::crdt::tree::TreeEditResult {
            changes: Vec::new(),
            gc_pairs: Vec::new(),
            diff: crate::DataSize::default(),
            removed_nodes: Vec::new(),
            merge_level,
            from_idx,
            to_idx: from_idx,
            inserted_size,
        };

        // The copy reverse of an insertion ends inside the pair.
        assert!(split
            .to_reverse_operation(&tree, &result(1, 0, 1))?
            .is_some());
        // A merge reverse (a split) starts inside it.
        assert!(merge
            .to_reverse_operation(&tree, &result(2, 1, 0))?
            .is_some());
        // A split reverse starts inside it.
        assert!(split
            .to_reverse_operation(&tree, &result(2, 0, 0))?
            .is_some());
        assert!(split
            .to_reverse_operation(&tree, &result(1, 0, 0))?
            .is_some());
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
