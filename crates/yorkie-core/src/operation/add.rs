use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta, RemoveOperation};
use crate::crdt::element::CrdtElement;
use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AddOperation {
    meta: OperationMeta,
    prev_created_at: TimeTicket,
    value: CrdtElement,
}

impl AddOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        prev_created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            prev_created_at,
            value,
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        prev_created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(parent_created_at, prev_created_at, value, executed_at)
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        let value = self.value.deepcopy();
        let value_created_at = value.created_at().clone();

        if source == OpSource::UndoRedo && root.find_by_created_at(&value_created_at).is_some() {
            root.deregister_element(&value);
        }

        root.insert_array_element(
            self.parent_created_at(),
            &self.prev_created_at,
            value,
            self.executed_at()?.clone(),
        )?;

        let path = root.create_path(self.parent_created_at())?;
        let index = root
            .container_sub_path(self.parent_created_at(), &value_created_at)?
            .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
            .parse::<usize>()
            .map_err(|_| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?;

        Ok(Some(ExecutionResult {
            op_infos: vec![OpInfo::Add { path, index }],
            reverse_op: Some(Operation::Remove(RemoveOperation::create(
                self.parent_created_at().clone(),
                value_created_at,
            ))),
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
        self.value.created_at()
    }

    pub(crate) fn prev_created_at(&self) -> &TimeTicket {
        &self.prev_created_at
    }

    pub(crate) fn value(&self) -> &CrdtElement {
        &self.value
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.ADD.{}",
            self.parent_created_at().to_test_string(),
            self.value.to_json()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::AddOperation;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{OpInfo, OpSource, Operation, SetOperation};
    use crate::TimeTicket;

    #[test]
    fn adds_element_to_array() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let value_at = ticket(2, "a");

        SetOperation::create(
            "items",
            CrdtElement::array(CrdtArray::create(array_at.clone())),
            TimeTicket::initial(),
            Some(array_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let result = AddOperation::create(
            array_at.clone(),
            TimeTicket::initial(),
            primitive("one", value_at.clone()),
            Some(value_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!(r#"{"items":["one"]}"#, root.to_json());
        assert_eq!("$.items.0", root.create_path(&value_at)?);
        assert_eq!(
            vec![OpInfo::Add {
                path: "$.items".to_owned(),
                index: 0
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::Remove(_))));
        Ok(())
    }

    fn primitive(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
