use super::{Change, ChangeId};
use crate::operation::Operation;
use crate::{TimeTicket, INITIAL_DELIMITER};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChangeContext {
    prev_id: ChangeId,
    next_id: ChangeId,
    delimiter: u32,
    message: Option<String>,
    operations: Vec<Operation>,
}

impl ChangeContext {
    pub(crate) fn new(prev_id: ChangeId, message: Option<String>) -> Self {
        let next_id = prev_id.next(false);
        Self {
            prev_id,
            next_id,
            delimiter: INITIAL_DELIMITER,
            message,
            operations: Vec::new(),
        }
    }

    pub(crate) fn create(prev_id: ChangeId, message: Option<String>) -> Self {
        Self::new(prev_id, message)
    }

    pub(crate) fn push(&mut self, operation: Operation) {
        self.operations.push(operation);
    }

    pub(crate) fn issue_time_ticket(&mut self) -> TimeTicket {
        self.delimiter += 1;
        self.next_id.create_time_ticket(self.delimiter)
    }

    pub(crate) fn last_time_ticket(&self) -> TimeTicket {
        self.next_id.create_time_ticket(self.delimiter)
    }

    pub(crate) fn next_id(&self) -> ChangeId {
        if self.is_presence_only_change() {
            return self
                .prev_id
                .next(true)
                .set_lamport(self.prev_id.lamport())
                .set_version_vector(self.prev_id.version_vector().clone());
        }

        self.next_id.clone()
    }

    pub(crate) fn to_change(&self) -> Change {
        let id = if self.is_presence_only_change() {
            self.prev_id.next(true)
        } else {
            self.next_id.clone()
        };

        Change::new(id, self.operations.clone(), self.message.clone())
    }

    pub(crate) fn is_presence_only_change(&self) -> bool {
        self.operations.is_empty()
    }

    pub(crate) fn has_change(&self) -> bool {
        !self.operations.is_empty()
    }

    pub(crate) fn operations(&self) -> &[Operation] {
        &self.operations
    }
}

#[cfg(test)]
mod tests {
    use super::ChangeContext;
    use crate::change::ChangeId;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::operation::{Operation, SetOperation};
    use crate::TimeTicket;

    #[test]
    fn issues_time_tickets_from_next_id() {
        let mut context = ChangeContext::new(ChangeId::initial(), None);

        let first = context.issue_time_ticket();
        let second = context.issue_time_ticket();

        assert_eq!(1, first.lamport());
        assert_eq!(1, first.delimiter());
        assert_eq!(1, second.lamport());
        assert_eq!(2, second.delimiter());
        assert_eq!(second, context.last_time_ticket());
    }

    #[test]
    fn creates_change_from_pushed_operations() {
        let mut context = ChangeContext::new(ChangeId::initial(), Some("set title".to_owned()));
        let ticket = context.issue_time_ticket();
        context.push(Operation::Set(SetOperation::create(
            "title",
            primitive_str("hello", ticket.clone()),
            TimeTicket::initial(),
            Some(ticket),
        )));

        let change = context.to_change();

        assert!(context.has_change());
        assert_eq!(1, context.next_id().client_seq());
        assert_eq!(1, context.next_id().lamport());
        assert_eq!(1, change.id().client_seq());
        assert_eq!(Some("set title"), change.message());
        assert_eq!("0:00:0.SET.title=\"hello\"", change.to_test_string());
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }
}
