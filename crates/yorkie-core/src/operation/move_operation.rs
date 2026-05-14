use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta};
use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MoveOperation {
    meta: OperationMeta,
    prev_created_at: TimeTicket,
    created_at: TimeTicket,
}

impl MoveOperation {
    pub(crate) fn new(
        parent_created_at: TimeTicket,
        prev_created_at: TimeTicket,
        created_at: TimeTicket,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            prev_created_at,
            created_at,
        }
    }

    pub(crate) fn create(
        parent_created_at: TimeTicket,
        prev_created_at: TimeTicket,
        created_at: TimeTicket,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(parent_created_at, prev_created_at, created_at, executed_at)
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        _source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        let (preserve_prev_created_at, previous_index) = {
            let array = root
                .array_by_created_at(self.parent_created_at())
                .ok_or_else(|| self.array_parent_error(root))?;
            let preserve_prev_created_at = array.get_prev_created_at(&self.created_at)?;
            let previous_index = array
                .sub_path_of(&self.created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?
                .parse::<usize>()
                .map_err(|_| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?;

            (preserve_prev_created_at, previous_index)
        };

        root.move_array_element(
            self.parent_created_at(),
            &self.prev_created_at,
            &self.created_at,
            self.executed_at()?.clone(),
        )?;

        let path = root.create_path(self.parent_created_at())?;
        let index = root
            .container_sub_path(self.parent_created_at(), &self.created_at)?
            .ok_or_else(|| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?
            .parse::<usize>()
            .map_err(|_| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?;

        Ok(Some(ExecutionResult {
            op_infos: vec![OpInfo::Move {
                path,
                index,
                previous_index,
            }],
            reverse_op: Some(Operation::Move(Self::create(
                self.parent_created_at().clone(),
                preserve_prev_created_at,
                self.created_at.clone(),
                None,
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
        &self.created_at
    }

    pub(crate) fn prev_created_at(&self) -> &TimeTicket {
        &self.prev_created_at
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!("{}.MOVE", self.parent_created_at().to_test_string())
    }

    fn array_parent_error(&self, root: &CrdtRoot) -> YorkieError {
        if root.find_by_created_at(self.parent_created_at()).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: self.parent_created_at().to_id_string(),
                expected: "array",
            };
        }

        YorkieError::MissingCrdtElement(self.parent_created_at().to_id_string())
    }
}

#[cfg(test)]
mod tests {
    use super::MoveOperation;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{AddOperation, OpInfo, OpSource, Operation, SetOperation};
    use crate::TimeTicket;

    #[test]
    fn moves_element_in_array() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let one_at = ticket(2, "a");
        let two_at = ticket(3, "a");
        let moved_at = ticket(4, "a");

        create_array(&mut root, array_at.clone())?;
        add(
            &mut root,
            array_at.clone(),
            TimeTicket::initial(),
            "one",
            one_at.clone(),
        )?;
        add(
            &mut root,
            array_at.clone(),
            one_at.clone(),
            "two",
            two_at.clone(),
        )?;

        let result = MoveOperation::create(
            array_at.clone(),
            two_at.clone(),
            one_at.clone(),
            Some(moved_at),
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!(r#"{"items":["two","one"]}"#, root.to_json());
        assert_eq!("$.items.1", root.create_path(&one_at)?);
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(1, root.stats().gc_pairs);
        assert_eq!(
            vec![OpInfo::Move {
                path: "$.items".to_owned(),
                index: 1,
                previous_index: 0
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::Move(_))));
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

    fn add(
        root: &mut CrdtRoot,
        array_at: TimeTicket,
        prev_at: TimeTicket,
        value: &str,
        created_at: TimeTicket,
    ) -> crate::Result<()> {
        AddOperation::create(
            array_at,
            prev_at,
            primitive(value, created_at.clone()),
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
