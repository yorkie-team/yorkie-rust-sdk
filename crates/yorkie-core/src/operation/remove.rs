use super::{
    AddOperation, ExecutionResult, OpInfo, OpSource, Operation, OperationMeta, SetOperation,
};
use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::{Result, TimeTicket, YorkieError};

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

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        if source == OpSource::UndoRedo && self.has_removed_target_or_ancestor(root)? {
            return Ok(None);
        }

        let sub_path = root
            .container_sub_path(self.parent_created_at(), &self.created_at)?
            .ok_or_else(|| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?
            .to_owned();
        let is_array_parent = root.array_by_created_at(self.parent_created_at()).is_some();
        let reverse_op = self.to_reverse_operation(root)?;
        let executed_at = self.executed_at()?.clone();

        root.remove_container_element(self.parent_created_at(), &self.created_at, executed_at)?;

        let path = root.create_path(self.parent_created_at())?;
        let op_infos = if is_array_parent {
            vec![OpInfo::ArrayRemove {
                path,
                index: sub_path
                    .parse::<usize>()
                    .map_err(|_| YorkieError::MissingCrdtElement(self.created_at.to_id_string()))?,
            }]
        } else {
            vec![OpInfo::Remove {
                path,
                key: sub_path,
            }]
        };

        Ok(Some(ExecutionResult {
            op_infos,
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

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn set_created_at(&mut self, created_at: TimeTicket) {
        self.created_at = created_at;
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.REMOVE.{}",
            self.parent_created_at().to_test_string(),
            self.created_at.to_test_string()
        )
    }

    fn to_reverse_operation(&self, root: &CrdtRoot) -> Result<Option<Operation>> {
        if let Some(array) = root.array_by_created_at(self.parent_created_at()) {
            let Some(value) = array.get_by_id(self.created_at()) else {
                return Ok(None);
            };
            let prev_created_at = array.get_prev_created_at(self.created_at())?;

            return Ok(Some(Operation::Add(AddOperation::create(
                self.parent_created_at().clone(),
                prev_created_at,
                value.deepcopy(),
                None,
            ))));
        }

        let Some(key) = root.object_member_sub_path(self.parent_created_at(), self.created_at())?
        else {
            return Ok(None);
        };

        let Some(value) = root.get_object_member(self.parent_created_at(), key)? else {
            return Ok(None);
        };

        Ok(Some(Operation::Set(SetOperation::create(
            key.to_owned(),
            value.deepcopy(),
            self.parent_created_at().clone(),
            None,
        ))))
    }

    fn has_removed_target_or_ancestor(&self, root: &CrdtRoot) -> Result<bool> {
        if root
            .container_sub_path(self.parent_created_at(), self.created_at())?
            .is_none()
        {
            return Ok(false);
        }

        let Some(mut current) = root.find_by_created_at(self.created_at()) else {
            return Ok(false);
        };

        loop {
            if current.removed_at().is_some() {
                return Ok(true);
            }

            let Some(pair) = root.find_element_pair_by_created_at(current.created_at()) else {
                return Ok(false);
            };

            let Some(parent) = pair.parent() else {
                return Ok(false);
            };

            current = parent;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RemoveOperation;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{AddOperation, OpInfo, OpSource, Operation, SetOperation};
    use crate::{TimeTicket, YorkieError};

    #[test]
    fn removes_member_from_root_object() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let title_at = ticket(1, "a");
        let removed_at = ticket(2, "a");

        set(
            &mut root,
            "title",
            primitive_str("hello", title_at.clone()),
            TimeTicket::initial(),
            title_at.clone(),
        )?;

        let result = RemoveOperation::new(
            TimeTicket::initial(),
            title_at.clone(),
            Some(removed_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!("{}", root.to_json());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            Some(&removed_at),
            root.find_by_created_at(&title_at).unwrap().removed_at()
        );
        assert_eq!(
            vec![OpInfo::Remove {
                path: "$".to_owned(),
                key: "title".to_owned(),
            }],
            result.op_infos
        );
        match result.reverse_op {
            Some(Operation::Set(operation)) => {
                assert_eq!("title", operation.key());
                assert_eq!("\"hello\"", operation.value().to_json());
            }
            _ => panic!("expected set reverse operation"),
        }
        Ok(())
    }

    #[test]
    fn removes_nested_object_member() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let profile_at = ticket(1, "a");
        let name_at = ticket(2, "a");
        let removed_at = ticket(3, "a");

        set(
            &mut root,
            "profile",
            CrdtElement::object(CrdtObject::create(profile_at.clone())),
            TimeTicket::initial(),
            profile_at.clone(),
        )?;
        set(
            &mut root,
            "name",
            primitive_str("yorkie", name_at.clone()),
            profile_at.clone(),
            name_at.clone(),
        )?;

        let result = RemoveOperation::new(profile_at.clone(), name_at, Some(removed_at))
            .execute(&mut root, OpSource::Remote)?
            .unwrap();

        assert_eq!(r#"{"profile":{}}"#, root.to_json());
        assert_eq!(
            vec![OpInfo::Remove {
                path: "$.profile".to_owned(),
                key: "name".to_owned(),
            }],
            result.op_infos
        );
        Ok(())
    }

    #[test]
    fn removes_array_element() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let value_at = ticket(2, "a");
        let removed_at = ticket(3, "a");

        set(
            &mut root,
            "items",
            CrdtElement::array(CrdtArray::create(array_at.clone())),
            TimeTicket::initial(),
            array_at.clone(),
        )?;
        AddOperation::create(
            array_at.clone(),
            TimeTicket::initial(),
            primitive_str("one", value_at.clone()),
            Some(value_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let result = RemoveOperation::new(array_at, value_at.clone(), Some(removed_at.clone()))
            .execute(&mut root, OpSource::Remote)?
            .unwrap();

        assert_eq!(r#"{"items":[]}"#, root.to_json());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            Some(&removed_at),
            root.find_by_created_at(&value_at).unwrap().removed_at()
        );
        assert_eq!(
            vec![OpInfo::ArrayRemove {
                path: "$.items".to_owned(),
                index: 0,
            }],
            result.op_infos
        );
        match result.reverse_op {
            Some(Operation::Add(operation)) => {
                assert_eq!(&TimeTicket::initial(), operation.prev_created_at());
                assert_eq!("\"one\"", operation.value().to_json());
            }
            _ => panic!("expected add reverse operation"),
        }
        Ok(())
    }

    #[test]
    fn skips_undo_redo_when_target_is_removed() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let title_at = ticket(1, "a");

        set(
            &mut root,
            "title",
            primitive_str("hello", title_at.clone()),
            TimeTicket::initial(),
            title_at.clone(),
        )?;

        RemoveOperation::new(
            TimeTicket::initial(),
            title_at.clone(),
            Some(ticket(2, "a")),
        )
        .execute(&mut root, OpSource::Remote)?;

        let result = RemoveOperation::new(TimeTicket::initial(), title_at, Some(ticket(3, "a")))
            .execute(&mut root, OpSource::UndoRedo)?;

        assert!(result.is_none());
        assert_eq!("{}", root.to_json());
        assert_eq!(1, root.get_garbage_len());
        Ok(())
    }

    #[test]
    fn reports_non_object_parent() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let title_at = ticket(1, "a");

        set(
            &mut root,
            "title",
            primitive_str("hello", title_at.clone()),
            TimeTicket::initial(),
            title_at.clone(),
        )?;

        let err = RemoveOperation::new(title_at.clone(), ticket(2, "a"), Some(ticket(3, "a")))
            .execute(&mut root, OpSource::Remote)
            .unwrap_err();

        assert_eq!(
            YorkieError::UnexpectedCrdtElement {
                id: title_at.to_id_string(),
                expected: "object or array"
            },
            err
        );
        Ok(())
    }

    #[test]
    fn reports_missing_execution_time() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let title_at = ticket(1, "a");

        set(
            &mut root,
            "title",
            primitive_str("hello", title_at.clone()),
            TimeTicket::initial(),
            title_at.clone(),
        )?;

        let err = RemoveOperation::create(TimeTicket::initial(), title_at)
            .execute(&mut root, OpSource::Remote)
            .unwrap_err();

        assert_eq!(YorkieError::MissingExecutionTime, err);
        Ok(())
    }

    fn set(
        root: &mut CrdtRoot,
        key: &str,
        value: CrdtElement,
        parent_created_at: TimeTicket,
        executed_at: TimeTicket,
    ) -> crate::Result<()> {
        SetOperation::create(key, value, parent_created_at, Some(executed_at))
            .execute(root, OpSource::Remote)?;

        Ok(())
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
