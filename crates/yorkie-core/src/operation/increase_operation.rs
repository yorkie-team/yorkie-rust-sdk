use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::element::CrdtElement;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IncreaseOperation {
    meta: OperationMeta,
    value: CrdtElement,
    actor: String,
}

impl IncreaseOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
        actor: Option<String>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            value,
            actor: actor.unwrap_or_default(),
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(parent_created_at, value, executed_at, None)
    }

    pub(crate) fn create_with_actor(
        parent_created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
        actor: impl Into<String>,
    ) -> Self {
        Self::new(parent_created_at, value, executed_at, Some(actor.into()))
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        _source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        self.executed_at()?;

        let value = match self.value.deepcopy() {
            CrdtElement::Primitive(value) => value,
            value => {
                return Err(YorkieError::UnexpectedCrdtElement {
                    id: value.created_at().to_id_string(),
                    expected: "primitive",
                });
            }
        };
        let reverse_op = if self.actor.is_empty() {
            Some(self.to_reverse_operation(&value))
        } else {
            None
        };

        root.increase_counter(self.parent_created_at(), &value)?;

        let path = root.create_path(self.parent_created_at())?;
        Ok(Some(ExecutionResult {
            op_infos: vec![OpInfo::Increase {
                path,
                value: value.value().clone(),
            }],
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

    pub(crate) fn effected_created_at(&self) -> &TimeTicket {
        self.parent_created_at()
    }

    pub(crate) fn value(&self) -> &CrdtElement {
        &self.value
    }

    pub(crate) fn actor(&self) -> &str {
        &self.actor
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.INCREASE.{}",
            self.parent_created_at().to_test_string(),
            self.value.to_json()
        )
    }

    fn to_reverse_operation(&self, value: &CrdtPrimitive) -> Operation {
        Operation::Increase(Self::create(
            self.parent_created_at().clone(),
            CrdtElement::primitive(CrdtPrimitive::new(
                negative_value(value.value()),
                value.created_at().clone(),
            )),
            None,
        ))
    }
}

fn negative_value(value: &PrimitiveValue) -> PrimitiveValue {
    match value {
        PrimitiveValue::Integer(value) if *value == i32::MIN => {
            PrimitiveValue::Long(i64::from(i32::MAX) + 1)
        }
        PrimitiveValue::Integer(value) => PrimitiveValue::Integer(-*value),
        PrimitiveValue::Long(value) => PrimitiveValue::Long(value.wrapping_neg()),
        PrimitiveValue::Double(value) => PrimitiveValue::Double(-value),
        value => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::IncreaseOperation;
    use crate::crdt::counter::{CounterType, CounterValue, CrdtCounter};
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{OpInfo, OpSource, Operation, SetOperation};
    use crate::{TimeTicket, YorkieError};

    #[test]
    fn increases_counter_and_reports_op_info() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let counter_at = ticket(1, "a");
        let increase_at = ticket(2, "a");

        create_counter(
            &mut root,
            "cnt",
            counter_at.clone(),
            CounterType::Integer,
            CounterValue::Integer(10),
        )?;

        let result = IncreaseOperation::create(
            counter_at.clone(),
            primitive(PrimitiveValue::Integer(5), increase_at.clone()),
            Some(increase_at),
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!(r#"{"cnt":15}"#, root.to_json());
        assert_eq!(
            vec![OpInfo::Increase {
                path: "$.cnt".to_owned(),
                value: PrimitiveValue::Integer(5),
            }],
            result.op_infos
        );
        match result.reverse_op {
            Some(Operation::Increase(operation)) => {
                assert_eq!("$.cnt", root.create_path(operation.parent_created_at())?);
                assert_eq!("-5", operation.value().to_json());
            }
            _ => panic!("expected increase reverse operation"),
        }
        Ok(())
    }

    #[test]
    fn increases_long_counter_with_wrapping() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let counter_at = ticket(1, "a");
        let increase_at = ticket(2, "a");

        create_counter(
            &mut root,
            "longCnt",
            counter_at.clone(),
            CounterType::Long,
            CounterValue::Long(i64::MAX),
        )?;

        IncreaseOperation::create(
            counter_at,
            primitive(PrimitiveValue::Long(1), increase_at.clone()),
            Some(increase_at),
        )
        .execute(&mut root, OpSource::Remote)?;

        assert_eq!(r#"{"longCnt":-9223372036854775808}"#, root.to_json());
        Ok(())
    }

    #[test]
    fn rejects_non_counter_parent() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let value_at = ticket(1, "a");

        SetOperation::create(
            "value",
            primitive(PrimitiveValue::Integer(1), value_at.clone()),
            TimeTicket::initial(),
            Some(value_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let err = IncreaseOperation::create(
            value_at.clone(),
            primitive(PrimitiveValue::Integer(1), ticket(2, "a")),
            Some(ticket(2, "a")),
        )
        .execute(&mut root, OpSource::Remote)
        .unwrap_err();

        assert_eq!(
            YorkieError::UnexpectedCrdtElement {
                id: value_at.to_id_string(),
                expected: "counter",
            },
            err
        );
        Ok(())
    }

    #[test]
    fn rejects_non_primitive_operand() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let counter_at = ticket(1, "a");
        let object_at = ticket(2, "a");

        create_counter(
            &mut root,
            "cnt",
            counter_at.clone(),
            CounterType::Integer,
            CounterValue::Integer(0),
        )?;

        let err = IncreaseOperation::create(
            counter_at,
            CrdtElement::object(CrdtObject::create(object_at.clone())),
            Some(ticket(3, "a")),
        )
        .execute(&mut root, OpSource::Remote)
        .unwrap_err();

        assert_eq!(
            YorkieError::UnexpectedCrdtElement {
                id: object_at.to_id_string(),
                expected: "primitive",
            },
            err
        );
        Ok(())
    }

    #[test]
    fn omits_reverse_operation_when_actor_is_present() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let counter_at = ticket(1, "a");
        let increase_at = ticket(2, "a");

        create_counter(
            &mut root,
            "cnt",
            counter_at.clone(),
            CounterType::Integer,
            CounterValue::Integer(0),
        )?;

        let result = IncreaseOperation::create_with_actor(
            counter_at,
            primitive(PrimitiveValue::Integer(1), increase_at.clone()),
            Some(increase_at),
            "actor-a",
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!(r#"{"cnt":1}"#, root.to_json());
        assert!(result.reverse_op.is_none());
        Ok(())
    }

    fn create_counter(
        root: &mut CrdtRoot,
        key: &str,
        created_at: TimeTicket,
        counter_type: CounterType,
        value: CounterValue,
    ) -> crate::Result<()> {
        SetOperation::create(
            key,
            CrdtElement::counter(CrdtCounter::create(counter_type, value, created_at.clone())),
            TimeTicket::initial(),
            Some(created_at),
        )
        .execute(root, OpSource::Remote)?;
        Ok(())
    }

    fn primitive(value: PrimitiveValue, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(value, created_at))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
