use super::{ExecutionResult, OpInfo, OpSource, Operation, OperationMeta, RemoveOperation};
use crate::crdt::element::CrdtElement;
use crate::crdt::root::CrdtRoot;
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SetOperation {
    meta: OperationMeta,
    key: String,
    value: CrdtElement,
}

impl SetOperation {
    pub(crate) fn new(
        key: impl Into<String>,
        value: CrdtElement,
        parent_created_at: TimeTicket,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self {
            meta: OperationMeta::new(parent_created_at, executed_at),
            key: key.into(),
            value,
        }
    }

    pub(crate) fn create(
        key: impl Into<String>,
        value: CrdtElement,
        parent_created_at: TimeTicket,
        executed_at: Option<TimeTicket>,
    ) -> Self {
        Self::new(key, value, parent_created_at, executed_at)
    }

    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        if source == OpSource::UndoRedo && self.has_removed_ancestor(root)? {
            return Ok(None);
        }

        let previous_value = root
            .get_object_member(self.parent_created_at(), &self.key)?
            .map(CrdtElement::deepcopy);
        let reverse_op = self.to_reverse_operation(previous_value.as_ref());
        let value = self.value.deepcopy();
        let executed_at = self.executed_at()?.clone();

        if source == OpSource::UndoRedo && root.find_by_created_at(value.created_at()).is_some() {
            root.deregister_element(&value);
        }

        root.set_object_member(
            self.parent_created_at(),
            self.key.clone(),
            value,
            executed_at,
        )?;

        let path = root.create_path(self.parent_created_at())?;
        Ok(Some(ExecutionResult {
            op_infos: vec![OpInfo::Set {
                path,
                key: self.key.clone(),
            }],
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

    pub(crate) fn effected_created_at(&self) -> &TimeTicket {
        self.value.created_at()
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn value(&self) -> &CrdtElement {
        &self.value
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}.SET.{}={}",
            self.parent_created_at().to_test_string(),
            self.key,
            self.value.to_sorted_json()
        )
    }

    fn to_reverse_operation(&self, value: Option<&CrdtElement>) -> Operation {
        if let Some(value) = value.filter(|value| !value.is_removed()) {
            return Operation::Set(Self::create(
                self.key.clone(),
                value.deepcopy(),
                self.parent_created_at().clone(),
                None,
            ));
        }

        Operation::Remove(RemoveOperation::create(
            self.parent_created_at().clone(),
            self.value.created_at().clone(),
        ))
    }

    fn has_removed_ancestor(&self, root: &CrdtRoot) -> Result<bool> {
        let mut current = root
            .find_by_created_at(self.parent_created_at())
            .ok_or_else(|| {
                YorkieError::MissingCrdtElement(self.parent_created_at().to_id_string())
            })?;

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
    use super::SetOperation;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::operation::{OpInfo, OpSource, Operation};
    use crate::{TimeTicket, YorkieError};

    #[test]
    fn sets_member_on_root_object() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let operation = SetOperation::create(
            "title",
            primitive_str("hello", created_at.clone()),
            TimeTicket::initial(),
            Some(created_at.clone()),
        );

        let result = operation.execute(&mut root, OpSource::Remote)?.unwrap();

        assert_eq!(r#"{"title":"hello"}"#, root.to_json());
        assert_eq!("$.title", root.create_path(&created_at)?);
        assert_eq!(2, root.get_element_map_size());
        assert_eq!(
            vec![OpInfo::Set {
                path: "$".to_owned(),
                key: "title".to_owned(),
            }],
            result.op_infos
        );
        assert!(matches!(result.reverse_op, Some(Operation::Remove(_))));
        Ok(())
    }

    #[test]
    fn registers_overwritten_member_as_removed() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        SetOperation::create(
            "title",
            primitive_str("old", t1.clone()),
            TimeTicket::initial(),
            Some(t1.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let result = SetOperation::create(
            "title",
            primitive_str("new", t2.clone()),
            TimeTicket::initial(),
            Some(t2.clone()),
        )
        .execute(&mut root, OpSource::Remote)?
        .unwrap();

        assert_eq!(r#"{"title":"new"}"#, root.to_json());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            Some(&t2),
            root.find_by_created_at(&t1).unwrap().removed_at()
        );
        assert!(matches!(result.reverse_op, Some(Operation::Set(_))));
        Ok(())
    }

    #[test]
    fn registers_lww_loser_as_removed() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let winner_at = ticket(2, "a");
        let loser_at = ticket(1, "b");

        SetOperation::create(
            "color",
            primitive_str("red", winner_at.clone()),
            TimeTicket::initial(),
            Some(winner_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        SetOperation::create(
            "color",
            primitive_str("blue", loser_at.clone()),
            TimeTicket::initial(),
            Some(loser_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        assert_eq!(r#"{"color":"red"}"#, root.to_json());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            Some(&winner_at),
            root.find_by_created_at(&loser_at).unwrap().removed_at()
        );
        Ok(())
    }

    #[test]
    fn sets_member_on_nested_object() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let profile_at = ticket(1, "a");
        let name_at = ticket(2, "a");
        let profile = CrdtElement::object(CrdtObject::create(profile_at.clone()));

        SetOperation::create(
            "profile",
            profile,
            TimeTicket::initial(),
            Some(profile_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        SetOperation::create(
            "name",
            primitive_str("yorkie", name_at.clone()),
            profile_at.clone(),
            Some(name_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, root.to_json());
        assert_eq!("$.profile.name", root.create_path(&name_at)?);
        Ok(())
    }

    #[test]
    fn skips_undo_redo_when_parent_is_removed() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let profile_at = ticket(1, "a");
        let removed_at = ticket(2, "a");
        let name_at = ticket(3, "a");
        let profile = CrdtElement::object(CrdtObject::create(profile_at.clone()));

        SetOperation::create(
            "profile",
            profile,
            TimeTicket::initial(),
            Some(profile_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let removed_profile = root
            .object_mut()
            .delete_by_key("profile", removed_at)
            .unwrap();
        root.register_removed_element(&removed_profile);

        let result = SetOperation::create(
            "name",
            primitive_str("yorkie", name_at),
            profile_at,
            Some(ticket(4, "a")),
        )
        .execute(&mut root, OpSource::UndoRedo)?;

        assert!(result.is_none());
        assert_eq!("{}", root.to_json());
        Ok(())
    }

    #[test]
    fn reports_non_object_parent() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let title_at = ticket(1, "a");

        SetOperation::create(
            "title",
            primitive_str("hello", title_at.clone()),
            TimeTicket::initial(),
            Some(title_at.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        let err = SetOperation::create(
            "name",
            primitive_str("yorkie", ticket(2, "a")),
            title_at.clone(),
            Some(ticket(2, "a")),
        )
        .execute(&mut root, OpSource::Remote)
        .unwrap_err();

        assert_eq!(
            YorkieError::UnexpectedCrdtElement {
                id: title_at.to_id_string(),
                expected: "object"
            },
            err
        );
        Ok(())
    }

    #[test]
    fn reports_missing_execution_time() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let operation = SetOperation::create(
            "title",
            primitive_str("hello", created_at),
            TimeTicket::initial(),
            None,
        );

        let err = operation.execute(&mut root, OpSource::Remote).unwrap_err();

        assert_eq!(YorkieError::MissingExecutionTime, err);
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
