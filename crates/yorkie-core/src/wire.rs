use crate::change::{Change, ChangeId, ChangePack, Checkpoint};
use crate::crdt::counter::CounterType;
use crate::crdt::element::CrdtElement;
use crate::crdt::primitive::PrimitiveValue;
use crate::operation::Operation;
use crate::{Result, TimeTicket, VersionVector, YorkieError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireValueType {
    Null,
    Boolean,
    Integer,
    Long,
    Double,
    String,
    Bytes,
    Date,
    JsonObject,
    JsonArray,
    Text,
    IntegerCnt,
    LongCnt,
    IntegerDedupCnt,
    Tree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireJsonElementSimple {
    pub created_at: TimeTicket,
    pub moved_at: Option<TimeTicket>,
    pub removed_at: Option<TimeTicket>,
    pub value_type: WireValueType,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireOperation {
    Set {
        parent_created_at: TimeTicket,
        key: String,
        value: WireJsonElementSimple,
        executed_at: TimeTicket,
    },
    Add {
        parent_created_at: TimeTicket,
        prev_created_at: TimeTicket,
        value: WireJsonElementSimple,
        executed_at: TimeTicket,
    },
    Move {
        parent_created_at: TimeTicket,
        prev_created_at: TimeTicket,
        created_at: TimeTicket,
        executed_at: TimeTicket,
    },
    Remove {
        parent_created_at: TimeTicket,
        created_at: TimeTicket,
        executed_at: TimeTicket,
    },
    Increase {
        parent_created_at: TimeTicket,
        value: WireJsonElementSimple,
        executed_at: TimeTicket,
        actor: Option<String>,
    },
    ArraySet {
        parent_created_at: TimeTicket,
        created_at: TimeTicket,
        value: WireJsonElementSimple,
        executed_at: TimeTicket,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireChangeId {
    pub client_seq: u32,
    pub server_seq: i64,
    pub lamport: i64,
    pub actor_id: String,
    pub version_vector: VersionVector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireChange {
    pub id: WireChangeId,
    pub message: Option<String>,
    pub operations: Vec<WireOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireChangePack {
    pub document_key: String,
    pub checkpoint: Checkpoint,
    pub is_removed: bool,
    pub changes: Vec<WireChange>,
    pub snapshot: Option<Vec<u8>>,
    pub version_vector: Option<VersionVector>,
}

impl TryFrom<&ChangePack> for WireChangePack {
    type Error = YorkieError;

    fn try_from(pack: &ChangePack) -> Result<Self> {
        let changes = pack
            .changes()
            .iter()
            .map(WireChange::try_from)
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            document_key: pack.document_key().to_owned(),
            checkpoint: pack.checkpoint(),
            is_removed: pack.is_removed(),
            changes,
            snapshot: pack.snapshot().map(ToOwned::to_owned),
            version_vector: pack.version_vector().cloned(),
        })
    }
}

impl TryFrom<&Change> for WireChange {
    type Error = YorkieError;

    fn try_from(change: &Change) -> Result<Self> {
        let operations = change
            .operations()
            .iter()
            .map(WireOperation::try_from)
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            id: WireChangeId::from(change.id()),
            message: change.message().map(ToOwned::to_owned),
            operations,
        })
    }
}

impl From<&ChangeId> for WireChangeId {
    fn from(id: &ChangeId) -> Self {
        Self {
            client_seq: id.client_seq(),
            server_seq: id.server_seq(),
            lamport: id.lamport(),
            actor_id: id.actor_id().to_string(),
            version_vector: id.version_vector().clone(),
        }
    }
}

impl TryFrom<&Operation> for WireOperation {
    type Error = YorkieError;

    fn try_from(operation: &Operation) -> Result<Self> {
        match operation {
            Operation::Set(operation) => Ok(Self::Set {
                parent_created_at: operation.parent_created_at().clone(),
                key: operation.key().to_owned(),
                value: WireJsonElementSimple::try_from(operation.value())?,
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Add(operation) => Ok(Self::Add {
                parent_created_at: operation.parent_created_at().clone(),
                prev_created_at: operation.prev_created_at().clone(),
                value: WireJsonElementSimple::try_from(operation.value())?,
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Move(operation) => Ok(Self::Move {
                parent_created_at: operation.parent_created_at().clone(),
                prev_created_at: operation.prev_created_at().clone(),
                created_at: operation.created_at().clone(),
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Remove(operation) => Ok(Self::Remove {
                parent_created_at: operation.parent_created_at().clone(),
                created_at: operation.created_at().clone(),
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Increase(operation) => Ok(Self::Increase {
                parent_created_at: operation.parent_created_at().clone(),
                value: WireJsonElementSimple::try_from(operation.value())?,
                executed_at: operation.executed_at()?.clone(),
                actor: (!operation.actor().is_empty()).then(|| operation.actor().to_owned()),
            }),
            Operation::ArraySet(operation) => Ok(Self::ArraySet {
                parent_created_at: operation.parent_created_at().clone(),
                created_at: operation.created_at().clone(),
                value: WireJsonElementSimple::try_from(operation.value())?,
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Edit(_) => Err(YorkieError::UnsupportedProtocolConversion("edit operation")),
            Operation::Style(_) => Err(YorkieError::UnsupportedProtocolConversion(
                "style operation",
            )),
            Operation::TreeEdit(_) => Err(YorkieError::UnsupportedProtocolConversion(
                "tree edit operation",
            )),
            Operation::TreeStyle(_) => Err(YorkieError::UnsupportedProtocolConversion(
                "tree style operation",
            )),
        }
    }
}

impl TryFrom<&CrdtElement> for WireJsonElementSimple {
    type Error = YorkieError;

    fn try_from(element: &CrdtElement) -> Result<Self> {
        let value_type = match element {
            CrdtElement::Primitive(value) => primitive_value_type(value.value()),
            CrdtElement::Counter(value) => counter_value_type(value.counter_type()),
            CrdtElement::Text(_) => WireValueType::Text,
            CrdtElement::Object(_) => {
                return Err(YorkieError::UnsupportedProtocolConversion(
                    "object element simple",
                ))
            }
            CrdtElement::Array(_) => {
                return Err(YorkieError::UnsupportedProtocolConversion(
                    "array element simple",
                ))
            }
            CrdtElement::Tree(_) => WireValueType::Tree,
        };

        Ok(Self {
            created_at: element.created_at().clone(),
            moved_at: element.moved_at().cloned(),
            removed_at: element.removed_at().cloned(),
            value_type,
            value: match element {
                CrdtElement::Primitive(value) => value.to_bytes(),
                CrdtElement::Counter(value) => value.to_bytes(),
                CrdtElement::Text(_) | CrdtElement::Tree(_) => Vec::new(),
                CrdtElement::Object(_) | CrdtElement::Array(_) => unreachable!(),
            },
        })
    }
}

fn primitive_value_type(value: &PrimitiveValue) -> WireValueType {
    match value {
        PrimitiveValue::Null => WireValueType::Null,
        PrimitiveValue::Boolean(_) => WireValueType::Boolean,
        PrimitiveValue::Integer(_) => WireValueType::Integer,
        PrimitiveValue::Long(_) => WireValueType::Long,
        PrimitiveValue::Double(_) => WireValueType::Double,
        PrimitiveValue::String(_) => WireValueType::String,
        PrimitiveValue::Bytes(_) => WireValueType::Bytes,
        PrimitiveValue::Date(_) => WireValueType::Date,
    }
}

fn counter_value_type(value_type: CounterType) -> WireValueType {
    match value_type {
        CounterType::Integer => WireValueType::IntegerCnt,
        CounterType::Long => WireValueType::LongCnt,
        CounterType::IntegerDedup => WireValueType::IntegerDedupCnt,
    }
}
