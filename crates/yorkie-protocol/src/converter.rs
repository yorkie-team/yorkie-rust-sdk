use crate::yorkie::v1 as api;
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

pub fn to_change_pack(pack: &CoreChangePack) -> Result<api::ChangePack> {
    let pack = WireChangePack::try_from(pack)?;
    wire_change_pack_to_proto(&pack)
}

pub fn to_checkpoint(checkpoint: CoreCheckpoint) -> api::Checkpoint {
    api::Checkpoint {
        server_seq: checkpoint.server_seq(),
        client_seq: checkpoint.client_seq(),
    }
}

pub fn to_time_ticket(ticket: &CoreTimeTicket) -> Result<api::TimeTicket> {
    Ok(api::TimeTicket {
        lamport: ticket.lamport(),
        delimiter: ticket.delimiter(),
        actor_id: actor_id_to_bytes(ticket.actor_id())?,
    })
}

pub fn to_version_vector(vector: &CoreVersionVector) -> Result<api::VersionVector> {
    let mut proto_vector = BTreeMap::new();
    for (actor_id, lamport) in vector.iter() {
        proto_vector.insert(base64_encode(&actor_id_to_bytes(actor_id)?), lamport);
    }

    Ok(api::VersionVector {
        vector: proto_vector,
    })
}

fn wire_change_pack_to_proto(pack: &WireChangePack) -> Result<api::ChangePack> {
    Ok(api::ChangePack {
        document_key: pack.document_key.clone(),
        checkpoint: Some(to_checkpoint(pack.checkpoint)),
        snapshot: pack.snapshot.clone().unwrap_or_default(),
        changes: pack
            .changes
            .iter()
            .map(wire_change_to_proto)
            .collect::<Result<Vec<_>>>()?,
        min_synced_ticket: None,
        is_removed: pack.is_removed,
        version_vector: pack
            .version_vector
            .as_ref()
            .map(to_version_vector)
            .transpose()?,
    })
}

fn wire_change_to_proto(change: &WireChange) -> Result<api::Change> {
    Ok(api::Change {
        id: Some(wire_change_id_to_proto(&change.id)?),
        message: change.message.clone().unwrap_or_default(),
        operations: change
            .operations
            .iter()
            .map(wire_operation_to_proto)
            .collect::<Result<Vec<_>>>()?,
        presence_change: None,
    })
}

fn wire_change_id_to_proto(id: &WireChangeId) -> Result<api::ChangeId> {
    Ok(api::ChangeId {
        client_seq: id.client_seq,
        server_seq: id.server_seq,
        lamport: id.lamport,
        actor_id: actor_id_to_bytes(&id.actor_id)?,
        version_vector: Some(to_version_vector(&id.version_vector)?),
    })
}

fn wire_operation_to_proto(operation: &WireOperation) -> Result<api::Operation> {
    let body = match operation {
        WireOperation::Set {
            parent_created_at,
            key,
            value,
            executed_at,
        } => api::operation::Body::Set(api::operation::Set {
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
        } => api::operation::Body::Add(api::operation::Add {
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
        } => api::operation::Body::Move(api::operation::Move {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            prev_created_at: Some(to_time_ticket(prev_created_at)?),
            created_at: Some(to_time_ticket(created_at)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::Remove {
            parent_created_at,
            created_at,
            executed_at,
        } => api::operation::Body::Remove(api::operation::Remove {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            created_at: Some(to_time_ticket(created_at)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::Increase {
            parent_created_at,
            value,
            executed_at,
            actor,
        } => api::operation::Body::Increase(api::operation::Increase {
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
        } => api::operation::Body::ArraySet(api::operation::ArraySet {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            created_at: Some(to_time_ticket(created_at)?),
            value: Some(wire_json_element_simple_to_proto(value)?),
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
    };

    Ok(api::Operation { body: Some(body) })
}

fn wire_json_element_simple_to_proto(
    value: &WireJsonElementSimple,
) -> Result<api::JsonElementSimple> {
    Ok(api::JsonElementSimple {
        created_at: Some(to_time_ticket(&value.created_at)?),
        moved_at: value.moved_at.as_ref().map(to_time_ticket).transpose()?,
        removed_at: value.removed_at.as_ref().map(to_time_ticket).transpose()?,
        r#type: wire_value_type_to_proto(value.value_type) as i32,
        value: value.value.clone(),
    })
}

fn wire_value_type_to_proto(value_type: WireValueType) -> api::ValueType {
    match value_type {
        WireValueType::Null => api::ValueType::Null,
        WireValueType::Boolean => api::ValueType::Boolean,
        WireValueType::Integer => api::ValueType::Integer,
        WireValueType::Long => api::ValueType::Long,
        WireValueType::Double => api::ValueType::Double,
        WireValueType::String => api::ValueType::String,
        WireValueType::Bytes => api::ValueType::Bytes,
        WireValueType::Date => api::ValueType::Date,
        WireValueType::JsonObject => api::ValueType::JsonObject,
        WireValueType::JsonArray => api::ValueType::JsonArray,
        WireValueType::Text => api::ValueType::Text,
        WireValueType::IntegerCnt => api::ValueType::IntegerCnt,
        WireValueType::LongCnt => api::ValueType::LongCnt,
        WireValueType::IntegerDedupCnt => api::ValueType::IntegerDedupCnt,
        WireValueType::Tree => api::ValueType::Tree,
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
    use crate::yorkie::v1::{operation::Body, ValueType};
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

        let Some(Body::Set(set)) = &proto.changes[0].operations[0].body else {
            panic!("expected set operation");
        };
        assert_eq!("count", set.key);
        let value = set.value.as_ref().expect("set value");
        assert_eq!(ValueType::IntegerCnt as i32, value.r#type);
        assert_eq!(1i32.to_le_bytes().to_vec(), value.value);

        let Some(Body::Increase(increase)) = &proto.changes[0].operations[1].body else {
            panic!("expected increase operation");
        };
        assert_eq!("", increase.actor);
        let value = increase.value.as_ref().expect("increase value");
        assert_eq!(ValueType::Integer as i32, value.r#type);
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

        let Some(Body::Set(set)) = &proto.changes[0].operations[0].body else {
            panic!("expected set operation");
        };
        assert_eq!(
            ValueType::IntegerDedupCnt as i32,
            set.value.as_ref().expect("set value").r#type
        );

        let Some(Body::Increase(increase)) = &proto.changes[0].operations[1].body else {
            panic!("expected increase operation");
        };
        assert_eq!("user-1", increase.actor);
        let value = increase.value.as_ref().expect("increase value");
        assert_eq!(ValueType::Integer as i32, value.r#type);
        assert_eq!(1i32.to_le_bytes().to_vec(), value.value);
        Ok(())
    }
}
