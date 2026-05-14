use super::OperationMeta;
use crate::TimeTicket;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoveOperation {
    meta: OperationMeta,
    created_at: TimeTicket,
}

impl RemoveOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        created_at: TimeTicket,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            created_at,
        }
    }

    pub(crate) fn create(parent_created_at: TimeTicket, created_at: TimeTicket) -> Self {
        Self::new(parent_created_at, created_at, None)
    }

    pub(crate) fn parent_created_at(&self) -> &TimeTicket {
        self.meta.parent_created_at()
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }
}
