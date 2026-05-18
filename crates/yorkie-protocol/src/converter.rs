use crate::resources;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use yorkie_core::wire::{
    WireChange, WireChangeId, WireChangePack, WireJsonElementSimple, WireOperation, WireValueType,
};
use yorkie_core::{ChangePack as CoreChangePack, Checkpoint as CoreCheckpoint};
use yorkie_core::{TimeTicket as CoreTimeTicket, VersionVector as CoreVersionVector, YorkieError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    Core(YorkieError),
    InvalidActorId(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

impl Display for ProtocolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(err) => Display::fmt(err, f),
            Self::InvalidActorId(actor_id) => write!(f, "invalid actor id {actor_id:?}"),
        }
    }
}

impl Error for ProtocolError {}

impl From<YorkieError> for ProtocolError {
    fn from(value: YorkieError) -> Self {
        Self::Core(value)
    }
}

pub fn to_change_pack(pack: &CoreChangePack) -> Result<resources::ChangePack> {
    let pack = WireChangePack::try_from(pack)?;
    wire_change_pack_to_proto(&pack)
}

pub fn to_checkpoint(checkpoint: CoreCheckpoint) -> resources::Checkpoint {
    resources::Checkpoint {
        server_seq: checkpoint.server_seq(),
        client_seq: checkpoint.client_seq(),
    }
}

pub fn to_time_ticket(ticket: &CoreTimeTicket) -> Result<resources::TimeTicket> {
    Ok(resources::TimeTicket {
        lamport: ticket.lamport(),
        delimiter: ticket.delimiter(),
        actor_id: actor_id_to_bytes(ticket.actor_id())?,
    })
}

pub fn to_version_vector(vector: &CoreVersionVector) -> Result<resources::VersionVector> {
    let mut proto_vector = BTreeMap::new();
    for (actor_id, lamport) in vector.iter() {
        proto_vector.insert(base64_encode(&actor_id_to_bytes(actor_id)?), lamport);
    }

    Ok(resources::VersionVector {
        vector: proto_vector,
    })
}

fn wire_change_pack_to_proto(pack: &WireChangePack) -> Result<resources::ChangePack> {
    Ok(resources::ChangePack {
        document_key: pack.document_key.clone(),
        checkpoint: Some(to_checkpoint(pack.checkpoint)),
        snapshot: pack.snapshot.clone().unwrap_or_default(),
        changes: pack
            .changes
            .iter()
            .map(wire_change_to_proto)
            .collect::<Result<Vec<_>>>()?,
        is_removed: pack.is_removed,
        version_vector: pack
            .version_vector
            .as_ref()
            .map(to_version_vector)
            .transpose()?,
    })
}

fn wire_change_to_proto(change: &WireChange) -> Result<resources::Change> {
    Ok(resources::Change {
        id: Some(wire_change_id_to_proto(&change.id)?),
        message: change.message.clone().unwrap_or_default(),
        operations: change
            .operations
            .iter()
            .map(wire_operation_to_proto)
            .collect::<Result<Vec<_>>>()?,
    })
}

fn wire_change_id_to_proto(id: &WireChangeId) -> Result<resources::ChangeId> {
    Ok(resources::ChangeId {
        client_seq: id.client_seq,
        server_seq: id.server_seq,
        lamport: id.lamport,
        actor_id: actor_id_to_bytes(&id.actor_id)?,
        version_vector: Some(to_version_vector(&id.version_vector)?),
    })
}

