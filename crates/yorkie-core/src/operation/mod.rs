mod add_operation;
mod array_set_operation;
mod edit_operation;
mod increase_operation;
mod move_operation;
mod remove_operation;
mod set_operation;
mod style_operation;

pub(crate) use add_operation::AddOperation;
pub(crate) use array_set_operation::ArraySetOperation;
pub(crate) use edit_operation::EditOperation;
pub(crate) use increase_operation::IncreaseOperation;
pub(crate) use move_operation::MoveOperation;
pub(crate) use remove_operation::RemoveOperation;
pub(crate) use set_operation::SetOperation;
pub(crate) use style_operation::StyleOperation;

use crate::crdt::primitive::PrimitiveValue;
use crate::crdt::root::CrdtRoot;
use crate::time::ActorId;
use crate::{TimeTicket, VersionVector};

use crate::Result;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpSource {
    Local,
    Remote,
    UndoRedo,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OpInfo {
    Set {
        path: String,
        key: String,
    },
    Remove {
        path: String,
        key: String,
    },
    ArrayRemove {
        path: String,
        index: usize,
    },
    Add {
        path: String,
        index: usize,
    },
    Increase {
        path: String,
        value: PrimitiveValue,
    },
    Move {
        path: String,
        index: usize,
        previous_index: usize,
    },
    ArraySet {
        path: String,
    },
    Edit {
        path: String,
        from: usize,
        to: usize,
        content: String,
        attributes: BTreeMap<String, String>,
    },
    Style {
        path: String,
        from: usize,
        to: usize,
        attributes: BTreeMap<String, String>,
        attributes_to_remove: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Operation {
    Set(SetOperation),
    Remove(RemoveOperation),
    Add(AddOperation),
    Increase(IncreaseOperation),
    Move(MoveOperation),
    ArraySet(ArraySetOperation),
    Edit(EditOperation),
    Style(StyleOperation),
}

impl Operation {
    pub(crate) fn execute(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
    ) -> Result<Option<ExecutionResult>> {
        self.execute_with_version_vector(root, source, None)
    }

    pub(crate) fn execute_with_version_vector(
        &self,
        root: &mut CrdtRoot,
        source: OpSource,
        version_vector: Option<&VersionVector>,
    ) -> Result<Option<ExecutionResult>> {
        match self {
            Self::Set(operation) => operation.execute(root, source),
            Self::Remove(operation) => operation.execute(root, source),
            Self::Add(operation) => operation.execute(root, source),
            Self::Increase(operation) => operation.execute(root, source),
            Self::Move(operation) => operation.execute(root, source),
            Self::ArraySet(operation) => operation.execute(root, source),
            Self::Edit(operation) => operation.execute(root, source, version_vector),
            Self::Style(operation) => operation.execute(root, source, version_vector),
        }
    }

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();
        match self {
            Self::Set(operation) => operation.set_actor(actor_id),
            Self::Remove(operation) => operation.set_actor(actor_id),
            Self::Add(operation) => operation.set_actor(actor_id),
            Self::Increase(operation) => operation.set_actor(actor_id),
            Self::Move(operation) => operation.set_actor(actor_id),
            Self::ArraySet(operation) => operation.set_actor(actor_id),
            Self::Edit(operation) => operation.set_actor(actor_id),
            Self::Style(operation) => operation.set_actor(actor_id),
        }
    }

    pub(crate) fn to_test_string(&self) -> String {
        match self {
            Self::Set(operation) => operation.to_test_string(),
            Self::Remove(operation) => operation.to_test_string(),
            Self::Add(operation) => operation.to_test_string(),
            Self::Increase(operation) => operation.to_test_string(),
            Self::Move(operation) => operation.to_test_string(),
            Self::ArraySet(operation) => operation.to_test_string(),
            Self::Edit(operation) => operation.to_test_string(),
            Self::Style(operation) => operation.to_test_string(),
        }
    }
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

    pub(crate) fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();
        if let Some(executed_at) = &self.executed_at {
            self.executed_at = Some(executed_at.set_actor(actor_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AddOperation, ArraySetOperation, MoveOperation, OpSource, Operation, RemoveOperation,
        SetOperation,
    };
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::root::CrdtRoot;
    use crate::TimeTicket;

    #[test]
    fn array_operations_converge_across_matrix_orders() -> crate::Result<()> {
        let operations = [
            MatrixOperation::InsertPrev,
            MatrixOperation::InsertPrevNext,
            MatrixOperation::MovePrev,
            MatrixOperation::MovePrevNext,
            MatrixOperation::MoveTarget,
            MatrixOperation::SetTarget,
            MatrixOperation::RemoveTarget,
        ];

        for &first_op in &operations {
            for &second_op in &operations {
                let tickets = MatrixTickets::new();
                let mut first = seeded_root(&tickets)?;
                apply_matrix_operation(&mut first, first_op, 0, &tickets)?;
                apply_matrix_operation(&mut first, second_op, 1, &tickets)?;

                let mut second = seeded_root(&tickets)?;
                apply_matrix_operation(&mut second, second_op, 1, &tickets)?;
                apply_matrix_operation(&mut second, first_op, 0, &tickets)?;

                assert_roots_converge(&first, &second, &tickets, first_op, second_op)?;
            }
        }

        Ok(())
    }

    #[test]
    fn array_operations_push_after_move_front_without_position_confusion() -> crate::Result<()> {
        let tickets = ArrayTickets::new();
        let mut root = seeded_integer_array(&tickets)?;

        move_front(
            &mut root,
            &tickets.array,
            tickets.two.clone(),
            tickets.move_one.clone(),
        )?;
        assert_eq!(r#"{"items":[2,0,1]}"#, root.to_json());

        push_integer(&mut root, &tickets.array, 3, tickets.three.clone())?;
        assert_eq!(r#"{"items":[2,0,1,3]}"#, root.to_json());

        move_front(
            &mut root,
            &tickets.array,
            tickets.three.clone(),
            tickets.move_two.clone(),
        )?;

        assert_eq!(r#"{"items":[3,2,0,1]}"#, root.to_json());
        assert_eq!("$.items.0", root.create_path(&tickets.three)?);
        assert_eq!("$.items.3", root.create_path(&tickets.one)?);
        Ok(())
    }

    #[test]
    fn array_operations_push_after_move_last_without_position_confusion() -> crate::Result<()> {
        let tickets = ArrayTickets::new();
        let mut root = seeded_integer_array(&tickets)?;

        move_last(
            &mut root,
            &tickets.array,
            tickets.zero.clone(),
            tickets.move_one.clone(),
        )?;
        assert_eq!(r#"{"items":[1,2,0]}"#, root.to_json());

        push_integer(&mut root, &tickets.array, 3, tickets.three.clone())?;

        assert_eq!(r#"{"items":[1,2,0,3]}"#, root.to_json());
        assert_eq!("$.items.3", root.create_path(&tickets.three)?);
        Ok(())
    }

    #[test]
    fn array_operations_insert_after_moved_element_uses_position_anchor() -> crate::Result<()> {
        let tickets = ArrayTickets::new();
        let mut root = seeded_integer_array(&tickets)?;

        move_front(
            &mut root,
            &tickets.array,
            tickets.two.clone(),
            tickets.move_one.clone(),
        )?;
        assert_eq!(r#"{"items":[2,0,1]}"#, root.to_json());

        let prev_at = array_pos_created_at(&root, &tickets.array, &tickets.one)?;
        AddOperation::create(
            tickets.array.clone(),
            prev_at,
            primitive_int(3, tickets.three.clone()),
            Some(tickets.three.clone()),
        )
        .execute(&mut root, OpSource::Remote)?;

        assert_eq!(r#"{"items":[2,0,1,3]}"#, root.to_json());
        assert_eq!("$.items.3", root.create_path(&tickets.three)?);
        Ok(())
    }

    #[test]
    fn array_operations_move_last_sequence_keeps_position_identity() -> crate::Result<()> {
        let tickets = ArrayTickets::new();
        let mut root = seeded_integer_array(&tickets)?;

        move_last(
            &mut root,
            &tickets.array,
            tickets.two.clone(),
            tickets.move_one.clone(),
        )?;
        assert_eq!(r#"{"items":[0,1,2]}"#, root.to_json());

        move_last(
            &mut root,
            &tickets.array,
            tickets.one.clone(),
            tickets.move_two.clone(),
        )?;
        assert_eq!(r#"{"items":[0,2,1]}"#, root.to_json());

        move_last(
            &mut root,
            &tickets.array,
            tickets.zero.clone(),
            tickets.move_three.clone(),
        )?;
        assert_eq!(r#"{"items":[2,1,0]}"#, root.to_json());
        assert_eq!("$.items.2", root.create_path(&tickets.zero)?);
        Ok(())
    }

    #[derive(Debug, Clone, Copy)]
    enum MatrixOperation {
        InsertPrev,
        InsertPrevNext,
        MovePrev,
        MovePrevNext,
        MoveTarget,
        SetTarget,
        RemoveTarget,
    }

    struct MatrixTickets {
        array: TimeTicket,
        one: TimeTicket,
        two: TimeTicket,
        three: TimeTicket,
        four: TimeTicket,
        operation_times: [TimeTicket; 2],
    }

    impl MatrixTickets {
        fn new() -> Self {
            Self {
                array: ticket(1, "a"),
                one: ticket(2, "a"),
                two: ticket(3, "a"),
                three: ticket(4, "a"),
                four: ticket(5, "a"),
                operation_times: [ticket(6, "a"), ticket(7, "a")],
            }
        }

        fn other_target(&self, client_id: usize) -> &TimeTicket {
            if client_id == 0 {
                &self.three
            } else {
                &self.four
            }
        }

        fn operation_time(&self, client_id: usize) -> TimeTicket {
            self.operation_times[client_id].clone()
        }

        fn tracked_ids(&self) -> Vec<TimeTicket> {
            vec![
                self.array.clone(),
                self.one.clone(),
                self.two.clone(),
                self.three.clone(),
                self.four.clone(),
                self.operation_times[0].clone(),
                self.operation_times[1].clone(),
            ]
        }
    }

    struct ArrayTickets {
        array: TimeTicket,
        zero: TimeTicket,
        one: TimeTicket,
        two: TimeTicket,
        three: TimeTicket,
        move_one: TimeTicket,
        move_two: TimeTicket,
        move_three: TimeTicket,
    }

    impl ArrayTickets {
        fn new() -> Self {
            Self {
                array: ticket(1, "a"),
                zero: ticket(2, "a"),
                one: ticket(3, "a"),
                two: ticket(4, "a"),
                three: ticket(5, "a"),
                move_one: ticket(6, "a"),
                move_two: ticket(7, "a"),
                move_three: ticket(8, "a"),
            }
        }
    }

    fn seeded_root(tickets: &MatrixTickets) -> crate::Result<CrdtRoot> {
        let mut root = CrdtRoot::create();
        Operation::Set(SetOperation::create(
            "items",
            CrdtElement::array(CrdtArray::create(tickets.array.clone())),
            TimeTicket::initial(),
            Some(tickets.array.clone()),
        ))
        .execute(&mut root, OpSource::Remote)?;

        for (prev_at, value, created_at) in [
            (TimeTicket::initial(), "1", tickets.one.clone()),
            (tickets.one.clone(), "2", tickets.two.clone()),
            (tickets.two.clone(), "3", tickets.three.clone()),
            (tickets.three.clone(), "4", tickets.four.clone()),
        ] {
            Operation::Add(AddOperation::create(
                tickets.array.clone(),
                prev_at,
                primitive(value, created_at.clone()),
                Some(created_at),
            ))
            .execute(&mut root, OpSource::Remote)?;
        }

        Ok(root)
    }

    fn seeded_integer_array(tickets: &ArrayTickets) -> crate::Result<CrdtRoot> {
        let mut root = CrdtRoot::create();
        Operation::Set(SetOperation::create(
            "items",
            CrdtElement::array(CrdtArray::create(tickets.array.clone())),
            TimeTicket::initial(),
            Some(tickets.array.clone()),
        ))
        .execute(&mut root, OpSource::Remote)?;

        for (prev_at, value, created_at) in [
            (TimeTicket::initial(), 0, tickets.zero.clone()),
            (tickets.zero.clone(), 1, tickets.one.clone()),
            (tickets.one.clone(), 2, tickets.two.clone()),
        ] {
            Operation::Add(AddOperation::create(
                tickets.array.clone(),
                prev_at,
                primitive_int(value, created_at.clone()),
                Some(created_at),
            ))
            .execute(&mut root, OpSource::Remote)?;
        }

        Ok(root)
    }

    fn apply_matrix_operation(
        root: &mut CrdtRoot,
        operation: MatrixOperation,
        client_id: usize,
        tickets: &MatrixTickets,
    ) -> crate::Result<()> {
        matrix_operation(operation, client_id, tickets).execute(root, OpSource::Remote)?;
        Ok(())
    }

    fn matrix_operation(
        operation: MatrixOperation,
        client_id: usize,
        tickets: &MatrixTickets,
    ) -> Operation {
        let executed_at = tickets.operation_time(client_id);
        let new_value = if client_id == 0 { "5" } else { "6" };

        match operation {
            MatrixOperation::InsertPrev => Operation::Add(AddOperation::create(
                tickets.array.clone(),
                tickets.two.clone(),
                primitive(new_value, executed_at.clone()),
                Some(executed_at),
            )),
            MatrixOperation::InsertPrevNext => Operation::Add(AddOperation::create(
                tickets.array.clone(),
                tickets.one.clone(),
                primitive(new_value, executed_at.clone()),
                Some(executed_at),
            )),
            MatrixOperation::MovePrev => Operation::Move(MoveOperation::create(
                tickets.array.clone(),
                tickets.two.clone(),
                tickets.other_target(client_id).clone(),
                Some(executed_at),
            )),
            MatrixOperation::MovePrevNext => Operation::Move(MoveOperation::create(
                tickets.array.clone(),
                tickets.one.clone(),
                tickets.other_target(client_id).clone(),
                Some(executed_at),
            )),
            MatrixOperation::MoveTarget => Operation::Move(MoveOperation::create(
                tickets.array.clone(),
                tickets.other_target(client_id).clone(),
                tickets.two.clone(),
                Some(executed_at),
            )),
            MatrixOperation::SetTarget => Operation::ArraySet(ArraySetOperation::create(
                tickets.array.clone(),
                tickets.two.clone(),
                primitive(new_value, executed_at.clone()),
                Some(executed_at),
            )),
            MatrixOperation::RemoveTarget => Operation::Remove(RemoveOperation::new(
                tickets.array.clone(),
                tickets.two.clone(),
                Some(executed_at),
            )),
        }
    }

    fn assert_roots_converge(
        first: &CrdtRoot,
        second: &CrdtRoot,
        tickets: &MatrixTickets,
        first_op: MatrixOperation,
        second_op: MatrixOperation,
    ) -> crate::Result<()> {
        assert_eq!(
            first.to_json(),
            second.to_json(),
            "{first_op:?} vs {second_op:?}"
        );
        assert_eq!(
            first.get_garbage_len(),
            second.get_garbage_len(),
            "{first_op:?} vs {second_op:?}"
        );
        assert_eq!(
            first.stats(),
            second.stats(),
            "{first_op:?} vs {second_op:?}"
        );
        assert_eq!(
            element_snapshots(first, tickets)?,
            element_snapshots(second, tickets)?,
            "{first_op:?} vs {second_op:?}"
        );
        Ok(())
    }

    fn element_snapshots(
        root: &CrdtRoot,
        tickets: &MatrixTickets,
    ) -> crate::Result<Vec<(String, bool, String)>> {
        let mut snapshots = Vec::new();

        for id in tickets.tracked_ids() {
            if let Some(element) = root.find_by_created_at(&id) {
                snapshots.push((
                    id.to_id_string(),
                    element.is_removed(),
                    root.create_path(&id)?,
                ));
            }
        }

        Ok(snapshots)
    }

    fn move_front(
        root: &mut CrdtRoot,
        array_at: &TimeTicket,
        target_at: TimeTicket,
        executed_at: TimeTicket,
    ) -> crate::Result<()> {
        Operation::Move(MoveOperation::create(
            array_at.clone(),
            TimeTicket::initial(),
            target_at,
            Some(executed_at),
        ))
        .execute(root, OpSource::Remote)?;
        Ok(())
    }

    fn move_last(
        root: &mut CrdtRoot,
        array_at: &TimeTicket,
        target_at: TimeTicket,
        executed_at: TimeTicket,
    ) -> crate::Result<()> {
        let prev_at = array_last_created_at(root, array_at)?;
        Operation::Move(MoveOperation::create(
            array_at.clone(),
            prev_at,
            target_at,
            Some(executed_at),
        ))
        .execute(root, OpSource::Remote)?;
        Ok(())
    }

    fn push_integer(
        root: &mut CrdtRoot,
        array_at: &TimeTicket,
        value: i32,
        created_at: TimeTicket,
    ) -> crate::Result<()> {
        let prev_at = array_last_created_at(root, array_at)?;
        Operation::Add(AddOperation::create(
            array_at.clone(),
            prev_at,
            primitive_int(value, created_at.clone()),
            Some(created_at),
        ))
        .execute(root, OpSource::Remote)?;
        Ok(())
    }

    fn array_last_created_at(root: &CrdtRoot, array_at: &TimeTicket) -> crate::Result<TimeTicket> {
        let array = root
            .array_by_created_at(array_at)
            .ok_or_else(|| crate::YorkieError::MissingCrdtElement(array_at.to_id_string()))?;
        Ok(array.get_last_created_at())
    }

    fn array_pos_created_at(
        root: &CrdtRoot,
        array_at: &TimeTicket,
        element_at: &TimeTicket,
    ) -> crate::Result<TimeTicket> {
        let array = root
            .array_by_created_at(array_at)
            .ok_or_else(|| crate::YorkieError::MissingCrdtElement(array_at.to_id_string()))?;
        array.pos_created_at(element_at)
    }

    fn primitive(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn primitive_int(value: i32, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::Integer(value),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
