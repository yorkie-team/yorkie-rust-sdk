mod remove;
mod set;

pub(crate) use remove::RemoveOperation;
pub(crate) use set::SetOperation;

use crate::TimeTicket;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpSource {
    Local,
    Remote,
    UndoRedo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpInfo {
    Set { path: String, key: String },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Operation {
    Set(SetOperation),
    Remove(RemoveOperation),
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
}
