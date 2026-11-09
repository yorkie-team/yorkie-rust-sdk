use super::ChangeId;
use crate::crdt::root::CrdtRoot;
use crate::crdt::tree::TreeNodeId;
use crate::operation::{OpInfo, OpSource, Operation};
use crate::presence::{Presence, PresenceChange};
use crate::time::ActorId;
use crate::{Result, TimeTicket};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Change {
    id: ChangeId,
    operations: Vec<Operation>,
    message: Option<String>,
    presence_change: Option<PresenceChange>,
    /// Whether an undo or a redo produced this change (JS `isUndoRedo`).
    /// Local only: it picks the source a replay runs the change with.
    undo_redo: bool,
}

impl Change {
    pub(crate) fn new(id: ChangeId, operations: Vec<Operation>, message: Option<String>) -> Self {
        Self {
            id,
            operations,
            message,
            presence_change: None,
            undo_redo: false,
        }
    }

    /// Marks this change as produced by an undo or a redo (JS
    /// `markAsUndoRedo`).
    pub(crate) fn mark_as_undo_redo(&mut self) {
        self.undo_redo = true;
    }

    /// Returns the source a replay of this local change runs with (JS
    /// `replaySourceOf`): the source it first ran with.
    pub(crate) fn replay_source(&self) -> OpSource {
        if self.undo_redo {
            OpSource::UndoRedo
        } else {
            OpSource::Local
        }
    }

    /// Returns a copy of this change under the given ID and operations that
    /// keeps its presence change, message and undo/redo mark (JS
    /// `reissueActor` rebuilds a change with `Change.create` from those same
    /// fields). Without the mark, a replay would run an undo's change as
    /// `Local`.
    pub(crate) fn with_id_and_operations(&self, id: ChangeId, operations: Vec<Operation>) -> Self {
        Self {
            id,
            operations,
            message: self.message.clone(),
            presence_change: self.presence_change.clone(),
            undo_redo: self.undo_redo,
        }
    }

    pub(crate) fn with_presence_change(mut self, presence_change: Option<PresenceChange>) -> Self {
        self.presence_change = presence_change;
        self
    }

    pub(crate) fn presence_change(&self) -> Option<&PresenceChange> {
        self.presence_change.as_ref()
    }

    /// Applies this change's presence change to the actor's entry (JS
    /// `Change.execute`, presence part).
    pub(crate) fn apply_presence_change(&self, presences: &mut BTreeMap<ActorId, Presence>) {
        match &self.presence_change {
            Some(PresenceChange::Put(presence)) => {
                presences.insert(self.id.actor_id().clone(), presence.clone());
            }
            Some(PresenceChange::Patch(patch)) => {
                let entry = presences.entry(self.id.actor_id().clone()).or_default();
                for (key, value) in patch {
                    entry.insert(key.clone(), value.clone());
                }
            }
            Some(PresenceChange::Clear) => {
                presences.remove(self.id.actor_id());
            }
            None => {}
        }
    }

