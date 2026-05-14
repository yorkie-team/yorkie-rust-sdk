mod add;
mod array_set;
mod move_op;
mod remove;
mod set;

pub(crate) use add::AddOperation;
pub(crate) use array_set::ArraySetOperation;
pub(crate) use move_op::MoveOperation;
pub(crate) use remove::RemoveOperation;
pub(crate) use set::SetOperation;

use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::TimeTicket;

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpSource {
    Local,
    Remote,
    UndoRedo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpInfo {
    Set {
        path: String,
        key: String,
    },
    Remove {
        path: String,
        key: String,
    },
    ArrayRemove {
        path: String,
        index: usize,
    },
    Add {
        path: String,
        index: usize,
    },
    Move {
        path: String,
        index: usize,
        previous_index: usize,
    },
    ArraySet {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Operation {
    Set(SetOperation),
    Remove(RemoveOperation),
    Add(AddOperation),
    Move(MoveOperation),
    ArraySet(ArraySetOperation),
}

impl Operation {
    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        match self {
            Self::Set(operation) => operation.execute(root, source),
            Self::Remove(operation) => operation.execute(root, source),
            Self::Add(operation) => operation.execute(root, source),
            Self::Move(operation) => operation.execute(root, source),
            Self::ArraySet(operation) => operation.execute(root, source),
        }
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();
        match self {
            Self::Set(operation) => operation.set_actor(actor_id),
            Self::Remove(operation) => operation.set_actor(actor_id),
            Self::Add(operation) => operation.set_actor(actor_id),
            Self::Move(operation) => operation.set_actor(actor_id),
            Self::ArraySet(operation) => operation.set_actor(actor_id),
        }
    }

    pub(crate) fn to_test_string(&self) -> String {
        match self {
            Self::Set(operation) => operation.to_test_string(),
            Self::Remove(operation) => operation.to_test_string(),
            Self::Add(operation) => operation.to_test_string(),
            Self::Move(operation) => operation.to_test_string(),
            Self::ArraySet(operation) => operation.to_test_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExecutionResult {
    pub(crate) op_infos: Vec<OpInfo>,
    pub(crate) reverse_op: Option<Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationMeta {
    parent_created_at: TimeTicket,
    executed_at: Option<TimeTicket>,
}

impl OperationMeta {
    pub(crate) fn new(parent_created_at: TimeTicket, executed_at: Option<TimeTicket>) -> Self {
        Self {
            parent_created_at,
            executed_at,
        }
    }

    pub(crate) fn parent_created_at(&self) -> &TimeTicket {
        &self.parent_created_at
    }

    pub(crate) fn executed_at(&self) -> crate::Result<&TimeTicket> {
        self.executed_at
            .as_ref()
            .ok_or(crate::YorkieError::MissingExecutionTime)
    }

    pub(crate) fn set_executed_at(&mut self, executed_at: TimeTicket) {
        self.executed_at = Some(executed_at);
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();
        if let Some(executed_at) = &self.executed_at {
            self.executed_at = Some(executed_at.set_actor(actor_id));
        }
    }
}