fn wire_operation_to_proto(operation: &WireOperation) -> Result<resources::Operation> {
    let body = match operation {
        WireOperation::Set {
            parent_created_at,
            key,
            value,
            executed_at,
        } => resources::OperationBody::Set(resources::OperationSet {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            key: key.clone(),
            value: Some(wire_json_element_simple_to_proto(value)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::Add {
            parent_created_at,
            prev_created_at,
            value,
            executed_at,
        } => resources::OperationBody::Add(resources::OperationAdd {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            prev_created_at: Some(to_time_ticket(prev_created_at)?),
            value: Some(wire_json_element_simple_to_proto(value)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::Move {
            parent_created_at,
            prev_created_at,
            created_at,
            executed_at,
        } => resources::OperationBody::Move(resources::OperationMove {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            prev_created_at: Some(to_time_ticket(prev_created_at)?),
            created_at: Some(to_time_ticket(created_at)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::Remove {
            parent_created_at,
            created_at,
            executed_at,
        } => resources::OperationBody::Remove(resources::OperationRemove {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            created_at: Some(to_time_ticket(created_at)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::Increase {
            parent_created_at,
            value,
            executed_at,
            actor,
        } => resources::OperationBody::Increase(resources::OperationIncrease {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            value: Some(wire_json_element_simple_to_proto(value)?),
            executed_at: Some(to_time_ticket(executed_at)?),
            actor: actor.clone().unwrap_or_default(),
        }),
        WireOperation::ArraySet {
            parent_created_at,
            created_at,
            value,
            executed_at,
        } => resources::OperationBody::ArraySet(resources::OperationArraySet {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            created_at: Some(to_time_ticket(created_at)?),
            value: Some(wire_json_element_simple_to_proto(value)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
    };

    Ok(resources::Operation { body })
}

fn wire_json_element_simple_to_proto(
    value: &WireJsonElementSimple,
) -> Result<resources::JsonElementSimple> {
    Ok(resources::JsonElementSimple {
        created_at: Some(to_time_ticket(&value.created_at)?),
        moved_at: value.moved_at.as_ref().map(to_time_ticket).transpose()?,
        removed_at: value.removed_at.as_ref().map(to_time_ticket).transpose()?,
        value_type: wire_value_type_to_proto(value.value_type),
        value: value.value.clone(),
    })
}

fn wire_value_type_to_proto(value_type: WireValueType) -> resources::ValueType {
    match value_type {
        WireValueType::Null => resources::ValueType::Null,
        WireValueType::Boolean => resources::ValueType::Boolean,
        WireValueType::Integer => resources::ValueType::Integer,
        WireValueType::Long => resources::ValueType::Long,
        WireValueType::Double => resources::ValueType::Double,
        WireValueType::String => resources::ValueType::String,
        WireValueType::Bytes => resources::ValueType::Bytes,
        WireValueType::Date => resources::ValueType::Date,
        WireValueType::JsonObject => resources::ValueType::JsonObject,
        WireValueType::JsonArray => resources::ValueType::JsonArray,
        WireValueType::Text => resources::ValueType::Text,
        WireValueType::IntegerCnt => resources::ValueType::IntegerCnt,
        WireValueType::LongCnt => resources::ValueType::LongCnt,
        WireValueType::IntegerDedupCnt => resources::ValueType::IntegerDedupCnt,
        WireValueType::Tree => resources::ValueType::Tree,
    }
}

fn actor_id_to_bytes(actor_id: impl AsRef<str>) -> Result<Vec<u8>> {
    let actor_id = actor_id.as_ref();
    if actor_id.len() % 2 != 0 {
        return Err(ProtocolError::InvalidActorId(actor_id.to_owned()));
    }

    let mut bytes = Vec::with_capacity(actor_id.len() / 2);
    for index in (0..actor_id.len()).step_by(2) {
        let byte = u8::from_str_radix(&actor_id[index..index + 2], 16)
            .map_err(|_| ProtocolError::InvalidActorId(actor_id.to_owned()))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::{to_change_pack, to_time_ticket, to_version_vector};
    use crate::resources::{OperationBody, ValueType};
    use std::error::Error;
    use yorkie_core::{Document, TimeTicket};

    #[test]
    fn converts_time_tickets_to_proto_shape() -> Result<(), Box<dyn Error>> {
        let ticket = TimeTicket::new(1, 2, "000000000000000000000001");
        let proto = to_time_ticket(&ticket)?;

        assert_eq!(1, proto.lamport);
        assert_eq!(2, proto.delimiter);
        assert_eq!(vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], proto.actor_id);
        Ok(())
    }

    #[test]
    fn converts_version_vector_actor_keys_to_base64() -> Result<(), Box<dyn Error>> {
        let mut vector = yorkie_core::VersionVector::new();
        vector.set("000000000000000000000001", 3);

        let proto = to_version_vector(&vector)?;

        assert_eq!(Some(&3), proto.vector.get("AAAAAAAAAAAAAAAB"));
        Ok(())
    }

    #[test]
    fn converts_counter_change_pack_operations() -> Result<(), Box<dyn Error>> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_counter("count", 1)?.increase(2i32)?;
            Ok(())
        })?;

        let proto = to_change_pack(&doc.create_change_pack())?;

        assert_eq!("doc-key", proto.document_key);
        assert_eq!(1, proto.changes.len());
        assert_eq!(2, proto.changes[0].operations.len());

        let OperationBody::Set(set) = &proto.changes[0].operations[0].body else {
            panic!("expected set operation");
        };
        assert_eq!("count", set.key);
        let value = set.value.as_ref().expect("set value");
        assert_eq!(ValueType::IntegerCnt, value.value_type);
        assert_eq!(1i32.to_le_bytes().to_vec(), value.value);

        let OperationBody::Increase(increase) = &proto.changes[0].operations[1].body else {
            panic!("expected increase operation");
        };
        assert_eq!("", increase.actor);
        let value = increase.value.as_ref().expect("increase value");
        assert_eq!(ValueType::Integer, value.value_type);
        assert_eq!(2i32.to_le_bytes().to_vec(), value.value);
        Ok(())
    }

    #[test]
    fn converts_dedup_counter_change_pack_operations() -> Result<(), Box<dyn Error>> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_dedup_counter("uv")?.add("user-1")?;
            Ok(())
        })?;

        let proto = to_change_pack(&doc.create_change_pack())?;

        let OperationBody::Set(set) = &proto.changes[0].operations[0].body else {
            panic!("expected set operation");
        };
        assert_eq!(
            ValueType::IntegerDedupCnt,
            set.value.as_ref().expect("set value").value_type
        );

        let OperationBody::Increase(increase) = &proto.changes[0].operations[1].body else {
            panic!("expected increase operation");
        };
        assert_eq!("user-1", increase.actor);
        let value = increase.value.as_ref().expect("increase value");
        assert_eq!(ValueType::Integer, value.value_type);
        assert_eq!(1i32.to_le_bytes().to_vec(), value.value);
        Ok(())
    }
}