    pub(crate) fn create(
        id: ChangeId,
        operations: Vec<Operation>,
        message: Option<String>,
    ) -> Self {
        Self::new(id, operations, message)
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<ChangeExecutionResult> {
        let mut result = ChangeExecutionResult::default();
        for operation in &self.operations {
            self.execute_operation(operation, root, source, &mut result)?;
        }
        Ok(result)
    }

    /// Executes a change built from a history entry, with
    /// `OpSource::UndoRedo`.
    ///
    /// Each undo edit is first refined against the text as it stands when the
    /// edit runs, and keeps the refined positions: JS refines by mutating the
    /// operation inside `execute`, so those are the positions the change
    /// carries to other replicas. As in JS, an identity edit is not refined,
    /// and an undo edit that inserts nothing moves to the head instead of
    /// refining (see `EditOperation::refine_undo_positions`). A Tree undo
    /// edit resolves its stored indexes the same way (see
    /// `TreeEditOperation::resolve_undo_positions`).
    ///
    /// A Tree split that re-creates elements a merge removed (the undo of a
    /// merge, or the redo of a split) mints them under its split tickets,
    /// while the operations after it in this change may still name the
    /// removed ones. Each is re-pointed once the split has run and before
    /// the next operation runs, for the tickets the split really took: a
    /// level it never reached mints nothing, and re-pointing at its ticket
    /// would name a node no replica has. The pairs are returned in
    /// `tree_node_repoints` for the history stacks, which follow only once
    /// the change has committed (JS `onSplitTicketConsumed` and
    /// `repointRest` in `executeUndoRedoInternal`). JS runs the change on a
    /// clone first and keeps the pairs off the handler idempotent; this runs
    /// it once, so each ticket is seen once.
    pub(crate) fn execute_undo_redo(
        &mut self,
        root: &mut CrdtRoot,
    ) -> Result<ChangeExecutionResult> {
        let mut result = ChangeExecutionResult::default();
        for index in 0..self.operations.len() {
            match &mut self.operations[index] {
                Operation::Edit(operation) => operation.refine_undo_positions(root)?,
                Operation::TreeEdit(operation) => operation.resolve_undo_positions(root)?,
                _ => {}
            }
            let executed_before = result.operations.len();
            self.execute_operation(
                &self.operations[index],
                root,
                OpSource::UndoRedo,
                &mut result,
            )?;
            if result.operations.len() == executed_before {
                continue;
            }
            let (Operation::TreeEdit(operation), Some(Operation::TreeEdit(executed))) =
                (&self.operations[index], result.operations.last())
            else {
                continue;
            };
            // Read off the operation as it stands: an earlier split of this
            // change may have re-pointed its `replaced_ids`.
            let executed_at = operation.executed_at()?;
            let repoints = operation
                .replaced_ids()
                .iter()
                .zip(operation.split_tickets())
                .take(executed.consumed_split_tickets())
                .enumerate()
                .map(|(ticket_index, (prev, ticket))| TreeNodeRepoint {
                    executed_at: executed_at.clone(),
                    parent_created_at: operation.parent_created_at().clone(),
                    prev: prev.clone(),
                    curr: TreeNodeId::new(ticket.clone(), 0),
                    ticket_index: Some(ticket_index),
                })
                .collect::<Vec<_>>();
            for repoint in &repoints {
                for later in &mut self.operations[index + 1..] {
                    repoint.apply_to(later);
                }
            }
            result.tree_node_repoints.extend(repoints);
        }
        Ok(result)
    }

    fn execute_operation(
        &self,
        operation: &Operation,
        root: &mut CrdtRoot,
        source: OpSource,
        result: &mut ChangeExecutionResult,
    ) -> Result<()> {
        let Some(execution_result) =
            operation.execute_with_version_vector(root, source, Some(self.id.version_vector()))?
        else {
            return Ok(());
        };

        result.op_infos.extend(execution_result.op_infos);
        // JS keeps what a Tree edit's run measured on the operation itself;
        // here it rides on the copy recorded as executed.
        let mut executed = operation.clone();
        if let (Operation::TreeEdit(executed), Some(facts)) =
            (&mut executed, execution_result.tree_edit)
        {
            executed.set_execution(facts);
        }
        result.operations.push(executed);
        if let Some(reverse_op) = execution_result.reverse_op {
            result.reverse_ops.insert(0, reverse_op);
        }
        Ok(())
    }

    pub(crate) fn id(&self) -> &ChangeId {
        &self.id
    }

    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub(crate) fn has_operations(&self) -> bool {
        !self.operations.is_empty()
    }

    pub(crate) fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();

        for operation in &mut self.operations {
            operation.set_actor(actor_id.clone());
        }

        self.id = self.id.set_actor(actor_id);
    }

