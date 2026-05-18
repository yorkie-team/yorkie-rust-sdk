use crate::yorkie::v1 as api;
use prost::Message;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use yorkie_core::wire::{
    WireChange, WireChangeId, WireChangePack, WireJsonElement, WireJsonElementSimple, WireNodeAttr,
    WireOperation, WireRgaNode, WireRhtNode, WireTextNode, WireTextNodeId, WireTextNodePos,
    WireTreeNode, WireTreeNodeId, WireTreePos, WireValueType,
};
use yorkie_core::{ChangePack as CoreChangePack, Checkpoint as CoreCheckpoint};
use yorkie_core::{SchemaRule as CoreSchemaRule, TreeNodeRule as CoreTreeNodeRule};
use yorkie_core::{TimeTicket as CoreTimeTicket, VersionVector as CoreVersionVector, YorkieError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    Core(YorkieError),
    Decode(String),
    InvalidActorId(String),
    InvalidBase64(String),
    InvalidInteger { field: &'static str, value: i32 },
    InvalidOffset { field: &'static str, value: i32 },
    MissingField(&'static str),
    UnsupportedValueType(i32),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

impl Display for ProtocolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(err) => Display::fmt(err, f),
            Self::Decode(err) => write!(f, "protobuf decode failed: {err}"),
            Self::InvalidActorId(actor_id) => write!(f, "invalid actor id {actor_id:?}"),
            Self::InvalidBase64(value) => write!(f, "invalid base64 value {value:?}"),
            Self::InvalidInteger { field, value } => {
                write!(f, "invalid integer for {field}: {value}")
            }
            Self::InvalidOffset { field, value } => {
                write!(f, "invalid negative offset for {field}: {value}")
            }
            Self::MissingField(field) => write!(f, "missing protobuf field {field}"),
            Self::UnsupportedValueType(value_type) => {
                write!(f, "unsupported protobuf value type {value_type}")
            }
        }
    }
}

impl Error for ProtocolError {}

impl From<YorkieError> for ProtocolError {
    fn from(value: YorkieError) -> Self {
        Self::Core(value)
    }
}

impl From<prost::DecodeError> for ProtocolError {
    fn from(value: prost::DecodeError) -> Self {
        Self::Decode(value.to_string())
    }
}

pub fn to_change_pack(pack: &CoreChangePack) -> Result<api::ChangePack> {
    let pack = WireChangePack::try_from(pack)?;
    wire_change_pack_to_proto(&pack)
}

pub fn from_change_pack(pack: &api::ChangePack) -> Result<CoreChangePack> {
    let pack = proto_change_pack_to_wire(pack)?;
    Ok(CoreChangePack::try_from(pack)?)
}

pub fn encode_change_pack(pack: &CoreChangePack) -> Result<Vec<u8>> {
    Ok(to_change_pack(pack)?.encode_to_vec())
}

pub fn decode_change_pack(bytes: &[u8]) -> Result<CoreChangePack> {
    let pack = api::ChangePack::decode(bytes)?;
    from_change_pack(&pack)
}

pub fn to_checkpoint(checkpoint: CoreCheckpoint) -> api::Checkpoint {
    api::Checkpoint {
        server_seq: checkpoint.server_seq(),
        client_seq: checkpoint.client_seq(),
    }
}

pub fn from_checkpoint(checkpoint: api::Checkpoint) -> CoreCheckpoint {
    CoreCheckpoint::new(checkpoint.server_seq, checkpoint.client_seq)
}

pub fn to_time_ticket(ticket: &CoreTimeTicket) -> Result<api::TimeTicket> {
    Ok(api::TimeTicket {
        lamport: ticket.lamport(),
        delimiter: ticket.delimiter(),
        actor_id: actor_id_to_bytes(ticket.actor_id())?,
    })
}

pub fn from_time_ticket(ticket: &api::TimeTicket) -> CoreTimeTicket {
    CoreTimeTicket::new(
        ticket.lamport,
        ticket.delimiter,
        bytes_to_actor_id(&ticket.actor_id),
    )
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

pub fn from_version_vector(vector: &api::VersionVector) -> Result<CoreVersionVector> {
    let mut core_vector = CoreVersionVector::new();
    for (actor_id, lamport) in &vector.vector {
        let actor_id = bytes_to_actor_id(&base64_decode(actor_id)?);
        core_vector.set(actor_id, *lamport);
    }
    Ok(core_vector)
}

pub fn to_schema_rules(rules: &[CoreSchemaRule]) -> Vec<api::Rule> {
    rules
        .iter()
        .map(|rule| api::Rule {
            path: rule.path.clone(),
            r#type: rule.rule_type.clone(),
            tree_nodes: rule
                .tree_nodes
                .iter()
                .map(|node| api::TreeNodeRule {
                    node_type: node.node_type.clone(),
                    content: node.content.clone(),
                    marks: node.marks.clone(),
                    group: node.group.clone(),
                })
                .collect(),
        })
        .collect()
}

pub fn from_schema_rules(rules: &[api::Rule]) -> Vec<CoreSchemaRule> {
    rules
        .iter()
        .map(|rule| {
            CoreSchemaRule::new(
                rule.path.clone(),
                rule.r#type.clone(),
                rule.tree_nodes
                    .iter()
                    .map(|node| {
                        CoreTreeNodeRule::new(
                            node.node_type.clone(),
                            node.content.clone(),
                            node.marks.clone(),
                            node.group.clone(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

fn wire_change_pack_to_proto(pack: &WireChangePack) -> Result<api::ChangePack> {
    let snapshot = if let Some(snapshot) = &pack.snapshot {
        snapshot.clone()
    } else if let Some(snapshot_root) = &pack.snapshot_root {
        snapshot_root_to_bytes(snapshot_root)?
    } else {
        Vec::new()
    };

    Ok(api::ChangePack {
        document_key: pack.document_key.clone(),
        checkpoint: Some(to_checkpoint(pack.checkpoint)),
        snapshot,
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

fn proto_change_pack_to_wire(pack: &api::ChangePack) -> Result<WireChangePack> {
    let snapshot_root = if pack.snapshot.is_empty() {
        None
    } else {
        Some(snapshot_root_from_bytes(&pack.snapshot)?)
    };

    Ok(WireChangePack {
        document_key: pack.document_key.clone(),
        checkpoint: from_checkpoint(*required(&pack.checkpoint, "change_pack.checkpoint")?),
        is_removed: pack.is_removed,
        changes: pack
            .changes
            .iter()
            .map(proto_change_to_wire)
            .collect::<Result<Vec<_>>>()?,
        snapshot: (!pack.snapshot.is_empty()).then(|| pack.snapshot.clone()),
        snapshot_root,
        version_vector: pack
            .version_vector
            .as_ref()
            .map(from_version_vector)
            .transpose()?,
    })
}

fn snapshot_root_to_bytes(snapshot_root: &WireJsonElement) -> Result<Vec<u8>> {
    Ok(api::Snapshot {
        root: Some(wire_json_element_to_proto(snapshot_root)?),
        presences: BTreeMap::new(),
    }
    .encode_to_vec())
}

fn snapshot_root_from_bytes(snapshot: &[u8]) -> Result<WireJsonElement> {
    let snapshot = api::Snapshot::decode(snapshot)?;
    proto_json_element_to_wire(required(&snapshot.root, "snapshot.root")?)
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

fn proto_change_to_wire(change: &api::Change) -> Result<WireChange> {
    Ok(WireChange {
        id: proto_change_id_to_wire(required(&change.id, "change.id")?)?,
        message: (!change.message.is_empty()).then(|| change.message.clone()),
        operations: change
            .operations
            .iter()
            .map(proto_operation_to_wire)
            .collect::<Result<Vec<_>>>()?,
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

fn proto_change_id_to_wire(id: &api::ChangeId) -> Result<WireChangeId> {
    Ok(WireChangeId {
        client_seq: id.client_seq,
        server_seq: id.server_seq,
        lamport: id.lamport,
        actor_id: bytes_to_actor_id(&id.actor_id),
        version_vector: id
            .version_vector
            .as_ref()
            .map(from_version_vector)
            .transpose()?
            .unwrap_or_else(CoreVersionVector::new),
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
        WireOperation::Edit {
            parent_created_at,
            from,
            to,
            content,
            attributes,
            executed_at,
        } => api::operation::Body::Edit(api::operation::Edit {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            from: Some(wire_text_node_pos_to_proto(from)?),
            to: Some(wire_text_node_pos_to_proto(to)?),
            created_at_map_by_actor: BTreeMap::new(),
            content: content.clone(),
            executed_at: Some(to_time_ticket(executed_at)?),
            attributes: attributes.clone(),
        }),
        WireOperation::Style {
            parent_created_at,
            from,
            to,
            attributes,
            attributes_to_remove,
            executed_at,
        } => api::operation::Body::Style(api::operation::Style {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            from: Some(wire_text_node_pos_to_proto(from)?),
            to: Some(wire_text_node_pos_to_proto(to)?),
            attributes: attributes.clone(),
            executed_at: Some(to_time_ticket(executed_at)?),
            created_at_map_by_actor: BTreeMap::new(),
            attributes_to_remove: attributes_to_remove.clone(),
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
        WireOperation::TreeEdit {
            parent_created_at,
            from,
            to,
            contents,
            split_level,
            executed_at,
        } => api::operation::Body::TreeEdit(api::operation::TreeEdit {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            from: Some(wire_tree_pos_to_proto(from)?),
            to: Some(wire_tree_pos_to_proto(to)?),
            created_at_map_by_actor: BTreeMap::new(),
            contents: contents
                .as_ref()
                .map(|groups| {
                    groups
                        .iter()
                        .map(wire_tree_nodes_to_proto)
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default(),
            split_level: i32_from_usize(*split_level, "tree_edit.split_level")?,
            executed_at: Some(to_time_ticket(executed_at)?),
        }),
        WireOperation::TreeStyle {
            parent_created_at,
            from,
            to,
            attributes,
            attributes_to_remove,
            executed_at,
        } => api::operation::Body::TreeStyle(api::operation::TreeStyle {
            parent_created_at: Some(to_time_ticket(parent_created_at)?),
            from: Some(wire_tree_pos_to_proto(from)?),
            to: Some(wire_tree_pos_to_proto(to)?),
            attributes: if attributes_to_remove.is_empty() {
                attributes.clone()
            } else {
                BTreeMap::new()
            },
            executed_at: Some(to_time_ticket(executed_at)?),
            attributes_to_remove: attributes_to_remove.clone(),
            created_at_map_by_actor: BTreeMap::new(),
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

fn proto_operation_to_wire(operation: &api::Operation) -> Result<WireOperation> {
    let body = required(&operation.body, "operation.body")?;
    match body {
        api::operation::Body::Set(operation) => Ok(WireOperation::Set {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.set.parent_created_at",
            )?,
            key: operation.key.clone(),
            value: proto_json_element_simple_to_wire(required(
                &operation.value,
                "operation.set.value",
            )?)?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.set.executed_at",
            )?,
        }),
        api::operation::Body::Add(operation) => Ok(WireOperation::Add {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.add.parent_created_at",
            )?,
            prev_created_at: proto_required_time_ticket(
                &operation.prev_created_at,
                "operation.add.prev_created_at",
            )?,
            value: proto_json_element_simple_to_wire(required(
                &operation.value,
                "operation.add.value",
            )?)?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.add.executed_at",
            )?,
        }),
        api::operation::Body::Move(operation) => Ok(WireOperation::Move {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.move.parent_created_at",
            )?,
            prev_created_at: proto_required_time_ticket(
                &operation.prev_created_at,
                "operation.move.prev_created_at",
            )?,
            created_at: proto_required_time_ticket(
                &operation.created_at,
                "operation.move.created_at",
            )?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.move.executed_at",
            )?,
        }),
        api::operation::Body::Remove(operation) => Ok(WireOperation::Remove {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.remove.parent_created_at",
            )?,
            created_at: proto_required_time_ticket(
                &operation.created_at,
                "operation.remove.created_at",
            )?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.remove.executed_at",
            )?,
        }),
        api::operation::Body::Edit(operation) => Ok(WireOperation::Edit {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.edit.parent_created_at",
            )?,
            from: proto_text_node_pos_to_wire(required(&operation.from, "operation.edit.from")?)?,
            to: proto_text_node_pos_to_wire(required(&operation.to, "operation.edit.to")?)?,
            content: operation.content.clone(),
            attributes: operation.attributes.clone(),
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.edit.executed_at",
            )?,
        }),
        api::operation::Body::Style(operation) => Ok(WireOperation::Style {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.style.parent_created_at",
            )?,
            from: proto_text_node_pos_to_wire(required(&operation.from, "operation.style.from")?)?,
            to: proto_text_node_pos_to_wire(required(&operation.to, "operation.style.to")?)?,
            attributes: operation.attributes.clone(),
            attributes_to_remove: operation.attributes_to_remove.clone(),
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.style.executed_at",
            )?,
        }),
        api::operation::Body::Increase(operation) => Ok(WireOperation::Increase {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.increase.parent_created_at",
            )?,
            value: proto_json_element_simple_to_wire(required(
                &operation.value,
                "operation.increase.value",
            )?)?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.increase.executed_at",
            )?,
            actor: (!operation.actor.is_empty()).then(|| operation.actor.clone()),
        }),
        api::operation::Body::TreeEdit(operation) => Ok(WireOperation::TreeEdit {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.tree_edit.parent_created_at",
            )?,
            from: proto_tree_pos_to_wire(required(&operation.from, "operation.tree_edit.from")?)?,
            to: proto_tree_pos_to_wire(required(&operation.to, "operation.tree_edit.to")?)?,
            contents: (!operation.contents.is_empty())
                .then(|| {
                    operation
                        .contents
                        .iter()
                        .map(proto_tree_nodes_to_wire)
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?,
            split_level: usize_from_i32(operation.split_level, "operation.tree_edit.split_level")?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.tree_edit.executed_at",
            )?,
        }),
        api::operation::Body::TreeStyle(operation) => Ok(WireOperation::TreeStyle {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.tree_style.parent_created_at",
            )?,
            from: proto_tree_pos_to_wire(required(&operation.from, "operation.tree_style.from")?)?,
            to: proto_tree_pos_to_wire(required(&operation.to, "operation.tree_style.to")?)?,
            attributes: if operation.attributes_to_remove.is_empty() {
                operation.attributes.clone()
            } else {
                BTreeMap::new()
            },
            attributes_to_remove: operation.attributes_to_remove.clone(),
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.tree_style.executed_at",
            )?,
        }),
        api::operation::Body::ArraySet(operation) => Ok(WireOperation::ArraySet {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.array_set.parent_created_at",
            )?,
            created_at: proto_required_time_ticket(
                &operation.created_at,
                "operation.array_set.created_at",
            )?,
            value: proto_json_element_simple_to_wire(required(
                &operation.value,
                "operation.array_set.value",
            )?)?,
            executed_at: proto_required_time_ticket(
                &operation.executed_at,
                "operation.array_set.executed_at",
            )?,
        }),
    }
}

fn wire_json_element_simple_to_proto(
    value: &WireJsonElementSimple,
) -> Result<api::JsonElementSimple> {
    let encoded_value = if let Some(element) = &value.element {
        wire_json_element_to_proto(element)?.encode_to_vec()
    } else {
        value.value.clone()
    };

    Ok(api::JsonElementSimple {
        created_at: Some(to_time_ticket(&value.created_at)?),
        moved_at: value.moved_at.as_ref().map(to_time_ticket).transpose()?,
        removed_at: value.removed_at.as_ref().map(to_time_ticket).transpose()?,
        r#type: wire_value_type_to_proto(value.value_type) as i32,
        value: encoded_value,
    })
}

fn proto_json_element_simple_to_wire(
    value: &api::JsonElementSimple,
) -> Result<WireJsonElementSimple> {
    let value_type = proto_value_type_to_wire(value.r#type)?;
    let element = if matches!(
        value_type,
        WireValueType::JsonObject | WireValueType::JsonArray | WireValueType::Tree
    ) && !value.value.is_empty()
    {
        let element = api::JsonElement::decode(value.value.as_slice())?;
        Some(Box::new(proto_json_element_to_wire(&element)?))
    } else {
        None
    };

    Ok(WireJsonElementSimple {
        created_at: proto_required_time_ticket(
            &value.created_at,
            "json_element_simple.created_at",
        )?,
        moved_at: proto_optional_time_ticket(&value.moved_at),
        removed_at: proto_optional_time_ticket(&value.removed_at),
        value_type,
        value: value.value.clone(),
        element,
    })
}

fn wire_json_element_to_proto(element: &WireJsonElement) -> Result<api::JsonElement> {
    let body = match element {
        WireJsonElement::Object {
            nodes,
            created_at,
            moved_at,
            removed_at,
        } => api::json_element::Body::JsonObject(api::json_element::JsonObject {
            nodes: nodes
                .iter()
                .map(wire_rht_node_to_proto)
                .collect::<Result<Vec<_>>>()?,
            created_at: Some(to_time_ticket(created_at)?),
            moved_at: moved_at.as_ref().map(to_time_ticket).transpose()?,
            removed_at: removed_at.as_ref().map(to_time_ticket).transpose()?,
        }),
        WireJsonElement::Array {
            nodes,
            created_at,
            moved_at,
            removed_at,
        } => api::json_element::Body::JsonArray(api::json_element::JsonArray {
            nodes: nodes
                .iter()
                .map(wire_rga_node_to_proto)
                .collect::<Result<Vec<_>>>()?,
            created_at: Some(to_time_ticket(created_at)?),
            moved_at: moved_at.as_ref().map(to_time_ticket).transpose()?,
            removed_at: removed_at.as_ref().map(to_time_ticket).transpose()?,
        }),
        WireJsonElement::Primitive {
            value_type,
            value,
            created_at,
            moved_at,
            removed_at,
        } => api::json_element::Body::Primitive(api::json_element::Primitive {
            r#type: wire_value_type_to_proto(*value_type) as i32,
            value: value.clone(),
            created_at: Some(to_time_ticket(created_at)?),
            moved_at: moved_at.as_ref().map(to_time_ticket).transpose()?,
            removed_at: removed_at.as_ref().map(to_time_ticket).transpose()?,
        }),
        WireJsonElement::Text {
            nodes,
            created_at,
            moved_at,
            removed_at,
        } => api::json_element::Body::Text(api::json_element::Text {
            nodes: nodes
                .iter()
                .map(wire_text_node_to_proto)
                .collect::<Result<Vec<_>>>()?,
            created_at: Some(to_time_ticket(created_at)?),
            moved_at: moved_at.as_ref().map(to_time_ticket).transpose()?,
            removed_at: removed_at.as_ref().map(to_time_ticket).transpose()?,
        }),
        WireJsonElement::Counter {
            value_type,
            value,
            hll_registers,
            created_at,
            moved_at,
            removed_at,
        } => api::json_element::Body::Counter(api::json_element::Counter {
            r#type: wire_value_type_to_proto(*value_type) as i32,
            value: value.clone(),
            created_at: Some(to_time_ticket(created_at)?),
            moved_at: moved_at.as_ref().map(to_time_ticket).transpose()?,
            removed_at: removed_at.as_ref().map(to_time_ticket).transpose()?,
            hll_registers: hll_registers.clone(),
        }),
        WireJsonElement::Tree {
            nodes,
            created_at,
            moved_at,
            removed_at,
        } => api::json_element::Body::Tree(api::json_element::Tree {
            nodes: nodes
                .iter()
                .map(wire_tree_node_to_proto)
                .collect::<Result<Vec<_>>>()?,
            created_at: Some(to_time_ticket(created_at)?),
            moved_at: moved_at.as_ref().map(to_time_ticket).transpose()?,
            removed_at: removed_at.as_ref().map(to_time_ticket).transpose()?,
        }),
    };

    Ok(api::JsonElement { body: Some(body) })
}

fn proto_json_element_to_wire(element: &api::JsonElement) -> Result<WireJsonElement> {
    let body = required(&element.body, "json_element.body")?;
    match body {
        api::json_element::Body::JsonObject(object) => Ok(WireJsonElement::Object {
            nodes: object
                .nodes
                .iter()
                .map(proto_rht_node_to_wire)
                .collect::<Result<Vec<_>>>()?,
            created_at: proto_required_time_ticket(&object.created_at, "json_object.created_at")?,
            moved_at: proto_optional_time_ticket(&object.moved_at),
            removed_at: proto_optional_time_ticket(&object.removed_at),
        }),
        api::json_element::Body::JsonArray(array) => Ok(WireJsonElement::Array {
            nodes: array
                .nodes
                .iter()
                .map(proto_rga_node_to_wire)
                .collect::<Result<Vec<_>>>()?,
            created_at: proto_required_time_ticket(&array.created_at, "json_array.created_at")?,
            moved_at: proto_optional_time_ticket(&array.moved_at),
            removed_at: proto_optional_time_ticket(&array.removed_at),
        }),
        api::json_element::Body::Primitive(primitive) => Ok(WireJsonElement::Primitive {
            value_type: proto_value_type_to_wire(primitive.r#type)?,
            value: primitive.value.clone(),
            created_at: proto_required_time_ticket(&primitive.created_at, "primitive.created_at")?,
            moved_at: proto_optional_time_ticket(&primitive.moved_at),
            removed_at: proto_optional_time_ticket(&primitive.removed_at),
        }),
        api::json_element::Body::Text(text) => Ok(WireJsonElement::Text {
            nodes: text
                .nodes
                .iter()
                .map(proto_text_node_to_wire)
                .collect::<Result<Vec<_>>>()?,
            created_at: proto_required_time_ticket(&text.created_at, "text.created_at")?,
            moved_at: proto_optional_time_ticket(&text.moved_at),
            removed_at: proto_optional_time_ticket(&text.removed_at),
        }),
        api::json_element::Body::Counter(counter) => Ok(WireJsonElement::Counter {
            value_type: proto_value_type_to_wire(counter.r#type)?,
            value: counter.value.clone(),
            hll_registers: counter.hll_registers.clone(),
            created_at: proto_required_time_ticket(&counter.created_at, "counter.created_at")?,
            moved_at: proto_optional_time_ticket(&counter.moved_at),
            removed_at: proto_optional_time_ticket(&counter.removed_at),
        }),
        api::json_element::Body::Tree(tree) => Ok(WireJsonElement::Tree {
            nodes: tree
                .nodes
                .iter()
                .map(proto_tree_node_to_wire)
                .collect::<Result<Vec<_>>>()?,
            created_at: proto_required_time_ticket(&tree.created_at, "tree.created_at")?,
            moved_at: proto_optional_time_ticket(&tree.moved_at),
            removed_at: proto_optional_time_ticket(&tree.removed_at),
        }),
    }
}

fn wire_rht_node_to_proto(node: &WireRhtNode) -> Result<api::RhtNode> {
    Ok(api::RhtNode {
        key: node.key.clone(),
        element: Some(wire_json_element_to_proto(&node.element)?),
    })
}

fn proto_rht_node_to_wire(node: &api::RhtNode) -> Result<WireRhtNode> {
    Ok(WireRhtNode {
        key: node.key.clone(),
        element: proto_json_element_to_wire(required(&node.element, "rht_node.element")?)?,
    })
}

fn wire_rga_node_to_proto(node: &WireRgaNode) -> Result<api::RgaNode> {
    Ok(api::RgaNode {
        next: None,
        element: node
            .element
            .as_ref()
            .map(wire_json_element_to_proto)
            .transpose()?,
        position_created_at: node
            .position_created_at
            .as_ref()
            .map(to_time_ticket)
            .transpose()?,
        position_moved_at: node
            .position_moved_at
            .as_ref()
            .map(to_time_ticket)
            .transpose()?,
        position_removed_at: node
            .position_removed_at
            .as_ref()
            .map(to_time_ticket)
            .transpose()?,
    })
}

fn proto_rga_node_to_wire(node: &api::RgaNode) -> Result<WireRgaNode> {
    Ok(WireRgaNode {
        element: node
            .element
            .as_ref()
            .map(proto_json_element_to_wire)
            .transpose()?,
        position_created_at: proto_optional_time_ticket(&node.position_created_at),
        position_moved_at: proto_optional_time_ticket(&node.position_moved_at),
        position_removed_at: proto_optional_time_ticket(&node.position_removed_at),
    })
}

fn wire_text_node_to_proto(node: &WireTextNode) -> Result<api::TextNode> {
    Ok(api::TextNode {
        id: Some(wire_text_node_id_to_proto(&node.id)?),
        value: node.value.clone(),
        removed_at: node.removed_at.as_ref().map(to_time_ticket).transpose()?,
        ins_prev_id: node
            .ins_prev_id
            .as_ref()
            .map(wire_text_node_id_to_proto)
            .transpose()?,
        attributes: wire_attrs_to_proto(&node.attributes)?,
    })
}

fn proto_text_node_to_wire(node: &api::TextNode) -> Result<WireTextNode> {
    Ok(WireTextNode {
        id: proto_text_node_id_to_wire(required(&node.id, "text_node.id")?)?,
        value: node.value.clone(),
        removed_at: proto_optional_time_ticket(&node.removed_at),
        ins_prev_id: node
            .ins_prev_id
            .as_ref()
            .map(proto_text_node_id_to_wire)
            .transpose()?,
        attributes: proto_attrs_to_wire(&node.attributes)?,
    })
}

fn wire_tree_nodes_to_proto(nodes: &Vec<WireTreeNode>) -> Result<api::TreeNodes> {
    Ok(api::TreeNodes {
        content: nodes
            .iter()
            .map(wire_tree_node_to_proto)
            .collect::<Result<Vec<_>>>()?,
    })
}

fn proto_tree_nodes_to_wire(nodes: &api::TreeNodes) -> Result<Vec<WireTreeNode>> {
    nodes
        .content
        .iter()
        .map(proto_tree_node_to_wire)
        .collect::<Result<Vec<_>>>()
}

fn wire_tree_node_to_proto(node: &WireTreeNode) -> Result<api::TreeNode> {
    Ok(api::TreeNode {
        id: Some(wire_tree_node_id_to_proto(&node.id)?),
        r#type: node.node_type.clone(),
        value: node.value.clone(),
        removed_at: node.removed_at.as_ref().map(to_time_ticket).transpose()?,
        ins_prev_id: node
            .ins_prev_id
            .as_ref()
            .map(wire_tree_node_id_to_proto)
            .transpose()?,
        ins_next_id: node
            .ins_next_id
            .as_ref()
            .map(wire_tree_node_id_to_proto)
            .transpose()?,
        depth: node.depth,
        attributes: wire_attrs_to_proto(&node.attributes)?,
        merged_from: node
            .merged_from
            .as_ref()
            .map(wire_tree_node_id_to_proto)
            .transpose()?,
        merged_at: node.merged_at.as_ref().map(to_time_ticket).transpose()?,
    })
}

fn proto_tree_node_to_wire(node: &api::TreeNode) -> Result<WireTreeNode> {
    Ok(WireTreeNode {
        id: proto_tree_node_id_to_wire(required(&node.id, "tree_node.id")?)?,
        node_type: node.r#type.clone(),
        value: node.value.clone(),
        removed_at: proto_optional_time_ticket(&node.removed_at),
        ins_prev_id: node
            .ins_prev_id
            .as_ref()
            .map(proto_tree_node_id_to_wire)
            .transpose()?,
        ins_next_id: node
            .ins_next_id
            .as_ref()
            .map(proto_tree_node_id_to_wire)
            .transpose()?,
        depth: node.depth,
        attributes: proto_attrs_to_wire(&node.attributes)?,
        merged_from: node
            .merged_from
            .as_ref()
            .map(proto_tree_node_id_to_wire)
            .transpose()?,
        merged_at: proto_optional_time_ticket(&node.merged_at),
    })
}

fn wire_attrs_to_proto(
    attrs: &BTreeMap<String, WireNodeAttr>,
) -> Result<BTreeMap<String, api::NodeAttr>> {
    attrs
        .iter()
        .map(|(key, attr)| {
            Ok((
                key.clone(),
                api::NodeAttr {
                    value: attr.value.clone(),
                    updated_at: Some(to_time_ticket(&attr.updated_at)?),
                    is_removed: attr.is_removed,
                },
            ))
        })
        .collect()
}

fn proto_attrs_to_wire(
    attrs: &BTreeMap<String, api::NodeAttr>,
) -> Result<BTreeMap<String, WireNodeAttr>> {
    attrs
        .iter()
        .map(|(key, attr)| {
            Ok((
                key.clone(),
                WireNodeAttr {
                    value: attr.value.clone(),
                    updated_at: proto_required_time_ticket(
                        &attr.updated_at,
                        "node_attr.updated_at",
                    )?,
                    is_removed: attr.is_removed,
                },
            ))
        })
        .collect()
}

fn wire_text_node_id_to_proto(id: &WireTextNodeId) -> Result<api::TextNodeId> {
    Ok(api::TextNodeId {
        created_at: Some(to_time_ticket(&id.created_at)?),
        offset: i32_from_usize(id.offset, "text_node_id.offset")?,
    })
}

fn proto_text_node_id_to_wire(id: &api::TextNodeId) -> Result<WireTextNodeId> {
    Ok(WireTextNodeId {
        created_at: proto_required_time_ticket(&id.created_at, "text_node_id.created_at")?,
        offset: usize_from_i32(id.offset, "text_node_id.offset")?,
    })
}

fn wire_text_node_pos_to_proto(pos: &WireTextNodePos) -> Result<api::TextNodePos> {
    Ok(api::TextNodePos {
        created_at: Some(to_time_ticket(&pos.id.created_at)?),
        offset: i32_from_usize(pos.id.offset, "text_node_pos.offset")?,
        relative_offset: i32_from_usize(pos.relative_offset, "text_node_pos.relative_offset")?,
    })
}

fn proto_text_node_pos_to_wire(pos: &api::TextNodePos) -> Result<WireTextNodePos> {
    Ok(WireTextNodePos {
        id: WireTextNodeId {
            created_at: proto_required_time_ticket(&pos.created_at, "text_node_pos.created_at")?,
            offset: usize_from_i32(pos.offset, "text_node_pos.offset")?,
        },
        relative_offset: usize_from_i32(pos.relative_offset, "text_node_pos.relative_offset")?,
    })
}

fn wire_tree_node_id_to_proto(id: &WireTreeNodeId) -> Result<api::TreeNodeId> {
    Ok(api::TreeNodeId {
        created_at: Some(to_time_ticket(&id.created_at)?),
        offset: i32_from_usize(id.offset, "tree_node_id.offset")?,
    })
}

fn proto_tree_node_id_to_wire(id: &api::TreeNodeId) -> Result<WireTreeNodeId> {
    Ok(WireTreeNodeId {
        created_at: proto_required_time_ticket(&id.created_at, "tree_node_id.created_at")?,
        offset: usize_from_i32(id.offset, "tree_node_id.offset")?,
    })
}

fn wire_tree_pos_to_proto(pos: &WireTreePos) -> Result<api::TreePos> {
    Ok(api::TreePos {
        parent_id: Some(wire_tree_node_id_to_proto(&pos.parent_id)?),
        left_sibling_id: Some(wire_tree_node_id_to_proto(&pos.left_sibling_id)?),
    })
}

fn proto_tree_pos_to_wire(pos: &api::TreePos) -> Result<WireTreePos> {
    Ok(WireTreePos {
        parent_id: proto_tree_node_id_to_wire(required(&pos.parent_id, "tree_pos.parent_id")?)?,
        left_sibling_id: proto_tree_node_id_to_wire(required(
            &pos.left_sibling_id,
            "tree_pos.left_sibling_id",
        )?)?,
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

fn proto_value_type_to_wire(value_type: i32) -> Result<WireValueType> {
    match api::ValueType::try_from(value_type)
        .map_err(|_| ProtocolError::UnsupportedValueType(value_type))?
    {
        api::ValueType::Null => Ok(WireValueType::Null),
        api::ValueType::Boolean => Ok(WireValueType::Boolean),
        api::ValueType::Integer => Ok(WireValueType::Integer),
        api::ValueType::Long => Ok(WireValueType::Long),
        api::ValueType::Double => Ok(WireValueType::Double),
        api::ValueType::String => Ok(WireValueType::String),
        api::ValueType::Bytes => Ok(WireValueType::Bytes),
        api::ValueType::Date => Ok(WireValueType::Date),
        api::ValueType::JsonObject => Ok(WireValueType::JsonObject),
        api::ValueType::JsonArray => Ok(WireValueType::JsonArray),
        api::ValueType::Text => Ok(WireValueType::Text),
        api::ValueType::IntegerCnt => Ok(WireValueType::IntegerCnt),
        api::ValueType::LongCnt => Ok(WireValueType::LongCnt),
        api::ValueType::IntegerDedupCnt => Ok(WireValueType::IntegerDedupCnt),
        api::ValueType::Tree => Ok(WireValueType::Tree),
    }
}

fn proto_required_time_ticket(
    ticket: &Option<api::TimeTicket>,
    field: &'static str,
) -> Result<CoreTimeTicket> {
    Ok(from_time_ticket(required(ticket, field)?))
}

fn proto_optional_time_ticket(ticket: &Option<api::TimeTicket>) -> Option<CoreTimeTicket> {
    ticket.as_ref().map(from_time_ticket)
}

fn required<'a, T>(value: &'a Option<T>, field: &'static str) -> Result<&'a T> {
    value.as_ref().ok_or(ProtocolError::MissingField(field))
}

fn usize_from_i32(value: i32, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| ProtocolError::InvalidOffset { field, value })
}

fn i32_from_usize(value: usize, field: &'static str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ProtocolError::InvalidOffset {
        field,
        value: i32::MIN,
    })
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

fn bytes_to_actor_id(bytes: &[u8]) -> String {
    let mut actor_id = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        actor_id.push_str(&format!("{byte:02x}"));
    }
    actor_id
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

fn base64_decode(value: &str) -> Result<Vec<u8>> {
    if value.len() % 4 != 0 {
        return Err(ProtocolError::InvalidBase64(value.to_owned()));
    }

    let mut bytes = Vec::with_capacity(value.len() / 4 * 3);
    for chunk in value.as_bytes().chunks(4) {
        let first = base64_value(chunk[0], value)?;
        let second = base64_value(chunk[1], value)?;
        let third = if chunk[2] == b'=' {
            None
        } else {
            Some(base64_value(chunk[2], value)?)
        };
        let fourth = if chunk[3] == b'=' {
            None
        } else {
            Some(base64_value(chunk[3], value)?)
        };

        bytes.push((first << 2) | (second >> 4));
        if let Some(third) = third {
            bytes.push(((second & 0b0000_1111) << 4) | (third >> 2));
            if let Some(fourth) = fourth {
                bytes.push(((third & 0b0000_0011) << 6) | fourth);
            }
        }
    }

    Ok(bytes)
}

fn base64_value(byte: u8, original: &str) -> Result<u8> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(ProtocolError::InvalidBase64(original.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_change_pack, encode_change_pack, from_change_pack, from_schema_rules,
        proto_change_pack_to_wire, proto_json_element_to_wire, proto_tree_nodes_to_wire,
        to_change_pack, to_schema_rules, to_time_ticket, to_version_vector,
        wire_change_pack_to_proto, wire_json_element_to_proto, wire_tree_nodes_to_proto,
        ProtocolError,
    };
    use crate::yorkie::v1::{self as api, json_element, operation::Body, ValueType};
    use prost::Message;
    use std::collections::BTreeMap;
    use std::error::Error;
    use yorkie_core::wire::{
        WireChange, WireChangeId, WireChangePack, WireJsonElement, WireJsonElementSimple,
        WireNodeAttr, WireOperation, WireRgaNode, WireRhtNode, WireTextNode, WireTextNodeId,
        WireTextNodePos, WireTreeNode, WireTreeNodeId, WireValueType,
    };
    use yorkie_core::{Checkpoint, Document, SchemaRule, TimeTicket, TreeNodeRule, VersionVector};

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
    fn converts_schema_rules_to_and_from_proto_shape() {
        let rules = vec![SchemaRule::new(
            "$.content",
            "tree",
            vec![TreeNodeRule::new(
                "paragraph",
                "text*",
                "bold italic",
                "block",
            )],
        )];

        let proto = to_schema_rules(&rules);

        assert_eq!("$.content", proto[0].path);
        assert_eq!("tree", proto[0].r#type);
        assert_eq!("paragraph", proto[0].tree_nodes[0].node_type);
        assert_eq!("text*", proto[0].tree_nodes[0].content);

        let decoded = from_schema_rules(&proto);
        assert_eq!(rules, decoded);
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

    #[test]
    fn encodes_object_simple_value_as_json_element_bytes() -> Result<(), Box<dyn Error>> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = yorkie_core::JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            Ok(())
        })?;

        let proto = to_change_pack(&doc.create_change_pack())?;
        let Some(Body::Set(set)) = &proto.changes[0].operations[0].body else {
            panic!("expected set operation");
        };
        let value = set.value.as_ref().expect("set value");
        assert_eq!(ValueType::JsonObject as i32, value.r#type);

        let element = crate::yorkie::v1::JsonElement::decode(value.value.as_slice())?;
        let Some(json_element::Body::JsonObject(object)) = element.body else {
            panic!("expected encoded object payload");
        };

        assert_eq!(1, object.nodes.len());
        assert_eq!("name", object.nodes[0].key);

        let mut target = Document::new("target-doc");
        target.apply_change_pack(&from_change_pack(&proto)?)?;
        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn encodes_array_simple_value_as_json_element_bytes() -> Result<(), Box<dyn Error>> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = yorkie_core::JsonArray::new();
            items.push("one")?.push(2i32)?;
            root.set("items", items)?;
            Ok(())
        })?;

        let proto = to_change_pack(&doc.create_change_pack())?;
        let Some(Body::Set(set)) = &proto.changes[0].operations[0].body else {
            panic!("expected set operation");
        };
        let value = set.value.as_ref().expect("set value");
        assert_eq!(ValueType::JsonArray as i32, value.r#type);

        let element = crate::yorkie::v1::JsonElement::decode(value.value.as_slice())?;
        let Some(json_element::Body::JsonArray(array)) = element.body else {
            panic!("expected encoded array payload");
        };

        let visible_nodes = array
            .nodes
            .iter()
            .filter(|node| node.element.is_some())
            .count();
        assert_eq!(2, visible_nodes);

        let mut target = Document::new("target-doc");
        target.apply_change_pack(&from_change_pack(&proto)?)?;
        assert_eq!(r#"{"items":["one",2]}"#, target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn roundtrips_tree_nodes_through_proto_shape() -> Result<(), Box<dyn Error>> {
        let nodes = sample_tree_nodes();

        let proto = wire_tree_nodes_to_proto(&nodes)?;
        assert_eq!(5, proto.content.len());
        assert_eq!("text", proto.content[0].r#type);
        assert_eq!("hello", proto.content[0].value);
        assert_eq!(2, proto.content[0].depth);
        assert_eq!("p", proto.content[1].r#type);
        assert_eq!(1, proto.content[1].attributes.len());
        assert_eq!("r", proto.content[4].r#type);

        let decoded = proto_tree_nodes_to_wire(&proto)?;
        assert_eq!(nodes, decoded);
        Ok(())
    }

    #[test]
    fn roundtrips_tree_json_element_bytes() -> Result<(), Box<dyn Error>> {
        let element = sample_tree_element();

        let bytes = wire_json_element_to_proto(&element)?.encode_to_vec();
        let proto = api::JsonElement::decode(bytes.as_slice())?;
        let Some(json_element::Body::Tree(tree)) = &proto.body else {
            panic!("expected encoded tree payload");
        };

        assert_eq!(5, tree.nodes.len());
        let attr = tree.nodes[1].attributes.get("b").expect("tree attr");
        assert_eq!("t", attr.value);
        assert!(!attr.is_removed);

        let decoded = proto_json_element_to_wire(&proto)?;
        assert_eq!(element, decoded);
        Ok(())
    }

    #[test]
    fn roundtrips_tree_merge_metadata_through_proto_payload() -> Result<(), Box<dyn Error>> {
        let merged_from = tree_node_id(5, 0);
        let merged_at = ticket(10);
        let mut moved_text = tree_text_node(tree_node_id(6, 0), "b", 2);
        moved_text.merged_from = Some(merged_from.clone());
        moved_text.merged_at = Some(merged_at.clone());

        let mut removed_parent = tree_element_node(merged_from.clone(), "p", 1);
        removed_parent.removed_at = Some(merged_at.clone());

        let element = WireJsonElement::Tree {
            nodes: vec![
                tree_text_node(tree_node_id(3, 0), "a", 2),
                moved_text,
                tree_element_node(tree_node_id(2, 0), "p", 1),
                removed_parent,
                tree_element_node(tree_node_id(1, 0), "r", 0),
            ],
            created_at: TimeTicket::initial(),
            moved_at: None,
            removed_at: None,
        };

        let proto = wire_json_element_to_proto(&element)?;
        let Some(json_element::Body::Tree(tree)) = &proto.body else {
            panic!("expected encoded tree payload");
        };
        let moved_proto = tree
            .nodes
            .iter()
            .find(|node| node.merged_from.is_some())
            .expect("node with merge metadata");
        let proto_merged_from = moved_proto.merged_from.as_ref().expect("merged_from");
        assert_eq!(5, proto_merged_from.created_at.as_ref().unwrap().lamport);
        assert_eq!(0, proto_merged_from.offset);
        assert_eq!(10, moved_proto.merged_at.as_ref().unwrap().lamport);

        let decoded = proto_json_element_to_wire(&proto)?;
        assert_eq!(element, decoded);
        Ok(())
    }

    #[test]
    fn roundtrips_snapshot_root_change_pack_through_proto_bytes() -> Result<(), Box<dyn Error>> {
        let actor_id = "000000000000000000000001";
        let mut version_vector = VersionVector::new();
        version_vector.set(actor_id, 12);

        let root = sample_snapshot_root();
        let pack = WireChangePack {
            document_key: "doc-key".to_owned(),
            checkpoint: Checkpoint::new(7, 0),
            is_removed: false,
            changes: Vec::new(),
            snapshot: None,
            snapshot_root: Some(root.clone()),
            version_vector: Some(version_vector.clone()),
        };

        let proto = wire_change_pack_to_proto(&pack)?;
        assert!(!proto.snapshot.is_empty());
        assert!(proto.changes.is_empty());

        let snapshot = proto.snapshot.clone();
        let decoded_wire = proto_change_pack_to_wire(&proto)?;
        assert_eq!("doc-key", decoded_wire.document_key);
        assert_eq!(Checkpoint::new(7, 0), decoded_wire.checkpoint);
        assert_eq!(Some(snapshot), decoded_wire.snapshot);
        assert_eq!(Some(root), decoded_wire.snapshot_root);
        assert_eq!(Some(version_vector), decoded_wire.version_vector);

        let mut doc = Document::new("doc-key");
        doc.apply_change_pack(&from_change_pack(&proto)?)?;
        assert_eq!(
            r#"{"k1":[{"val":"B"}],"k2":[true,1,"4"],"k3":6,"meta":{"active":true,"name":"yorkie"}}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    #[test]
    fn roundtrips_mixed_change_pack_operations_through_proto_replay() -> Result<(), Box<dyn Error>>
    {
        let object_at = ticket(1);
        let array_at = ticket(5);
        let text_at = ticket(11);
        let text_insert_at = ticket(12);
        let counter_at = ticket(14);

        let pack = WireChangePack {
            document_key: "doc-key".to_owned(),
            checkpoint: Checkpoint::new(0, 1),
            is_removed: false,
            changes: vec![WireChange {
                id: wire_change_id(1, 30),
                message: None,
                operations: vec![
                    WireOperation::Set {
                        parent_created_at: TimeTicket::initial(),
                        key: "k1".to_owned(),
                        value: simple_object(object_at.clone()),
                        executed_at: object_at.clone(),
                    },
                    WireOperation::Set {
                        parent_created_at: object_at.clone(),
                        key: "flag".to_owned(),
                        value: simple_bool(true, ticket(2)),
                        executed_at: ticket(2),
                    },
                    WireOperation::Set {
                        parent_created_at: object_at.clone(),
                        key: "title".to_owned(),
                        value: simple_str("4", ticket(3)),
                        executed_at: ticket(3),
                    },
                    WireOperation::Remove {
                        parent_created_at: object_at.clone(),
                        created_at: ticket(3),
                        executed_at: ticket(4),
                    },
                    WireOperation::Set {
                        parent_created_at: TimeTicket::initial(),
                        key: "k2".to_owned(),
                        value: simple_array(array_at.clone()),
                        executed_at: array_at.clone(),
                    },
                    WireOperation::Add {
                        parent_created_at: array_at.clone(),
                        prev_created_at: TimeTicket::initial(),
                        value: simple_bool(true, ticket(6)),
                        executed_at: ticket(6),
                    },
                    WireOperation::Add {
                        parent_created_at: array_at.clone(),
                        prev_created_at: ticket(6),
                        value: simple_i32(1, ticket(7)),
                        executed_at: ticket(7),
                    },
                    WireOperation::Add {
                        parent_created_at: array_at.clone(),
                        prev_created_at: ticket(7),
                        value: simple_str("4", ticket(8)),
                        executed_at: ticket(8),
                    },
                    WireOperation::Remove {
                        parent_created_at: array_at.clone(),
                        created_at: ticket(8),
                        executed_at: ticket(9),
                    },
                    WireOperation::Move {
                        parent_created_at: array_at,
                        prev_created_at: TimeTicket::initial(),
                        created_at: ticket(7),
                        executed_at: ticket(10),
                    },
                    WireOperation::Set {
                        parent_created_at: TimeTicket::initial(),
                        key: "k3".to_owned(),
                        value: simple_text(text_at.clone()),
                        executed_at: text_at.clone(),
                    },
                    WireOperation::Edit {
                        parent_created_at: text_at.clone(),
                        from: text_pos(TimeTicket::initial(), 0, 0),
                        to: text_pos(TimeTicket::initial(), 0, 0),
                        content: "Hello World".to_owned(),
                        attributes: BTreeMap::new(),
                        executed_at: text_insert_at.clone(),
                    },
                    WireOperation::Style {
                        parent_created_at: text_at,
                        from: text_pos(TimeTicket::initial(), 0, 0),
                        to: text_pos(text_insert_at, 0, 5),
                        attributes: BTreeMap::from([("b".to_owned(), "1".to_owned())]),
                        attributes_to_remove: Vec::new(),
                        executed_at: ticket(13),
                    },
                    WireOperation::Set {
                        parent_created_at: TimeTicket::initial(),
                        key: "k4".to_owned(),
                        value: simple_counter_i32(0, counter_at.clone()),
                        executed_at: counter_at.clone(),
                    },
                    WireOperation::Increase {
                        parent_created_at: counter_at,
                        value: simple_i32(5, ticket(15)),
                        executed_at: ticket(15),
                        actor: None,
                    },
                ],
            }],
            snapshot: None,
            snapshot_root: None,
            version_vector: Some(version_vector_with_lamport(30)),
        };

        let proto = wire_change_pack_to_proto(&pack)?;
        assert_eq!(15, proto.changes[0].operations.len());
        assert!(matches!(
            &proto.changes[0].operations[9].body,
            Some(Body::Move(_))
        ));
        assert!(matches!(
            &proto.changes[0].operations[11].body,
            Some(Body::Edit(_))
        ));
        assert!(matches!(
            &proto.changes[0].operations[12].body,
            Some(Body::Style(_))
        ));
        assert!(matches!(
            &proto.changes[0].operations[14].body,
            Some(Body::Increase(_))
        ));

        let decoded_wire = proto_change_pack_to_wire(&proto)?;
        assert_eq!("doc-key", decoded_wire.document_key);
        assert_eq!(Checkpoint::new(0, 1), decoded_wire.checkpoint);
        assert_eq!(1, decoded_wire.changes.len());
        assert_eq!(15, decoded_wire.changes[0].operations.len());

        let mut doc = Document::new("doc-key");
        doc.apply_change_pack(&from_change_pack(&proto)?)?;
        assert_eq!(
            r#"{"k1":{"flag":true},"k2":[1,true],"k3":[{"attrs":{"b":1},"val":"Hello"},{"val":" World"}],"k4":5}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    #[test]
    fn rejects_change_pack_without_checkpoint() {
        let err = from_change_pack(&api::ChangePack::default()).unwrap_err();

        assert_eq!(ProtocolError::MissingField("change_pack.checkpoint"), err);
    }

    #[test]
    fn decodes_proto_change_pack_into_core_domain() -> Result<(), Box<dyn Error>> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            let mut profile = yorkie_core::JsonObject::new();
            profile.set("name", "yorkie")?;
            let mut todos = yorkie_core::JsonArray::new();
            todos.push("write protocol")?.push(false)?;
            root.set("profile", profile)?;
            root.set("todos", todos)?;
            root.set_counter("count", 1)?.increase(2i32)?;
            Ok(())
        })?;

        let proto = to_change_pack(&source.create_change_pack())?;
        let remote_pack = from_change_pack(&proto)?;
        target.apply_change_pack(&remote_pack)?;

        assert_eq!(
            r#"{"count":3,"profile":{"name":"yorkie"},"todos":["write protocol",false]}"#,
            target.to_sorted_json()
        );
        Ok(())
    }

    #[test]
    fn roundtrips_binary_change_pack() -> Result<(), Box<dyn Error>> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            let mut array = yorkie_core::JsonArray::new();
            array.push("sync")?.push(1i32)?;
            root.set("items", array)?;
            Ok(())
        })?;

        let bytes = encode_change_pack(&source.create_change_pack())?;
        let decoded = decode_change_pack(&bytes)?;
        target.apply_change_pack(&decoded)?;

        assert_eq!(r#"{"items":["sync",1]}"#, target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn decodes_proto_snapshot_change_pack_into_document_root() -> Result<(), Box<dyn Error>> {
        let actor_id = "000000000000000000000001";
        let mut version_vector = yorkie_core::VersionVector::new();
        version_vector.set(actor_id, 1);

        let snapshot = api::Snapshot {
            root: Some(api::JsonElement {
                body: Some(json_element::Body::JsonObject(json_element::JsonObject {
                    nodes: vec![api::RhtNode {
                        key: "title".to_owned(),
                        element: Some(api::JsonElement {
                            body: Some(json_element::Body::Primitive(json_element::Primitive {
                                r#type: ValueType::String as i32,
                                value: b"snapshot".to_vec(),
                                created_at: Some(to_time_ticket(&TimeTicket::new(1, 1, actor_id))?),
                                moved_at: None,
                                removed_at: None,
                            })),
                        }),
                    }],
                    created_at: Some(to_time_ticket(&TimeTicket::initial())?),
                    moved_at: None,
                    removed_at: None,
                })),
            }),
            presences: BTreeMap::new(),
        }
        .encode_to_vec();

        let proto = api::ChangePack {
            document_key: "target-doc".to_owned(),
            checkpoint: Some(api::Checkpoint {
                server_seq: 7,
                client_seq: 0,
            }),
            snapshot,
            changes: Vec::new(),
            min_synced_ticket: None,
            is_removed: false,
            version_vector: Some(to_version_vector(&version_vector)?),
        };

        let pack = from_change_pack(&proto)?;
        let encoded_proto = to_change_pack(&pack)?;
        assert_eq!(proto.snapshot, encoded_proto.snapshot);

        let mut doc = Document::new("target-doc");
        doc.apply_change_pack(&pack)?;

        assert_eq!(r#"{"title":"snapshot"}"#, doc.to_sorted_json());
        assert_eq!(yorkie_core::Checkpoint::new(7, 0), doc.checkpoint());
        Ok(())
    }

    fn sample_snapshot_root() -> WireJsonElement {
        WireJsonElement::Object {
            nodes: vec![
                rht_node("k1", text_element("B", ticket(2))),
                rht_node("k2", array_element()),
                rht_node("k3", counter_i32(6, ticket(8))),
                rht_node(
                    "meta",
                    WireJsonElement::Object {
                        nodes: vec![
                            rht_node("active", primitive_bool(true, ticket(10))),
                            rht_node("name", primitive_str("yorkie", ticket(11))),
                        ],
                        created_at: ticket(9),
                        moved_at: None,
                        removed_at: None,
                    },
                ),
            ],
            created_at: TimeTicket::initial(),
            moved_at: None,
            removed_at: None,
        }
    }

    fn rht_node(key: &str, element: WireJsonElement) -> WireRhtNode {
        WireRhtNode {
            key: key.to_owned(),
            element,
        }
    }

    fn array_element() -> WireJsonElement {
        WireJsonElement::Array {
            nodes: vec![
                rga_node(primitive_bool(true, ticket(4))),
                rga_node(primitive_i32(1, ticket(5))),
                rga_node(primitive_str("4", ticket(6))),
            ],
            created_at: ticket(3),
            moved_at: None,
            removed_at: None,
        }
    }

    fn rga_node(element: WireJsonElement) -> WireRgaNode {
        WireRgaNode {
            element: Some(element),
            position_created_at: None,
            position_moved_at: None,
            position_removed_at: None,
        }
    }

    fn text_element(value: &str, created_at: TimeTicket) -> WireJsonElement {
        WireJsonElement::Text {
            nodes: vec![WireTextNode {
                id: WireTextNodeId {
                    created_at: created_at.clone(),
                    offset: 0,
                },
                value: value.to_owned(),
                removed_at: None,
                ins_prev_id: None,
                attributes: BTreeMap::new(),
            }],
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    fn counter_i32(value: i32, created_at: TimeTicket) -> WireJsonElement {
        WireJsonElement::Counter {
            value_type: WireValueType::IntegerCnt,
            value: value.to_le_bytes().to_vec(),
            hll_registers: Vec::new(),
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    fn primitive_bool(value: bool, created_at: TimeTicket) -> WireJsonElement {
        WireJsonElement::Primitive {
            value_type: WireValueType::Boolean,
            value: vec![u8::from(value)],
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    fn primitive_i32(value: i32, created_at: TimeTicket) -> WireJsonElement {
        WireJsonElement::Primitive {
            value_type: WireValueType::Integer,
            value: value.to_le_bytes().to_vec(),
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> WireJsonElement {
        WireJsonElement::Primitive {
            value_type: WireValueType::String,
            value: value.as_bytes().to_vec(),
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    fn wire_change_id(client_seq: u32, lamport: i64) -> WireChangeId {
        WireChangeId {
            client_seq,
            server_seq: 0,
            lamport,
            actor_id: "000000000000000000000001".to_owned(),
            version_vector: version_vector_with_lamport(lamport),
        }
    }

    fn version_vector_with_lamport(lamport: i64) -> VersionVector {
        let mut version_vector = VersionVector::new();
        version_vector.set("000000000000000000000001", lamport);
        version_vector
    }

    fn simple_object(created_at: TimeTicket) -> WireJsonElementSimple {
        simple_full_element(
            created_at.clone(),
            WireValueType::JsonObject,
            WireJsonElement::Object {
                nodes: Vec::new(),
                created_at,
                moved_at: None,
                removed_at: None,
            },
        )
    }

    fn simple_array(created_at: TimeTicket) -> WireJsonElementSimple {
        simple_full_element(
            created_at.clone(),
            WireValueType::JsonArray,
            WireJsonElement::Array {
                nodes: Vec::new(),
                created_at,
                moved_at: None,
                removed_at: None,
            },
        )
    }

    fn simple_full_element(
        created_at: TimeTicket,
        value_type: WireValueType,
        element: WireJsonElement,
    ) -> WireJsonElementSimple {
        WireJsonElementSimple {
            created_at,
            moved_at: None,
            removed_at: None,
            value_type,
            value: Vec::new(),
            element: Some(Box::new(element)),
        }
    }

    fn simple_text(created_at: TimeTicket) -> WireJsonElementSimple {
        WireJsonElementSimple {
            created_at,
            moved_at: None,
            removed_at: None,
            value_type: WireValueType::Text,
            value: Vec::new(),
            element: None,
        }
    }

    fn simple_counter_i32(value: i32, created_at: TimeTicket) -> WireJsonElementSimple {
        WireJsonElementSimple {
            created_at,
            moved_at: None,
            removed_at: None,
            value_type: WireValueType::IntegerCnt,
            value: value.to_le_bytes().to_vec(),
            element: None,
        }
    }

    fn simple_bool(value: bool, created_at: TimeTicket) -> WireJsonElementSimple {
        WireJsonElementSimple {
            created_at,
            moved_at: None,
            removed_at: None,
            value_type: WireValueType::Boolean,
            value: vec![u8::from(value)],
            element: None,
        }
    }

    fn simple_i32(value: i32, created_at: TimeTicket) -> WireJsonElementSimple {
        WireJsonElementSimple {
            created_at,
            moved_at: None,
            removed_at: None,
            value_type: WireValueType::Integer,
            value: value.to_le_bytes().to_vec(),
            element: None,
        }
    }

    fn simple_str(value: &str, created_at: TimeTicket) -> WireJsonElementSimple {
        WireJsonElementSimple {
            created_at,
            moved_at: None,
            removed_at: None,
            value_type: WireValueType::String,
            value: value.as_bytes().to_vec(),
            element: None,
        }
    }

    fn text_pos(created_at: TimeTicket, offset: usize, relative_offset: usize) -> WireTextNodePos {
        WireTextNodePos {
            id: WireTextNodeId { created_at, offset },
            relative_offset,
        }
    }

    fn sample_tree_element() -> WireJsonElement {
        WireJsonElement::Tree {
            nodes: sample_tree_nodes(),
            created_at: TimeTicket::initial(),
            moved_at: None,
            removed_at: None,
        }
    }

    fn sample_tree_nodes() -> Vec<WireTreeNode> {
        let mut first_paragraph = tree_element_node(tree_node_id(2, 0), "p", 1);
        first_paragraph.attributes.insert(
            "b".to_owned(),
            WireNodeAttr {
                value: "t".to_owned(),
                updated_at: ticket(6),
                is_removed: false,
            },
        );

        vec![
            tree_text_node(tree_node_id(3, 0), "hello", 2),
            first_paragraph,
            tree_text_node(tree_node_id(5, 0), "world", 2),
            tree_element_node(tree_node_id(4, 0), "p", 1),
            tree_element_node(tree_node_id(1, 0), "r", 0),
        ]
    }

    fn tree_text_node(id: WireTreeNodeId, value: &str, depth: i32) -> WireTreeNode {
        WireTreeNode {
            id,
            node_type: "text".to_owned(),
            value: value.to_owned(),
            removed_at: None,
            ins_prev_id: None,
            ins_next_id: None,
            depth,
            attributes: BTreeMap::new(),
            merged_from: None,
            merged_at: None,
        }
    }

    fn tree_element_node(id: WireTreeNodeId, node_type: &str, depth: i32) -> WireTreeNode {
        WireTreeNode {
            id,
            node_type: node_type.to_owned(),
            value: String::new(),
            removed_at: None,
            ins_prev_id: None,
            ins_next_id: None,
            depth,
            attributes: BTreeMap::new(),
            merged_from: None,
            merged_at: None,
        }
    }

    fn tree_node_id(lamport: i64, offset: usize) -> WireTreeNodeId {
        WireTreeNodeId {
            created_at: ticket(lamport),
            offset,
        }
    }

    fn ticket(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, "000000000000000000000001")
    }
}
