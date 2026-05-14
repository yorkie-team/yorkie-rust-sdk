use super::ChangeId;
use crate::crdt::root::CrdtRoot;
use crate::operation::{OpInfo, OpSource, Operation};
use crate::time::ActorId;
use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Change {
    id: ChangeId,
    operations: Vec<Operation>,
    message: Option<String>,
}

impl Change {
    pub(crate) fn new(id: ChangeId, operations: Vec<Operation>, message: Option<String>) -> Self {
        Self {
            id,
            operations,
            message,
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
        let mut op_infos = Vec::new();
        let mut operations = Vec::new();
        let mut reverse_ops = Vec::new();

        for operation in &self.operations {
            let Some(execution_result) = operation.execute(root, source)? else {
                continue;
            };

            op_infos.extend(execution_result.op_infos);
            operations.push(operation.clone());
            if let Some(reverse_op) = execution_result.reverse_op {
                reverse_ops.insert(0, reverse_op);
            }
        }

        Ok(ChangeExecutionResult {
            operations,
            op_infos,
            reverse_ops,
        })
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChangeExecutionResult {
    pub(crate) operations: Vec<Operation>,
    pub(crate) op_infos: Vec<OpInfo>,
    pub(crate) reverse_ops: Vec<Operation>,
}

#[cfg(test)]
mod tests {
    use super::Change;
    use crate::change::ChangeId;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{OpInfo, OpSource, Operation, RemoveOperation, SetOperation};
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

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }
}