    pub(crate) fn to_test_string(&self) -> String {
        self.operations
            .iter()
            .map(Operation::to_test_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ChangeExecutionResult {
    pub(crate) operations: Vec<Operation>,
    pub(crate) op_infos: Vec<OpInfo>,
    pub(crate) reverse_ops: Vec<Operation>,
    /// The Tree nodes an undo or a redo split re-minted, which the history
    /// stacks have to follow (see `Change::execute_undo_redo`).
    pub(crate) tree_node_repoints: Vec<TreeNodeRepoint>,
}

/// A Tree node an undo or a redo minted under a new ID in place of `prev`,
/// recorded so the rest of its entry and the history stacks can follow it
/// (an entry of JS `pending` in `executeUndoRedoInternal`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeNodeRepoint {
    /// The ticket of the Tree edit that minted the node, which names it
    /// within its change.
    pub(crate) executed_at: TimeTicket,
    pub(crate) parent_created_at: TimeTicket,
    pub(crate) prev: TreeNodeId,
    pub(crate) curr: TreeNodeId,
    /// Which of the edit's split tickets `curr` is; `None` for a content ID
    /// the edit re-issued, which is minted whether or not the split runs.
    pub(crate) ticket_index: Option<usize>,
}

impl TreeNodeRepoint {
    /// Re-points `operation` from `prev` to `curr` if it is a Tree edit or
    /// style. Like JS `repointRest`, it is not scoped to one tree: it runs
    /// on the operations of one change, and IDs are change tickets.
    pub(crate) fn apply_to(&self, operation: &mut Operation) {
        match operation {
            Operation::TreeEdit(operation) => operation.reconcile_node_id(&self.prev, &self.curr),
            Operation::TreeStyle(operation) => operation.reconcile_node_id(&self.prev, &self.curr),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Change;
    use crate::change::ChangeId;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::operation::{
        OpInfo, OpSource, Operation, RemoveOperation, SetOperation, TreeEditOperation,
    };
    use crate::TimeTicket;

    #[test]
    fn executes_operations_and_collects_execution_info() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let id = ChangeId::initial().next(false);
        let title_at = id.create_time_ticket(1);
        let change = Change::create(
            id,
            vec![Operation::Set(SetOperation::create(
                "title",
                primitive_str("hello", title_at.clone()),
                TimeTicket::initial(),
                Some(title_at.clone()),
            ))],
            Some("set title".to_owned()),
        );

        let result = change.execute(&mut root, OpSource::Local)?;

        assert_eq!(r#"{"title":"hello"}"#, root.to_json());
        assert_eq!(1, result.operations.len());
        assert_eq!(1, result.reverse_ops.len());
        assert_eq!(
            vec![OpInfo::Set {
                path: "$".to_owned(),
                key: "title".to_owned(),
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_ops[0], Operation::Remove(_)));
        assert_eq!(Some("set title"), change.message());
        assert!(change.has_operations());
        assert_eq!("0:00:0.SET.title=\"hello\"", change.to_test_string());
        Ok(())
    }

    #[test]
    fn stacks_reverse_operations_in_reverse_execution_order() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let id = ChangeId::initial().next(false);
        let title_at = id.create_time_ticket(1);
        let remove_at = id.create_time_ticket(2);
        let change = Change::create(
            id,
            vec![
                Operation::Set(SetOperation::create(
                    "title",
                    primitive_str("hello", title_at.clone()),
                    TimeTicket::initial(),
                    Some(title_at.clone()),
                )),
                Operation::Remove(RemoveOperation::new(
                    TimeTicket::initial(),
                    title_at,
                    Some(remove_at),
                )),
            ],
            None,
        );

        let result = change.execute(&mut root, OpSource::Local)?;

        assert_eq!("{}", root.to_json());
        assert_eq!(2, result.operations.len());
        assert_eq!(2, result.reverse_ops.len());
        assert!(matches!(result.reverse_ops[0], Operation::Set(_)));
        assert!(matches!(result.reverse_ops[1], Operation::Remove(_)));
        assert_eq!(
            vec![
                OpInfo::Set {
                    path: "$".to_owned(),
                    key: "title".to_owned(),
                },
                OpInfo::Remove {
                    path: "$".to_owned(),
                    key: "title".to_owned(),
                },
            ],
            result.op_infos
        );
        Ok(())
    }

    #[test]
    fn sets_actor_on_change_and_operations() {
        let id = ChangeId::initial().next(false);
        let title_at = id.create_time_ticket(1);
        let mut change = Change::create(
            id,
            vec![Operation::Set(SetOperation::create(
                "title",
                primitive_str("hello", title_at),
                TimeTicket::initial(),
                Some(TimeTicket::new(1, 1, "actor-a")),
            ))],
            None,
        );

        change.set_actor("actor-b");

        assert_eq!("actor-b", change.id().actor_id().as_str());
        match &change.operations()[0] {
            Operation::Set(operation) => {
                assert_eq!(
                    "actor-b",
                    operation.executed_at().unwrap().actor_id().as_str()
                );
            }
            _ => panic!("expected set operation"),
        }
    }

    /// JS keeps what a Tree edit's run measured on the operation, which its
    /// `execute` mutates: here it rides on the copy recorded as executed,
    /// and leaves the operation's equality alone.
    #[test]
    fn records_what_a_tree_edit_measured_on_its_executed_copy() -> crate::Result<()> {
        let (mut root, tree_at) = tree_root();
        let id = ChangeId::initial().next(false);
        let pos = root
            .tree_by_created_at(&tree_at)
            .unwrap()
            .find_pos(1, true)?;
        let operation = Operation::TreeEdit(TreeEditOperation::create(
            tree_at.clone(),
            pos.clone(),
            pos,
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(id.create_time_ticket(1), 0),
                "X",
            )]),
            0,
            Some(id.create_time_ticket(1)),
        ));
        let change = Change::create(id, vec![operation.clone()], None);

        let result = change.execute(&mut root, OpSource::Local)?;

        assert_eq!("<root><p>Xab</p></root>", tree_xml(&root, &tree_at));
        let Operation::TreeEdit(executed) = &result.operations[0] else {
            panic!("expected a tree edit");
        };
        let facts = executed.execution().unwrap();
        assert_eq!(Some(1), facts.last_from_idx);
        assert_eq!(Some(1), facts.inserted_content_size);
        assert_eq!(vec![(1, 1, 1)], executed.executed_ranges());
        assert_eq!(operation, result.operations[0]);
        Ok(())
    }

    /// JS `TreeEditOperation.execute` resolves an undo edit's stored indexes
    /// and keeps the positions on the operation, so the change carries them
    /// to other replicas; `execute_undo_redo` does it before each runs.
    #[test]
    fn resolves_tree_undo_positions_before_they_run() -> crate::Result<()> {
        let (mut root, tree_at) = tree_root();
        let id = ChangeId::initial().next(false);
        let (stale, from, to) = {
            let tree = root.tree_by_created_at(&tree_at).unwrap();
            (
                tree.find_pos(0, true)?,
                tree.find_pos(1, true)?,
                tree.find_pos(2, true)?,
            )
        };
        let undo = TreeEditOperation::create(
            tree_at.clone(),
            stale.clone(),
            stale,
            None,
            0,
            Some(id.create_time_ticket(1)),
        )
        .into_undo(1, 2);
        let mut change = Change::create(id, vec![Operation::TreeEdit(undo)], None);

        change.execute_undo_redo(&mut root)?;

        assert_eq!("<root><p>b</p></root>", tree_xml(&root, &tree_at));
        let Operation::TreeEdit(carried) = &change.operations()[0] else {
            panic!("expected a tree edit");
        };
        assert_eq!((&from, &to), (carried.range_from(), carried.to_pos()));
        Ok(())
    }

    fn tree_root() -> (CrdtRoot, TimeTicket) {
        // Created by the initial actor, so every change knows the nodes.
        let ticket =
            |delimiter| TimeTicket::new(0, delimiter, TimeTicket::initial().actor_id().clone());
        let tree_at = ticket(1);
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(tree_at.clone(), 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    TreeNodeId::new(ticket(2), 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(TreeNodeId::new(ticket(3), 0), "ab")],
                )],
            ),
            tree_at.clone(),
        );
        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("body", CrdtElement::tree(tree))],
        ));
        (root, tree_at)
    }

    fn tree_xml(root: &CrdtRoot, tree_at: &TimeTicket) -> String {
        root.tree_by_created_at(tree_at).unwrap().to_xml()
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }
}
