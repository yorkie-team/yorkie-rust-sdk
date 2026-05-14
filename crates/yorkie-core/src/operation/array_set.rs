use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::element::CrdtElement;
use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArraySetOperation {
    meta: OperationMeta,
    created_at: TimeTicket,
    value: CrdtElement,
}

impl ArraySetOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            created_at,
            value,
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        created_at: TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(parent_created_at, created_at, value, executed_at)
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        let previous_value = root
            .get_container_child(self.parent_created_at(), &self.created_at)?
            .ok_or_else(|| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?;

        let value = self.value.deepcopy();
        let value_created_at = value.created_at().clone();
        let reverse_op = Operation::ArraySet(Self::create(
            self.parent_created_at().clone(),
            value_created_at.clone(),
            previous_value,
            None,
        ));

        if source == OpSource::UndoRedo && root.find_by_created_at(&value_created_at).is_some() {
            root.deregister_element(&value);
        }

        root.set_array_element(
            self.parent_created_at(),
            &self.created_at,
            value,
            self.executed_at()?.clone(),
        )?;

        let path = root.create_path(self.parent_created_at())?;
        Ok(Some(ExecutionResult {
            op_infos: vec![OpInfo::ArraySet { path }],
            reverse_op: Some(reverse_op),
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
        &self.created_at
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn value(&self) -> &CrdtElement {
        &self.value
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.ARRAY_SET.{}={}",
            self.parent_created_at().to_test_string(),
            self.created_at.to_test_string(),
            self.value.to_sorted_json()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ArraySetOperation;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{AddOperation, OpInfo, OpSource, Operation, SetOperation};
    use crate::TimeTicket;

    #[test]
    fn sets_element_in_array() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let old_at = ticket(2, "a");
        let new_at = ticket(3, "a");

        create_array(&mut root, array_at.clone())?;
        AddOperation::create(
            array_at.clone(),
            TimeTicket::initial(),
            primitive("old", old_at.clone()),
            Some(old_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let result = ArraySetOperation::create(
            array_at,
            old_at.clone(),
            primitive("new", new_at),
            Some(ticket(4, "a")),
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!(r#"{"items":["new"]}"#, root.to_json());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            vec![OpInfo::ArraySet {
                path: "$.items".to_owned()
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::ArraySet(_))));
        assert!(root.find_by_created_at(&old_at).unwrap().is_removed());
        Ok(())
    }

    fn create_array(root: &mut CrdtRoot, created_at: TimeTicket) -> crate::Result<()> {
        SetOperation::create(
            "items",
            CrdtElement::array(CrdtArray::create(created_at.clone())),
            TimeTicket::initial(),
            Some(created_at),
        )
        .execute(root, OpSource::Remote)?;
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
