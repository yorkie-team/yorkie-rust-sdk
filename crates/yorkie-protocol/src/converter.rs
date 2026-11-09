// The server proto marks some legacy fields deprecated; the wire format still
// carries them, so the converter sets them explicitly.
#![allow(deprecated)]

use crate::yorkie::v1 as api;
use prost::Message;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use yorkie_core::wire::{
    WireChange, WireChangeId, WireChangePack, WireJsonElement, WireJsonElementSimple, WireNodeAttr,
    WireOperation, WirePresence, WirePresenceChange, WireRestoreMode, WireRestoreSpan, WireRgaNode,
    WireRhtNode, WireTextNode, WireTextNodeId, WireTextNodePos, WireTreeNode, WireTreeNodeId,
    WireTreePos, WireTreeRestoreSpan, WireValueType,
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
        snapshot_root_to_bytes(snapshot_root, &pack.snapshot_presences)?
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
        ..Default::default()
    })
}

fn proto_change_pack_to_wire(pack: &api::ChangePack) -> Result<WireChangePack> {
    let (snapshot_root, snapshot_presences) = if pack.snapshot.is_empty() {
        (None, BTreeMap::new())
    } else {
        let (root, presences) = snapshot_from_bytes(&pack.snapshot)?;
        (Some(root), presences)
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
        snapshot_presences,
        version_vector: pack
            .version_vector
            .as_ref()
            .map(from_version_vector)
            .transpose()?,
    })
}

fn snapshot_root_to_bytes(
    snapshot_root: &WireJsonElement,
    presences: &BTreeMap<String, WirePresence>,
) -> Result<Vec<u8>> {
    Ok(api::Snapshot {
        root: Some(wire_json_element_to_proto(snapshot_root)?),
        presences: presences
            .iter()
            .map(|(actor_id, presence)| (actor_id.clone(), wire_presence_to_proto(presence)))
            .collect(),
    }
    .encode_to_vec())
}

#[allow(clippy::type_complexity)]
fn snapshot_from_bytes(
    snapshot: &[u8],
) -> Result<(WireJsonElement, BTreeMap<String, WirePresence>)> {
    let snapshot = api::Snapshot::decode(snapshot)?;
    let root = proto_json_element_to_wire(required(&snapshot.root, "snapshot.root")?)?;
    let presences = snapshot
        .presences
        .iter()
        .map(|(actor_id, presence)| (actor_id.clone(), proto_presence_to_wire(presence)))
        .collect();
    Ok((root, presences))
}

fn wire_presence_to_proto(presence: &WirePresence) -> api::Presence {
    api::Presence {
        data: presence.clone(),
    }
}

fn proto_presence_to_wire(presence: &api::Presence) -> WirePresence {
    presence.data.clone()
}

fn wire_presence_change_to_proto(change: &WirePresenceChange) -> api::PresenceChange {
    use api::presence_change::ChangeType;
    match change {
        WirePresenceChange::Put(presence) => api::PresenceChange {
            r#type: ChangeType::Put as i32,
            presence: Some(wire_presence_to_proto(presence)),
            removed_keys: Vec::new(),
        },
        WirePresenceChange::Patch(presence) => api::PresenceChange {
            r#type: ChangeType::Patch as i32,
            presence: Some(wire_presence_to_proto(presence)),
            removed_keys: Vec::new(),
        },
        WirePresenceChange::Clear => api::PresenceChange {
            r#type: ChangeType::Clear as i32,
            presence: None,
            removed_keys: Vec::new(),
        },
    }
}

// Mirrors JS `fromPresenceChange`: every type decodes, so a stored change the
// server redelivers on each pull can never wedge the pack. UNSPECIFIED (what a
// server without the presence-patch capability stores for a patch) and DELETE
// decode as an empty Put; PATCH keeps its keys and tolerates an absent presence.
fn proto_presence_change_to_wire(change: &api::PresenceChange) -> Result<WirePresenceChange> {
    use api::presence_change::ChangeType;
    match ChangeType::try_from(change.r#type) {
        Ok(ChangeType::Put) => Ok(WirePresenceChange::Put(proto_presence_to_wire(required(
            &change.presence,
            "presence_change.presence",
        )?))),
        Ok(ChangeType::Clear) => Ok(WirePresenceChange::Clear),
        Ok(ChangeType::Patch) => Ok(WirePresenceChange::Patch(
            change
                .presence
                .as_ref()
                .map(proto_presence_to_wire)
                .unwrap_or_default(),
        )),
        Ok(ChangeType::Unspecified | ChangeType::Delete) | Err(_) => {
            Ok(WirePresenceChange::Put(WirePresence::new()))
        }
    }
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
        presence_change: change
            .presence_change
            .as_ref()
            .map(wire_presence_change_to_proto),
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
        presence_change: change
            .presence_change
            .as_ref()
            .map(proto_presence_change_to_wire)
            .transpose()?,
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
            ..Default::default()
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
            ..Default::default()
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
            restore_spans,
            restore_mode,
            retombstone_spans,
        } => {
            let mut edit = api::operation::Edit {
                parent_created_at: Some(to_time_ticket(parent_created_at)?),
                from: Some(wire_text_node_pos_to_proto(from)?),
                to: Some(wire_text_node_pos_to_proto(to)?),
                created_at_map_by_actor: BTreeMap::new(),
                content: content.clone(),
                executed_at: Some(to_time_ticket(executed_at)?),
                attributes: attributes.clone(),
                ..Default::default()
            };
            // JS `toOperation`: both span lists and the mode are written only
            // when a list is non-empty, so an ordinary edit keeps its bytes.
            if !restore_spans.is_empty() || !retombstone_spans.is_empty() {
                edit.restore_spans = wire_restore_spans_to_proto(restore_spans)?;
                edit.retombstone_spans = wire_restore_spans_to_proto(retombstone_spans)?;
                edit.restore_mode = wire_restore_mode_to_proto(*restore_mode) as i32;
            }
            api::operation::Body::Edit(edit)
        }
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
            split_tickets,
            restore_spans,
            restore_mode,
            retombstone_spans,
        } => {
            let mut tree_edit = api::operation::TreeEdit {
                parent_created_at: Some(to_time_ticket(parent_created_at)?),
                from: Some(wire_tree_pos_to_proto(from)?),
                to: Some(wire_tree_pos_to_proto(to)?),
                created_at_map_by_actor: BTreeMap::new(),
                contents: contents
                    .as_ref()
                    .map(|groups| {
                        groups
                            .iter()
                            .map(|nodes| wire_tree_nodes_to_proto(nodes))
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
                split_level: i32_from_usize(*split_level, "tree_edit.split_level")?,
                executed_at: Some(to_time_ticket(executed_at)?),
                split_tickets: split_tickets
                    .iter()
                    .map(to_time_ticket)
                    .collect::<Result<Vec<_>>>()?,
                ..Default::default()
            };
            // JS `toOperation`, as for Text: both span lists and the mode
            // are written only when a list is non-empty.
            if !restore_spans.is_empty() || !retombstone_spans.is_empty() {
                tree_edit.restore_spans = wire_tree_restore_spans_to_proto(restore_spans)?;
                tree_edit.retombstone_spans = wire_tree_restore_spans_to_proto(retombstone_spans)?;
                tree_edit.restore_mode = wire_restore_mode_to_proto(*restore_mode) as i32;
            }
            api::operation::Body::TreeEdit(tree_edit)
        }
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
            // Both maps, as JS and Go write them: the reverse of a style that
            // overwrote some attributes and added others carries both.
            attributes: attributes.clone(),
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
            ..Default::default()
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
        api::operation::Body::Edit(operation) => {
            // JS `fromOperation`: the spans are read only when a list is
            // non-empty; then any mode but RETOMBSTONE means restore.
            let (restore_spans, restore_mode, retombstone_spans) =
                if operation.restore_spans.is_empty() && operation.retombstone_spans.is_empty() {
                    (Vec::new(), None, Vec::new())
                } else {
                    (
                        proto_restore_spans_to_wire(&operation.restore_spans)?,
                        Some(proto_restore_mode_to_wire(operation.restore_mode)),
                        proto_restore_spans_to_wire(&operation.retombstone_spans)?,
                    )
                };
            Ok(WireOperation::Edit {
                parent_created_at: proto_required_time_ticket(
                    &operation.parent_created_at,
                    "operation.edit.parent_created_at",
                )?,
                from: proto_text_node_pos_to_wire(required(
                    &operation.from,
                    "operation.edit.from",
                )?)?,
                to: proto_text_node_pos_to_wire(required(&operation.to, "operation.edit.to")?)?,
                content: operation.content.clone(),
                attributes: operation.attributes.clone(),
                executed_at: proto_required_time_ticket(
                    &operation.executed_at,
                    "operation.edit.executed_at",
                )?,
                restore_spans,
                restore_mode,
                retombstone_spans,
            })
        }
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
        api::operation::Body::TreeEdit(operation) => {
            // JS `fromOperation`: the spans are read only when a list is
            // non-empty, and then the mode is set, RESTORE unless the sender
            // wrote RETOMBSTONE, the proto default included.
            let (restore_spans, restore_mode, retombstone_spans) =
                if operation.restore_spans.is_empty() && operation.retombstone_spans.is_empty() {
                    (Vec::new(), None, Vec::new())
                } else {
                    (
                        proto_tree_restore_spans_to_wire(&operation.restore_spans)?,
                        Some(proto_restore_mode_to_wire(operation.restore_mode)),
                        proto_tree_restore_spans_to_wire(&operation.retombstone_spans)?,
                    )
                };
            Ok(WireOperation::TreeEdit {
                parent_created_at: proto_required_time_ticket(
                    &operation.parent_created_at,
                    "operation.tree_edit.parent_created_at",
                )?,
                from: proto_tree_pos_to_wire(required(
                    &operation.from,
                    "operation.tree_edit.from",
                )?)?,
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
                split_level: usize_from_i32(
                    operation.split_level,
                    "operation.tree_edit.split_level",
                )?,
                executed_at: proto_required_time_ticket(
                    &operation.executed_at,
                    "operation.tree_edit.executed_at",
                )?,
                // Read back as sent, as JS and Go read them: the server applies
                // them verbatim.
                split_tickets: operation
                    .split_tickets
                    .iter()
                    .map(from_time_ticket)
                    .collect(),
                restore_spans,
                restore_mode,
                retombstone_spans,
            })
        }
        api::operation::Body::TreeStyle(operation) => Ok(WireOperation::TreeStyle {
            parent_created_at: proto_required_time_ticket(
                &operation.parent_created_at,
                "operation.tree_style.parent_created_at",
            )?,
            from: proto_tree_pos_to_wire(required(&operation.from, "operation.tree_style.from")?)?,
            to: proto_tree_pos_to_wire(required(&operation.to, "operation.tree_style.to")?)?,
            attributes: operation.attributes.clone(),
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

fn wire_restore_spans_to_proto(spans: &[WireRestoreSpan]) -> Result<Vec<api::RestoreSpan>> {
    spans
        .iter()
        .map(|span| {
            Ok(api::RestoreSpan {
                created_at: Some(to_time_ticket(&span.created_at)?),
                start: i32_from_usize(span.start, "restore_span.start")?,
                end: i32_from_usize(span.end, "restore_span.end")?,
                content: span.content.clone(),
                attributes: span.attributes.clone(),
            })
        })
        .collect()
}

/// Reads the spans of an identity edit (JS `fromPbSpan`, Go
/// `fromRestoreSpans`). A span addresses content by insertion identity, so it
/// is malformed without a `created_at`, and `restore` slices its content by
/// offset, so `[start, end)` has to be an ordered interval exactly covering
/// the content, in UTF-16 code units. A malformed span rejects the whole
/// operation here, as JS throws `ErrInvalidArgument`, instead of failing deep
/// in the restore path on every peer.
fn proto_restore_spans_to_wire(spans: &[api::RestoreSpan]) -> Result<Vec<WireRestoreSpan>> {
    spans
        .iter()
        .map(|span| {
            let malformed = || {
                ProtocolError::Core(YorkieError::InvalidArgument(
                    "malformed restore span: missing timestamp or invalid range".to_owned(),
                ))
            };
            let created_at = span.created_at.as_ref().ok_or_else(malformed)?;
            let start = usize::try_from(span.start).map_err(|_| malformed())?;
            let end = usize::try_from(span.end).map_err(|_| malformed())?;
            if end < start || end - start != span.content.encode_utf16().count() {
                return Err(malformed());
            }
            Ok(WireRestoreSpan {
                created_at: from_time_ticket(created_at),
                start,
                end,
                content: span.content.clone(),
                attributes: span.attributes.clone(),
            })
        })
        .collect()
}

fn wire_restore_mode_to_proto(mode: Option<WireRestoreMode>) -> api::RestoreMode {
    match mode {
        Some(WireRestoreMode::Retombstone) => api::RestoreMode::Retombstone,
        Some(WireRestoreMode::Restore) | None => api::RestoreMode::Restore,
    }
}

/// Any mode but RETOMBSTONE, unspecified or unknown included, means restore
/// (JS `fromOperation`).
fn proto_restore_mode_to_wire(mode: i32) -> WireRestoreMode {
    if mode == api::RestoreMode::Retombstone as i32 {
        WireRestoreMode::Retombstone
    } else {
        WireRestoreMode::Restore
    }
}

/// JS `toPbTreeRestoreSpan`. An empty `value` and an absent anchor are not
/// written, as JS leaves them unset.
fn wire_tree_restore_spans_to_proto(
    spans: &[WireTreeRestoreSpan],
) -> Result<Vec<api::TreeRestoreSpan>> {
    let anchor =
        |id: &Option<WireTreeNodeId>| id.as_ref().map(wire_tree_node_id_to_proto).transpose();
    spans
        .iter()
        .map(|span| {
            Ok(api::TreeRestoreSpan {
                id: Some(wire_tree_node_id_to_proto(&span.id)?),
                node_type: span.node_type.clone(),
                is_text: span.is_text,
                length: i32_from_usize(span.length, "tree_restore_span.length")?,
                value: span.value.clone(),
                attributes: wire_attrs_to_proto(&span.attributes)?,
                parent_id: anchor(&span.parent_id)?,
                left_sibling_id: anchor(&span.left_sibling_id)?,
                right_sibling_id: anchor(&span.right_sibling_id)?,
            })
        })
        .collect()
}

/// Reads the spans of an identity Tree edit (JS `fromPbTreeRestoreSpan`, Go
/// `fromTreeRestoreSpans`). A span addresses nodes by insertion identity, so
/// its `id` and every anchor it carries are malformed without a
/// `created_at`, and its attribute snapshot without an `updated_at`: those
/// reject the whole operation with `ErrInvalidArgument`, as JS throws,
/// instead of failing deep in the restore path. Nothing else is checked, as
/// in JS: a `length` that disagrees with `value`, or a kind that disagrees
/// with `node_type` (which restore skips), decodes, where Go rejects the
/// former. A negative `length` or offset, which `usize` cannot hold, is
/// rejected as a negative offset is everywhere else in this decoder; Go
/// rejects a negative `length` too.
fn proto_tree_restore_spans_to_wire(
    spans: &[api::TreeRestoreSpan],
) -> Result<Vec<WireTreeRestoreSpan>> {
    let anchor =
        |id: &Option<api::TreeNodeId>| id.as_ref().map(proto_tree_node_id_to_wire).transpose();
    spans
        .iter()
        .map(|span| {
            let malformed = || {
                ProtocolError::Core(YorkieError::InvalidArgument(
                    "malformed tree restore span: missing timestamp".to_owned(),
                ))
            };
            let id = span
                .id
                .as_ref()
                .filter(|id| id.created_at.is_some())
                .ok_or_else(malformed)?;
            let anchors = [
                &span.parent_id,
                &span.left_sibling_id,
                &span.right_sibling_id,
            ];
            if anchors.iter().any(|anchor| {
                anchor
                    .as_ref()
                    .is_some_and(|anchor| anchor.created_at.is_none())
            }) || span
                .attributes
                .values()
                .any(|attr| attr.updated_at.is_none())
            {
                return Err(malformed());
            }
            Ok(WireTreeRestoreSpan {
                id: proto_tree_node_id_to_wire(id)?,
                node_type: span.node_type.clone(),
                is_text: span.is_text,
                length: usize_from_i32(span.length, "tree_restore_span.length")?,
                value: span.value.clone(),
                attributes: proto_attrs_to_wire(&span.attributes)?,
                parent_id: anchor(&span.parent_id)?,
                left_sibling_id: anchor(&span.left_sibling_id)?,
                right_sibling_id: anchor(&span.right_sibling_id)?,
            })
        })
        .collect()
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
        // JS `toTextNodes`: empty when `removed_at` says it all, so such a
        // node encodes to the bytes it did before the ledger.
        removal_events: node
            .removal_events
            .iter()
            .map(to_time_ticket)
            .collect::<Result<Vec<_>>>()?,
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
        // Kept as sent; the domain decoder drops a ledger that breaks the
        // rules (JS `fromTextNode`).
        removal_events: node.removal_events.iter().map(from_time_ticket).collect(),
    })
}

fn wire_tree_nodes_to_proto(nodes: &[WireTreeNode]) -> Result<api::TreeNodes> {
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
    if !value.len().is_multiple_of(4) {
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
        proto_change_pack_to_wire, proto_json_element_to_wire, proto_operation_to_wire,
        proto_tree_nodes_to_wire, snapshot_from_bytes, snapshot_root_to_bytes, to_change_pack,
        to_schema_rules, to_time_ticket, to_version_vector, wire_change_pack_to_proto,
        wire_change_to_proto, wire_json_element_to_proto, wire_operation_to_proto,
        wire_tree_nodes_to_proto, ProtocolError,
    };
    use crate::yorkie::v1::{self as api, json_element, operation::Body, ValueType};
    use prost::Message;
    use std::collections::BTreeMap;
    use std::error::Error;
    use yorkie_core::wire::{
        WireChange, WireChangeId, WireChangePack, WireJsonElement, WireJsonElementSimple,
        WireNodeAttr, WireOperation, WirePresence, WirePresenceChange, WireRestoreMode,
        WireRestoreSpan, WireRgaNode, WireRhtNode, WireTextNode, WireTextNodeId, WireTextNodePos,
        WireTreeNode, WireTreeNodeId, WireTreePos, WireTreeRestoreSpan, WireValueType,
    };
    use yorkie_core::{
        Checkpoint, Document, JsonValue, SchemaRule, TimeTicket, TreeNodeRule, VersionVector,
        YorkieError,
    };

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

    // JS `converter.toOperation/fromOperation`: a tree edit carries the
    // tickets its split issued as `split_tickets`, read back as sent.
    #[test]
    fn roundtrips_tree_edit_split_tickets() -> Result<(), Box<dyn Error>> {
        let pos = WireTreePos {
            parent_id: tree_node_id(1, 0),
            left_sibling_id: tree_node_id(2, 1),
        };
        let operation = WireOperation::TreeEdit {
            parent_created_at: ticket(1),
            from: pos.clone(),
            to: pos,
            contents: None,
            split_level: 2,
            executed_at: ticket(10),
            split_tickets: vec![
                TimeTicket::new(10, 3, "000000000000000000000001"),
                TimeTicket::new(10, 4, "000000000000000000000001"),
            ],
            restore_spans: Vec::new(),
            restore_mode: None,
            retombstone_spans: Vec::new(),
        };

        let proto = wire_operation_to_proto(&operation)?;
        let Some(Body::TreeEdit(tree_edit)) = &proto.body else {
            panic!("expected a tree edit");
        };
        assert_eq!(
            vec![3, 4],
            tree_edit
                .split_tickets
                .iter()
                .map(|ticket| ticket.delimiter)
                .collect::<Vec<_>>()
        );
        assert_eq!(operation, proto_operation_to_wire(&proto)?);
        Ok(())
    }

    // JS `tree_style_combined_test.ts` "keeps both halves across the
    // protobuf round trip": the reverse of a style that overwrote `b` and
    // added `c` restores the one and removes the other in one operation,
    // and both halves reach the peer.
    #[test]
    fn roundtrips_both_halves_of_a_tree_style() -> Result<(), Box<dyn Error>> {
        let pos = WireTreePos {
            parent_id: tree_node_id(1, 0),
            left_sibling_id: tree_node_id(2, 0),
        };
        let operation = WireOperation::TreeStyle {
            parent_created_at: ticket(1),
            from: pos.clone(),
            to: pos,
            attributes: BTreeMap::from([("b".to_owned(), "x".to_owned())]),
            attributes_to_remove: vec!["c".to_owned()],
            executed_at: ticket(10),
        };

        let proto = wire_operation_to_proto(&operation)?;
        let Some(Body::TreeStyle(tree_style)) = &proto.body else {
            panic!("expected a tree style");
        };
        assert_eq!(
            BTreeMap::from([("b".to_owned(), "x".to_owned())]),
            tree_style.attributes
        );
        assert_eq!(vec!["c".to_owned()], tree_style.attributes_to_remove);
        assert_eq!(operation, proto_operation_to_wire(&proto)?);
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
            snapshot_presences: BTreeMap::new(),
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
                presence_change: None,
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
                        restore_spans: Vec::new(),
                        restore_mode: None,
                        retombstone_spans: Vec::new(),
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
            snapshot_presences: BTreeMap::new(),
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
                                ..Default::default()
                            })),
                        }),
                    }],
                    created_at: Some(to_time_ticket(&TimeTicket::initial())?),
                    moved_at: None,
                    removed_at: None,
                    ..Default::default()
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
            ..Default::default()
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
                removal_events: Vec::new(),
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

    // JS `converter.toPresenceChange/fromPresence`: each presence value is its
    // JSON encoding on the wire.
    #[test]
    fn presence_change_roundtrips_through_change_pack() -> Result<(), Box<dyn Error>> {
        let mut doc = Document::new("doc-key");
        doc.set_actor("000000000000000000000001");
        doc.update_with_presence(|_, presence| {
            let mut data = yorkie_core::Presence::new();
            data.insert("name".to_owned(), serde_json::json!("a"));
            data.insert(
                "pos".to_owned(),
                serde_json::json!({"x": 1, "y": [2, null]}),
            );
            presence.set(data);
            Ok(())
        })?;
        doc.update_with_presence(|_, presence| {
            presence.clear();
            Ok(())
        })?;
        let pack = doc.create_change_pack();

        let proto = to_change_pack(&pack)?;
        let put = proto.changes[0].presence_change.as_ref().unwrap();
        assert_eq!(api::presence_change::ChangeType::Put as i32, put.r#type);
        let data = &put.presence.as_ref().unwrap().data;
        assert_eq!(r#""a""#, data["name"]);
        assert_eq!(r#"{"x":1,"y":[2,null]}"#, data["pos"]);
        let clear = proto.changes[1].presence_change.as_ref().unwrap();
        assert_eq!(api::presence_change::ChangeType::Clear as i32, clear.r#type);

        let decoded = decode_change_pack(&encode_change_pack(&pack)?)?;
        assert_eq!(pack, decoded);
        Ok(())
    }

    fn presence_pack_with(
        change_type: api::presence_change::ChangeType,
        data: &[(&str, &str)],
        with_presence: bool,
    ) -> api::ChangePack {
        let mut pack = to_change_pack(&Document::new("doc-key").create_change_pack()).unwrap();
        let mut change = to_change_pack(&{
            let mut doc = Document::new("doc-key");
            doc.update(|root| {
                root.set("k", 1i32)?;
                Ok(())
            })
            .unwrap();
            doc.create_change_pack()
        })
        .unwrap()
        .changes
        .remove(0);
        change.operations.clear();
        change.presence_change = Some(api::PresenceChange {
            r#type: change_type as i32,
            presence: with_presence.then(|| api::Presence {
                data: data
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }),
            removed_keys: Vec::new(),
        });
        pack.changes.push(change);
        pack.checkpoint = Some(api::Checkpoint {
            server_seq: 1,
            client_seq: 0,
        });
        pack
    }

    fn remote_presence_of(
        pack: &api::ChangePack,
        existing: Option<&str>,
    ) -> Option<yorkie_core::Presence> {
        let mut doc = Document::new("doc-key");
        let actor = yorkie_core::ActorId::from("000000000000000000000000");
        doc.set_actor(actor.clone());
        doc.apply_status(yorkie_core::DocStatus::Attached);
        if let Some(name) = existing {
            doc.update_with_presence(|_, presence| {
                let mut data = yorkie_core::Presence::new();
                data.insert("name".to_owned(), serde_json::json!(name));
                data.insert("x".to_owned(), serde_json::json!(1));
                presence.set(data);
                Ok(())
            })
            .unwrap();
        }
        // The change in the pack carries the document's own actor.
        doc.apply_change_pack(&from_change_pack(pack).unwrap())
            .unwrap();
        doc.presence(&actor)
    }

    // JS `fromPresenceChange`: UNSPECIFIED, DELETE and PATCH must decode, not
    // fail the pack (a failing stored change would wedge sync forever).
    #[test]
    fn decodes_unspecified_and_delete_presence_changes_as_empty_put() {
        use api::presence_change::ChangeType;
        for change_type in [ChangeType::Unspecified, ChangeType::Delete] {
            let proto = presence_pack_with(change_type, &[("a", "1")], true);
            let wire = proto_change_pack_to_wire(&proto).unwrap();
            assert_eq!(
                Some(WirePresenceChange::Put(WirePresence::new())),
                wire.changes[0].presence_change
            );
            let got = remote_presence_of(&proto, Some("n")).unwrap();
            assert!(
                got.is_empty(),
                "{change_type:?} should store an empty presence"
            );
        }
    }

    #[test]
    fn decodes_patch_presence_change_and_merges_onto_existing_presence() {
        use api::presence_change::ChangeType;
        let proto = presence_pack_with(ChangeType::Patch, &[("x", "2"), ("y", "\"z\"")], true);
        let wire = proto_change_pack_to_wire(&proto).unwrap();
        assert!(matches!(
            wire.changes[0].presence_change,
            Some(WirePresenceChange::Patch(_))
        ));
        // Encoding back keeps PATCH, so the round trip is lossless.
        assert_eq!(
            proto.changes[0].presence_change,
            to_change_pack(&from_change_pack(&proto).unwrap())
                .unwrap()
                .changes[0]
                .presence_change
        );

        let got = remote_presence_of(&proto, Some("n")).unwrap();
        assert_eq!(serde_json::json!("n"), got["name"]);
        assert_eq!(serde_json::json!(2), got["x"]);
        assert_eq!(serde_json::json!("z"), got["y"]);

        // No base presence: the patch becomes the presence.
        let got = remote_presence_of(&proto, None).unwrap();
        assert_eq!(2, got.len());
    }

    #[test]
    fn decodes_patch_without_presence_as_empty_patch() {
        use api::presence_change::ChangeType;
        let proto = presence_pack_with(ChangeType::Patch, &[], false);
        let wire = proto_change_pack_to_wire(&proto).unwrap();
        assert_eq!(
            Some(WirePresenceChange::Patch(WirePresence::new())),
            wire.changes[0].presence_change
        );
        let got = remote_presence_of(&proto, Some("n")).unwrap();
        assert_eq!(2, got.len());
    }

    #[test]
    fn snapshot_restores_presences() -> Result<(), Box<dyn Error>> {
        let actor_a = "000000000000000000000002";
        let mut version_vector = VersionVector::new();
        version_vector.set(actor_a, 3);
        let mut presences = BTreeMap::new();
        presences.insert(
            actor_a.to_owned(),
            BTreeMap::from([("name".to_owned(), r#""b""#.to_owned())]),
        );
        let wire = WireChangePack {
            document_key: "doc-key".to_owned(),
            checkpoint: Checkpoint::new(7, 0),
            is_removed: false,
            changes: Vec::new(),
            snapshot: None,
            snapshot_root: Some(sample_snapshot_root()),
            snapshot_presences: presences.clone(),
            version_vector: Some(version_vector),
        };

        let proto = wire_change_pack_to_proto(&wire)?;
        assert_eq!(
            presences,
            proto_change_pack_to_wire(&proto)?.snapshot_presences
        );

        let mut doc = Document::new("doc-key");
        doc.apply_change_pack(&from_change_pack(&proto)?)?;
        let actor = yorkie_core::ActorId::from(actor_a);
        assert!(doc.has_presence(&actor));
        doc.add_online_client(actor.clone());
        let mut expected = yorkie_core::Presence::new();
        expected.insert("name".to_owned(), serde_json::json!("b"));
        assert_eq!(Some(expected), doc.presence(&actor));
        Ok(())
    }

    // --- Text restore spans (JS restore_converter_test.ts) ---

    /// JS `restore_converter_test.ts` "round-trips a restore operation over
    /// the wire".
    #[test]
    fn roundtrips_a_restore_operation_over_the_wire() -> Result<(), Box<dyn Error>> {
        let operation = identity_edit(
            vec![restore_span(4, 6, "45"), restore_span(2, 8, "234567")],
            WireRestoreMode::Restore,
            Vec::new(),
        );

        let decoded = operation_through_bytes(&operation)?;
        assert_eq!(operation, decoded);
        let WireOperation::Edit {
            restore_spans,
            restore_mode,
            ..
        } = decoded
        else {
            panic!("expected an edit");
        };
        assert_eq!(Some(WireRestoreMode::Restore), restore_mode);
        assert_eq!(2, restore_spans.len());
        assert_eq!((4, 6, "45"), span_parts(&restore_spans[0]));
        assert_eq!("234567", restore_spans[1].content);
        assert_eq!(ticket(1), restore_spans[0].created_at);
        Ok(())
    }

    /// JS `restore_converter_test.ts` "round-trips a retombstone operation".
    #[test]
    fn roundtrips_a_retombstone_operation() -> Result<(), Box<dyn Error>> {
        let operation = identity_edit(
            vec![restore_span(4, 6, "45")],
            WireRestoreMode::Retombstone,
            Vec::new(),
        );

        let proto = wire_operation_to_proto(&operation)?;
        let Some(Body::Edit(edit)) = &proto.body else {
            panic!("expected an edit");
        };
        assert_eq!(api::RestoreMode::Retombstone as i32, edit.restore_mode);
        assert_eq!(operation, operation_through_bytes(&operation)?);
        Ok(())
    }

    /// JS `restore_converter_test.ts` "round-trips the companion
    /// retombstone_spans of a replace reverse": the reverse of a replace
    /// revives the removed content and re-removes the inserted content, both
    /// by identity, and both span lists must survive the wire.
    #[test]
    fn roundtrips_the_companion_retombstone_spans_of_a_replace_reverse(
    ) -> Result<(), Box<dyn Error>> {
        let operation = identity_edit(
            vec![restore_span(2, 4, "CD")],
            WireRestoreMode::Restore,
            vec![restore_span(0, 2, "12")],
        );

        let decoded = operation_through_bytes(&operation)?;
        assert_eq!(operation, decoded);
        let WireOperation::Edit {
            restore_spans,
            retombstone_spans,
            ..
        } = decoded
        else {
            panic!("expected an edit");
        };
        assert_eq!("CD", restore_spans[0].content);
        assert_eq!(1, retombstone_spans.len());
        assert_eq!("12", retombstone_spans[0].content);
        assert_eq!(ticket(1), retombstone_spans[0].created_at);
        Ok(())
    }

    /// JS `restore_converter_test.ts` "keeps span attributes, stamped with
    /// executedAt": the attribute map crosses the wire as it was. The stamp
    /// is the domain decoder's (`yorkie-core` wire test
    /// `stamps_span_attributes_with_executed_at`).
    #[test]
    fn keeps_span_attributes() -> Result<(), Box<dyn Error>> {
        let mut styled = restore_span(0, 1, "X");
        styled.attributes.insert("b".to_owned(), "1".to_owned());
        let operation = identity_edit(Vec::new(), WireRestoreMode::Restore, vec![styled]);

        let WireOperation::Edit {
            retombstone_spans, ..
        } = operation_through_bytes(&operation)?
        else {
            panic!("expected an edit");
        };
        assert_eq!(
            BTreeMap::from([("b".to_owned(), "1".to_owned())]),
            retombstone_spans[0].attributes
        );
        Ok(())
    }

    /// JS `restore_converter_test.ts` "rejects a malformed span": a span is
    /// untrusted input from any peer, so a missing `created_at`, a negative
    /// start, an inverted interval, or an interval that does not cover the
    /// content in UTF-16 code units rejects the whole operation at the
    /// boundary instead of failing deep in the restore path.
    #[test]
    fn rejects_a_malformed_span() -> Result<(), Box<dyn Error>> {
        let decode_with = |corrupt: &dyn Fn(&mut api::RestoreSpan)| -> Result<_, Box<dyn Error>> {
            let operation = identity_edit(
                vec![restore_span(4, 6, "45")],
                WireRestoreMode::Restore,
                Vec::new(),
            );
            let mut proto = wire_operation_to_proto(&operation)?;
            let Some(Body::Edit(edit)) = &mut proto.body else {
                panic!("expected an edit");
            };
            corrupt(&mut edit.restore_spans[0]);
            let bytes = proto.encode_to_vec();
            Ok(proto_operation_to_wire(&api::Operation::decode(
                bytes.as_slice(),
            )?))
        };
        let rejects = |result: super::Result<WireOperation>| {
            matches!(
                result,
                Err(ProtocolError::Core(YorkieError::InvalidArgument(_)))
            )
        };

        assert!(decode_with(&|_| {})?.is_ok());
        assert!(rejects(decode_with(&|span| span.created_at = None)?));
        assert!(rejects(decode_with(&|span| {
            span.start = -2;
            span.end = 0;
        })?));
        assert!(rejects(decode_with(&|span| {
            span.start = 6;
            span.end = 4;
        })?));
        assert!(rejects(decode_with(&|span| span.end = 9)?));
        // An emoji is two UTF-16 code units and four UTF-8 bytes.
        assert!(decode_with(&|span| {
            span.content = "\u{1F600}".to_owned();
        })?
        .is_ok());
        assert!(rejects(decode_with(&|span| {
            span.content = "\u{1F600}".to_owned();
            span.end = 8;
        })?));
        // The retombstone list is checked the same way.
        let operation = identity_edit(
            Vec::new(),
            WireRestoreMode::Restore,
            vec![restore_span(4, 6, "45")],
        );
        let mut proto = wire_operation_to_proto(&operation)?;
        let Some(Body::Edit(edit)) = &mut proto.body else {
            panic!("expected an edit");
        };
        edit.retombstone_spans[0].end = 9;
        assert!(rejects(proto_operation_to_wire(&proto)));
        Ok(())
    }

    /// A malformed span fails the change pack that carries it, as JS throws
    /// from `fromOperation`.
    #[test]
    fn rejects_a_change_pack_with_a_malformed_span() -> Result<(), Box<dyn Error>> {
        let mut pack = api::ChangePack {
            document_key: "doc-key".to_owned(),
            checkpoint: Some(api::Checkpoint {
                server_seq: 0,
                client_seq: 1,
            }),
            changes: vec![wire_change_to_proto(&WireChange {
                id: wire_change_id(1, 4),
                message: None,
                presence_change: None,
                operations: vec![identity_edit(
                    vec![restore_span(4, 6, "45")],
                    WireRestoreMode::Restore,
                    Vec::new(),
                )],
            })?],
            ..Default::default()
        };
        let Some(Body::Edit(edit)) = &mut pack.changes[0].operations[0].body else {
            panic!("expected an edit");
        };
        edit.restore_spans[0].end = 9;

        assert!(matches!(
            decode_change_pack(&pack.encode_to_vec()),
            Err(ProtocolError::Core(YorkieError::InvalidArgument(_)))
        ));
        Ok(())
    }

    /// JS `restore_converter_test.ts` "leaves ordinary edits without a
    /// restore payload": no span list and no mode on the wire, so the bytes
    /// are the ones written before restore spans existed.
    #[test]
    fn leaves_ordinary_edits_without_a_restore_payload() -> Result<(), Box<dyn Error>> {
        let operation = WireOperation::Edit {
            parent_created_at: TimeTicket::initial(),
            from: text_pos(ticket(1), 0, 0),
            to: text_pos(ticket(1), 0, 0),
            content: "hi".to_owned(),
            attributes: BTreeMap::new(),
            executed_at: ticket(4),
            restore_spans: Vec::new(),
            restore_mode: None,
            retombstone_spans: Vec::new(),
        };

        let proto = wire_operation_to_proto(&operation)?;
        let Some(Body::Edit(edit)) = &proto.body else {
            panic!("expected an edit");
        };
        assert!(edit.restore_spans.is_empty());
        assert!(edit.retombstone_spans.is_empty());
        assert_eq!(0, edit.restore_mode);
        assert_eq!(operation, operation_through_bytes(&operation)?);
        Ok(())
    }

    /// JS `toOperation` writes `restore_mode` only with a span, so a mode
    /// with no span leaves no trace in the bytes.
    #[test]
    fn writes_no_restore_mode_without_a_span() -> Result<(), Box<dyn Error>> {
        let operation = identity_edit(Vec::new(), WireRestoreMode::Retombstone, Vec::new());

        let proto = wire_operation_to_proto(&operation)?;
        let Some(Body::Edit(edit)) = &proto.body else {
            panic!("expected an edit");
        };
        assert_eq!(0, edit.restore_mode);
        assert!(matches!(
            proto_operation_to_wire(&proto)?,
            WireOperation::Edit {
                restore_mode: None,
                ..
            }
        ));
        Ok(())
    }

    /// JS `fromOperation` reads any mode other than RETOMBSTONE as restore.
    #[test]
    fn reads_an_unspecified_mode_with_spans_as_restore() -> Result<(), Box<dyn Error>> {
        let operation = identity_edit(
            vec![restore_span(4, 6, "45")],
            WireRestoreMode::Restore,
            Vec::new(),
        );
        let mut proto = wire_operation_to_proto(&operation)?;
        let Some(Body::Edit(edit)) = &mut proto.body else {
            panic!("expected an edit");
        };
        edit.restore_mode = api::RestoreMode::Unspecified as i32;

        assert_eq!(operation, proto_operation_to_wire(&proto)?);
        Ok(())
    }

    /// JS `restore_converter_test.ts` "decodes to a harmless no-op for peers
    /// that ignore restore fields": an undo by identity carries its content
    /// only in its spans, and its positional part is a zero-width,
    /// empty-content edit, so a peer that drops the restore fields has no
    /// inline content to re-insert and no range to delete. A peer that reads
    /// them restores the text.
    #[test]
    fn decodes_to_a_harmless_no_op_for_peers_that_ignore_restore_fields(
    ) -> Result<(), Box<dyn Error>> {
        let source = deleted_then(|doc| doc.history_mut().undo())?;
        let restored = r#"{"t":[{"val":"a"},{"val":"b"},{"val":"c"}]}"#;
        assert_eq!(restored, source.to_sorted_json());

        let proto = to_change_pack(&source.create_change_pack())?;
        let undo = &proto.changes.last().unwrap().operations[0];
        let Some(Body::Edit(edit)) = &undo.body else {
            panic!("expected the undo edit");
        };
        assert_eq!("", edit.content, "no inline content to re-insert");

        let WireOperation::Edit {
            from,
            to,
            content,
            restore_spans,
            restore_mode,
            ..
        } = operation_through_bytes(&proto_operation_to_wire(undo)?)?
        else {
            panic!("expected an edit");
        };
        assert_eq!(from, to, "zero-width, so nothing to delete either");
        assert_eq!("", content);
        assert_eq!(Some(WireRestoreMode::Restore), restore_mode);
        assert_eq!(1, restore_spans.len());

        let mut target = Document::new("doc-key");
        target.apply_change_pack(&decode_change_pack(&proto.encode_to_vec())?)?;
        assert_eq!(restored, target.to_sorted_json());
        Ok(())
    }

    /// The redo of an undo by identity re-removes by identity on the peer.
    #[test]
    fn carries_an_identity_redo_through_change_pack_bytes() -> Result<(), Box<dyn Error>> {
        let source = deleted_then(|doc| {
            doc.history_mut().undo()?;
            doc.history_mut().redo()
        })?;
        assert_eq!(
            r#"{"t":[{"val":"a"},{"val":"c"}]}"#,
            source.to_sorted_json()
        );

        let proto = to_change_pack(&source.create_change_pack())?;
        let Some(Body::Edit(redo)) = &proto.changes.last().unwrap().operations[0].body else {
            panic!("expected the redo edit");
        };
        assert_eq!(api::RestoreMode::Retombstone as i32, redo.restore_mode);

        let mut target = Document::new("doc-key");
        target.apply_change_pack(&decode_change_pack(&proto.encode_to_vec())?)?;
        assert_eq!(source.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    /// A document whose text `abc` lost its `b`, then `then` ran on it.
    fn deleted_then(
        then: impl FnOnce(&mut Document) -> yorkie_core::Result<()>,
    ) -> Result<Document, Box<dyn Error>> {
        let mut doc = Document::new("doc-key");
        doc.set_actor(A1);
        doc.update(|root| {
            root.set_text("t")?.edit(0, 0, "abc", None)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_text_mut("t")?.edit(1, 2, "", None)?;
            Ok(())
        })?;
        then(&mut doc)?;
        Ok(doc)
    }

    // --- Tree restore spans (JS tree_restore_converter_test.ts) ---

    /// The JS test's element span: `<p bold="true">` with all three anchors.
    fn tree_element_span() -> WireTreeRestoreSpan {
        WireTreeRestoreSpan {
            id: tree_node_id(1, 2),
            node_type: "p".to_owned(),
            is_text: false,
            length: 0,
            value: String::new(),
            attributes: BTreeMap::from([(
                "bold".to_owned(),
                WireNodeAttr {
                    value: "true".to_owned(),
                    updated_at: ticket(1),
                    is_removed: false,
                },
            )]),
            parent_id: Some(tree_node_id(1, 0)),
            left_sibling_id: Some(tree_node_id(1, 1)),
            right_sibling_id: Some(tree_node_id(1, 3)),
        }
    }

    /// The JS test's text span: `hello` under the element, its only anchor
    /// the parent.
    fn tree_text_span() -> WireTreeRestoreSpan {
        WireTreeRestoreSpan {
            id: tree_node_id(1, 5),
            node_type: "text".to_owned(),
            is_text: true,
            length: 5,
            value: "hello".to_owned(),
            attributes: BTreeMap::new(),
            parent_id: Some(tree_node_id(1, 2)),
            left_sibling_id: None,
            right_sibling_id: None,
        }
    }

    fn tree_edit(
        restore_spans: Vec<WireTreeRestoreSpan>,
        restore_mode: Option<WireRestoreMode>,
        retombstone_spans: Vec<WireTreeRestoreSpan>,
    ) -> WireOperation {
        let pos = WireTreePos {
            parent_id: tree_node_id(1, 0),
            left_sibling_id: tree_node_id(1, 0),
        };
        WireOperation::TreeEdit {
            parent_created_at: TimeTicket::initial(),
            from: pos.clone(),
            to: pos,
            contents: None,
            split_level: 0,
            executed_at: ticket(4),
            split_tickets: Vec::new(),
            restore_spans,
            restore_mode,
            retombstone_spans,
        }
    }

    fn tree_spans_of(
        operation: WireOperation,
    ) -> (
        Option<WireRestoreMode>,
        Vec<WireTreeRestoreSpan>,
        Vec<WireTreeRestoreSpan>,
    ) {
        let WireOperation::TreeEdit {
            restore_spans,
            restore_mode,
            retombstone_spans,
            ..
        } = operation
        else {
            panic!("expected a tree edit");
        };
        (restore_mode, restore_spans, retombstone_spans)
    }

    /// JS `tree_restore_converter_test.ts` "round-trips element + text
    /// restore spans with anchors and attrs".
    #[test]
    fn roundtrips_element_and_text_tree_restore_spans() -> Result<(), Box<dyn Error>> {
        let operation = tree_edit(
            vec![tree_element_span(), tree_text_span()],
            Some(WireRestoreMode::Restore),
            Vec::new(),
        );

        let decoded = operation_through_bytes(&operation)?;
        assert_eq!(operation, decoded);
        let (mode, spans, retombstone) = tree_spans_of(decoded);
        assert_eq!(Some(WireRestoreMode::Restore), mode);
        assert!(retombstone.is_empty());
        let [element, text] = &spans[..] else {
            panic!("expected two spans, got {spans:?}");
        };
        // Identity, the tree anchors and the attribute snapshot survive.
        assert_eq!(tree_node_id(1, 2), element.id);
        assert_eq!(("p", false), (element.node_type.as_str(), element.is_text));
        assert_eq!(Some(tree_node_id(1, 0)), element.parent_id);
        assert_eq!(Some(tree_node_id(1, 1)), element.left_sibling_id);
        assert_eq!(Some(tree_node_id(1, 3)), element.right_sibling_id);
        assert_eq!("true", element.attributes["bold"].value);
        // Identity, value and length survive.
        assert_eq!(tree_node_id(1, 5), text.id);
        assert!(text.is_text);
        assert_eq!((5, "hello"), (text.length, text.value.as_str()));
        assert_eq!(Some(tree_node_id(1, 2)), text.parent_id);
        Ok(())
    }

    /// JS `tree_restore_converter_test.ts` "round-trips a pure-insert
    /// reverse (retombstone spans only)": the reverse of an insert carries
    /// its nodes only in `retombstone_spans`.
    #[test]
    fn roundtrips_a_pure_insert_tree_reverse() -> Result<(), Box<dyn Error>> {
        let operation = tree_edit(
            Vec::new(),
            Some(WireRestoreMode::Restore),
            vec![tree_element_span(), tree_text_span()],
        );

        let decoded = operation_through_bytes(&operation)?;
        assert_eq!(operation, decoded);
        let (mode, spans, retombstone) = tree_spans_of(decoded);
        assert_eq!(Some(WireRestoreMode::Restore), mode);
        assert!(spans.is_empty());
        assert_eq!(2, retombstone.len());
        assert_eq!(tree_node_id(1, 2), retombstone[0].id);
        assert_eq!("hello", retombstone[1].value);
        Ok(())
    }

    /// JS `tree_restore_converter_test.ts` "round-trips a redo (retombstone
    /// direction)".
    #[test]
    fn roundtrips_a_tree_redo() -> Result<(), Box<dyn Error>> {
        let operation = tree_edit(
            vec![tree_text_span()],
            Some(WireRestoreMode::Retombstone),
            vec![tree_element_span()],
        );

        let decoded = operation_through_bytes(&operation)?;
        assert_eq!(operation, decoded);
        let (mode, spans, retombstone) = tree_spans_of(decoded);
        assert_eq!(Some(WireRestoreMode::Retombstone), mode);
        assert_eq!((1, 1), (spans.len(), retombstone.len()));
        Ok(())
    }

    /// JS `tree_restore_converter_test.ts` "leaves ordinary tree edits
    /// without a restore payload": no span list and no mode on the wire, so
    /// the bytes are the ones written before restore spans existed, and a
    /// mode with no span leaves no trace.
    #[test]
    fn leaves_ordinary_tree_edits_without_a_restore_payload() -> Result<(), Box<dyn Error>> {
        for mode in [None, Some(WireRestoreMode::Retombstone)] {
            let proto = wire_operation_to_proto(&tree_edit(Vec::new(), mode, Vec::new()))?;
            let Some(Body::TreeEdit(edit)) = &proto.body else {
                panic!("expected a tree edit");
            };
            assert!(edit.restore_spans.is_empty());
            assert!(edit.retombstone_spans.is_empty());
            assert_eq!(0, edit.restore_mode, "{mode:?}");
            assert_eq!(
                (None, Vec::new(), Vec::new()),
                tree_spans_of(proto_operation_to_wire(&proto)?)
            );
        }
        Ok(())
    }

    /// JS `fromOperation` reads any mode but RETOMBSTONE as restore, so a
    /// peer that leaves `restore_mode` at its proto default still sends an
    /// identity edit, which Rust tells apart by its mode.
    #[test]
    fn reads_a_tree_identity_edit_without_a_mode_as_a_restore() -> Result<(), Box<dyn Error>> {
        let operation = tree_edit(
            vec![tree_text_span()],
            Some(WireRestoreMode::Restore),
            Vec::new(),
        );
        let mut proto = wire_operation_to_proto(&operation)?;
        let Some(Body::TreeEdit(edit)) = &mut proto.body else {
            panic!("expected a tree edit");
        };
        edit.restore_mode = api::RestoreMode::Unspecified as i32;
        let bytes = proto.encode_to_vec();

        assert_eq!(
            operation,
            proto_operation_to_wire(&api::Operation::decode(bytes.as_slice())?)?
        );
        Ok(())
    }

    /// JS `tree_restore_converter_test.ts` "rejects a span with a missing
    /// created_at", every row: a span addresses nodes by insertion identity,
    /// so a missing `created_at` on its id or an anchor, or a missing
    /// `updated_at` on an attribute, rejects the whole operation with
    /// `ErrInvalidArgument`. A disagreeing kind or length still decodes.
    #[test]
    fn rejects_a_tree_span_with_a_missing_timestamp() -> Result<(), Box<dyn Error>> {
        type Corrupt = fn(&mut api::TreeRestoreSpan);
        let decode_with = |corrupt: Corrupt| -> Result<_, Box<dyn Error>> {
            let operation = tree_edit(
                vec![tree_element_span()],
                Some(WireRestoreMode::Restore),
                Vec::new(),
            );
            let mut proto = wire_operation_to_proto(&operation)?;
            let Some(Body::TreeEdit(edit)) = &mut proto.body else {
                panic!("expected a tree edit");
            };
            corrupt(&mut edit.restore_spans[0]);
            let bytes = proto.encode_to_vec();
            Ok(proto_operation_to_wire(&api::Operation::decode(
                bytes.as_slice(),
            )?))
        };

        let rows: [(&str, Corrupt); 6] = [
            ("on the span id", |span| {
                span.id.as_mut().unwrap().created_at = None;
            }),
            ("without a span id", |span| span.id = None),
            ("on the parent id", |span| {
                span.parent_id.as_mut().unwrap().created_at = None;
            }),
            ("on the left sibling id", |span| {
                span.left_sibling_id.as_mut().unwrap().created_at = None;
            }),
            ("on the right sibling id", |span| {
                span.right_sibling_id.as_mut().unwrap().created_at = None;
            }),
            ("on an attribute updatedAt", |span| {
                span.attributes.get_mut("bold").unwrap().updated_at = None;
            }),
        ];
        for (name, corrupt) in rows {
            assert!(
                matches!(
                    decode_with(corrupt)?,
                    Err(ProtocolError::Core(YorkieError::InvalidArgument(_)))
                ),
                "{name}"
            );
        }

        let decodes: [(&str, Corrupt); 4] = [
            ("as written", |_| {}),
            ("with a kind that disagrees", |span| span.is_text = true),
            ("with a length that disagrees", |span| span.length = 7),
            ("without anchors", |span| {
                span.parent_id = None;
                span.left_sibling_id = None;
                span.right_sibling_id = None;
            }),
        ];
        for (name, corrupt) in decodes {
            assert!(decode_with(corrupt)?.is_ok(), "{name}");
        }
        Ok(())
    }

    // --- Text removal ledger in snapshots ---

    /// A Text node without a ledger encodes to the snapshot bytes it did
    /// before `removal_events` existed. The vector was captured from `main`
    /// (a3ad7b8) before the change.
    #[test]
    fn encodes_a_text_node_without_a_ledger_to_the_bytes_it_did_before(
    ) -> Result<(), Box<dyn Error>> {
        const BEFORE: [u8; 201] = [
            10, 198, 1, 10, 195, 1, 10, 176, 1, 10, 1, 116, 18, 170, 1, 42, 167, 1, 10, 52, 10, 18,
            10, 16, 8, 2, 26, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 18, 2, 97, 98, 42, 26, 10, 1,
            98, 18, 21, 10, 1, 49, 18, 16, 8, 3, 26, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 10,
            64, 10, 20, 10, 16, 8, 2, 26, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 16, 2, 18, 2, 99,
            100, 26, 16, 8, 5, 26, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 34, 18, 10, 16, 8, 2,
            26, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 10, 27, 10, 18, 10, 16, 8, 4, 26, 12, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 18, 5, 240, 159, 152, 128, 120, 18, 16, 8, 1, 26, 12, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 18, 14, 26, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let root = text_snapshot_root(ledger_text_nodes(Vec::new()));
        assert_eq!(
            BEFORE.to_vec(),
            snapshot_root_to_bytes(&root, &BTreeMap::new())?
        );
        Ok(())
    }

    /// JS `toTextNodes`/`fromTextNode`: a ledger beyond `removed_at` survives
    /// snapshot bytes in ticket order, and the snapshot applies.
    #[test]
    fn roundtrips_a_ledgered_text_node_through_snapshot_bytes() -> Result<(), Box<dyn Error>> {
        // `cd` was removed by A at 3, restored by A at 4, removed by B at 5.
        let events = vec![actor_ticket(3, A1), actor_ticket(4, A1)];
        let root = text_snapshot_root(ledger_text_nodes(events.clone()));

        let bytes = snapshot_root_to_bytes(&root, &BTreeMap::new())?;
        let snapshot = api::Snapshot::decode(bytes.as_slice())?;
        let Some(json_element::Body::JsonObject(object)) = &snapshot.root.as_ref().unwrap().body
        else {
            panic!("expected an object");
        };
        let Some(json_element::Body::Text(text)) = &object.nodes[0].element.as_ref().unwrap().body
        else {
            panic!("expected a text");
        };
        assert_eq!(
            vec![3, 4],
            text.nodes[1]
                .removal_events
                .iter()
                .map(|ticket| ticket.lamport)
                .collect::<Vec<_>>()
        );
        assert!(text.nodes[0].removal_events.is_empty());

        let (decoded, _) = snapshot_from_bytes(&bytes)?;
        assert_eq!(root, decoded);

        let mut doc = Document::new("doc-key");
        doc.apply_change_pack(&decode_change_pack(&snapshot_pack(bytes).encode_to_vec())?)?;
        assert_eq!(
            r#"{"t":[{"attrs":{"b":1},"val":"ab"},{"val":"😀x"}]}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    /// JS `fromTextNode`: a ledger that breaks the rules is dropped, the node
    /// keeps `removed_at` alone, and the snapshot still applies.
    #[test]
    fn drops_a_malformed_ledger_and_still_applies_the_snapshot() -> Result<(), Box<dyn Error>> {
        for events in [
            // repeats removed_at
            vec![actor_ticket(5, A2)],
            // an uncancelled removal later than removed_at, which it would replace
            vec![actor_ticket(9, A1)],
        ] {
            let root = text_snapshot_root(ledger_text_nodes(events));
            let bytes = snapshot_root_to_bytes(&root, &BTreeMap::new())?;

            let mut doc = Document::new("doc-key");
            doc.apply_change_pack(&decode_change_pack(&snapshot_pack(bytes).encode_to_vec())?)?;
            assert_eq!(
                r#"{"t":[{"attrs":{"b":1},"val":"ab"},{"val":"😀x"}]}"#,
                doc.to_sorted_json()
            );
        }
        Ok(())
    }

    const A1: &str = "000000000000000000000001";
    const A2: &str = "000000000000000000000002";

    fn actor_ticket(lamport: i64, actor: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor)
    }

    /// `ab` (styled), `cd` removed by A2 at 5 with `removal_events`, and an
    /// emoji run by A2.
    fn ledger_text_nodes(removal_events: Vec<TimeTicket>) -> Vec<WireTextNode> {
        let id = |lamport, actor, offset| WireTextNodeId {
            created_at: actor_ticket(lamport, actor),
            offset,
        };
        vec![
            WireTextNode {
                id: id(2, A1, 0),
                value: "ab".to_owned(),
                removed_at: None,
                ins_prev_id: None,
                attributes: BTreeMap::from([(
                    "b".to_owned(),
                    WireNodeAttr {
                        value: "1".to_owned(),
                        updated_at: actor_ticket(3, A1),
                        is_removed: false,
                    },
                )]),
                removal_events: Vec::new(),
            },
            WireTextNode {
                id: id(2, A1, 2),
                value: "cd".to_owned(),
                removed_at: Some(actor_ticket(5, A2)),
                ins_prev_id: Some(id(2, A1, 0)),
                attributes: BTreeMap::new(),
                removal_events,
            },
            WireTextNode {
                id: id(4, A2, 0),
                value: "\u{1F600}x".to_owned(),
                removed_at: None,
                ins_prev_id: None,
                attributes: BTreeMap::new(),
                removal_events: Vec::new(),
            },
        ]
    }

    fn text_snapshot_root(nodes: Vec<WireTextNode>) -> WireJsonElement {
        WireJsonElement::Object {
            nodes: vec![rht_node(
                "t",
                WireJsonElement::Text {
                    nodes,
                    created_at: actor_ticket(1, A1),
                    moved_at: None,
                    removed_at: None,
                },
            )],
            created_at: TimeTicket::initial(),
            moved_at: None,
            removed_at: None,
        }
    }

    fn snapshot_pack(snapshot: Vec<u8>) -> api::ChangePack {
        let mut version_vector = VersionVector::new();
        version_vector.set(A1, 5);
        version_vector.set(A2, 5);
        api::ChangePack {
            document_key: "doc-key".to_owned(),
            checkpoint: Some(api::Checkpoint {
                server_seq: 7,
                client_seq: 0,
            }),
            snapshot,
            version_vector: Some(to_version_vector(&version_vector).unwrap()),
            ..Default::default()
        }
    }

    fn identity_edit(
        restore_spans: Vec<WireRestoreSpan>,
        mode: WireRestoreMode,
        retombstone_spans: Vec<WireRestoreSpan>,
    ) -> WireOperation {
        WireOperation::Edit {
            parent_created_at: TimeTicket::initial(),
            from: text_pos(ticket(1), 0, 0),
            to: text_pos(ticket(1), 0, 0),
            content: String::new(),
            attributes: BTreeMap::new(),
            executed_at: ticket(4),
            restore_spans,
            restore_mode: Some(mode),
            retombstone_spans,
        }
    }

    fn restore_span(start: usize, end: usize, content: &str) -> WireRestoreSpan {
        WireRestoreSpan {
            created_at: ticket(1),
            start,
            end,
            content: content.to_owned(),
            attributes: BTreeMap::new(),
        }
    }

    fn span_parts(span: &WireRestoreSpan) -> (usize, usize, &str) {
        (span.start, span.end, span.content.as_str())
    }

    fn operation_through_bytes(operation: &WireOperation) -> super::Result<WireOperation> {
        let bytes = wire_operation_to_proto(operation)?.encode_to_vec();
        proto_operation_to_wire(&api::Operation::decode(bytes.as_slice())?)
    }

    // --- Text undo ledger replays over protobuf bytes ---
    //
    // JS `text_undo_removal_ledger_test.ts` passes every change pack through
    // protobuf; `yorkie-core` replays it over the wire projection only. These
    // replay its key cases over the bytes.

    /// JS `send`: delivers `from`'s pending changes to each of `to` as
    /// change pack bytes, without acking them at `from`.
    fn send_bytes(from: &Document, to: &mut [&mut Document]) -> Result<(), Box<dyn Error>> {
        let mut proto = to_change_pack(&from.create_change_pack())?;
        proto.checkpoint = Some(api::Checkpoint {
            server_seq: 0,
            client_seq: 0,
        });
        proto.version_vector = Some(to_version_vector(&VersionVector::new())?);
        let bytes = proto.encode_to_vec();
        for doc in to {
            doc.apply_change_pack(&decode_change_pack(&bytes)?)?;
        }
        Ok(())
    }

    /// JS `ack`: drops the changes `doc` has already sent.
    fn ack_bytes(doc: &mut Document) -> Result<(), Box<dyn Error>> {
        let sent = to_change_pack(&doc.create_change_pack())?;
        let last_seq = sent
            .changes
            .last()
            .and_then(|change| change.id.as_ref())
            .map_or(0, |id| id.client_seq);
        let proto = api::ChangePack {
            document_key: doc.key().to_owned(),
            checkpoint: Some(api::Checkpoint {
                server_seq: 0,
                client_seq: last_seq,
            }),
            version_vector: Some(to_version_vector(&VersionVector::new())?),
            ..Default::default()
        };
        doc.apply_change_pack(&decode_change_pack(&proto.encode_to_vec())?)?;
        Ok(())
    }

    fn exchange_bytes(a: &mut Document, b: &mut Document) -> Result<(), Box<dyn Error>> {
        send_bytes(a, &mut [b])?;
        send_bytes(b, &mut [a])?;
        ack_bytes(a)?;
        ack_bytes(b)
    }

    fn ledger_replicas(content: &str) -> Result<(Document, Document), Box<dyn Error>> {
        let mut a = Document::new("ledger");
        let mut b = Document::new("ledger");
        a.set_actor(A1);
        b.set_actor(A2);
        a.update(|root| root.set_text("t")?.edit(0, 0, content, None).map(|_| ()))?;
        exchange_bytes(&mut a, &mut b)?;
        Ok((a, b))
    }

    fn delete_text(doc: &mut Document, from: usize, to: usize) -> yorkie_core::Result<()> {
        doc.update(|root| root.get_text_mut("t")?.edit(from, to, "", None).map(|_| ()))
    }

    fn text_of(doc: &Document) -> String {
        match doc.get_root().get("t") {
            Some(JsonValue::Text(text)) => text.to_string(),
            other => panic!("expected a text at t, got {other:?}"),
        }
    }

    /// JS `does not remove text a concurrent undo revives (yorkie#2237)`,
    /// every change as protobuf bytes.
    #[test]
    fn replays_yorkie_2237_over_proto_bytes() -> Result<(), Box<dyn Error>> {
        let (mut d1, mut d2) = ledger_replicas("efgh")?;
        delete_text(&mut d2, 1, 2)?;
        exchange_bytes(&mut d1, &mut d2)?;
        assert_eq!("egh", text_of(&d1));

        // d1 deletes `g` while d2 undoes its deletion of `f`.
        delete_text(&mut d1, 1, 2)?;
        d2.history_mut().undo()?;
        exchange_bytes(&mut d1, &mut d2)?;

        // d1 saw `f` removed, so its deletion leaves the revived `f` alone.
        assert_eq!("efh", text_of(&d1));
        assert_eq!("efh", text_of(&d2));
        assert_eq!(d1.to_sorted_json(), d2.to_sorted_json());
        Ok(())
    }

    /// JS `keeps the ledger through a snapshot`, every change and the
    /// snapshot as protobuf bytes.
    ///
    /// The protocol crate cannot read a document's root, so the snapshot of
    /// `b` is written out here: `abcdefgh` inserted by A1 at 1, `d` removed
    /// by A2 at 3 and by A1 at 2 (its `removal_events`, as `removed_at` is
    /// the later A2 removal), `efgh` removed by A1 at 2. The control run
    /// drops `removal_events` from the bytes: `c` then forgets A1's part and
    /// revives `d` on b's undo, so the ledger crossing the bytes is what
    /// keeps the replicas together.
    #[test]
    fn replays_keeps_the_ledger_through_a_snapshot_over_proto_bytes() -> Result<(), Box<dyn Error>>
    {
        for with_ledger in [true, false] {
            let (mut a, mut b) = ledger_replicas("abcdefgh")?;
            delete_text(&mut b, 3, 4)?;
            delete_text(&mut a, 3, 8)?;
            send_bytes(&a, &mut [&mut b])?;

            // c loads b's snapshot, where `d` is removed by both deletions;
            // it must not forget a's part when b's undo arrives.
            let mut snapshot_vector = VersionVector::new();
            snapshot_vector.set(A1, 2);
            snapshot_vector.set(A2, 4);
            assert_eq!(&snapshot_vector, b.version_vector());
            let root = ledger_snapshot_root(with_ledger);
            let pack = api::ChangePack {
                document_key: "ledger".to_owned(),
                checkpoint: Some(api::Checkpoint {
                    server_seq: 1,
                    client_seq: 0,
                }),
                snapshot: snapshot_root_to_bytes(&root, &BTreeMap::new())?,
                version_vector: Some(to_version_vector(&snapshot_vector)?),
                ..Default::default()
            };
            let mut c = Document::new("ledger");
            c.set_actor("000000000000000000000003");
            c.apply_change_pack(&decode_change_pack(&pack.encode_to_vec())?)?;
            assert_eq!(b.to_sorted_json(), c.to_sorted_json());

            b.history_mut().undo()?;
            send_bytes(&b, &mut [&mut a, &mut c])?;

            assert_eq!("abc", text_of(&a));
            assert_eq!("abc", text_of(&b));
            let want = if with_ledger { "abc" } else { "abcd" };
            assert_eq!(want, text_of(&c), "with ledger: {with_ledger}");
        }
        Ok(())
    }

    /// The root of `b` in
    /// `replays_keeps_the_ledger_through_a_snapshot_over_proto_bytes`.
    fn ledger_snapshot_root(with_ledger: bool) -> WireJsonElement {
        let inserted = |offset| WireTextNodeId {
            created_at: TimeTicket::new(1, 2, A1),
            offset,
        };
        let a1_removal = TimeTicket::new(2, 1, A1);
        let a2_removal = TimeTicket::new(3, 1, A2);
        let d_events = if with_ledger {
            vec![a1_removal.clone()]
        } else {
            Vec::new()
        };
        let text_at = TimeTicket::new(1, 1, A1);
        let node = |offset, value: &str, removed_at, ins_prev: Option<usize>, removal_events| {
            WireTextNode {
                id: inserted(offset),
                value: value.to_owned(),
                removed_at,
                ins_prev_id: ins_prev.map(inserted),
                attributes: BTreeMap::new(),
                removal_events,
            }
        };
        WireJsonElement::Object {
            nodes: vec![rht_node(
                "t",
                WireJsonElement::Text {
                    nodes: vec![
                        node(0, "abc", None, None, Vec::new()),
                        node(3, "d", Some(a2_removal), Some(0), d_events),
                        node(4, "efgh", Some(a1_removal), Some(3), Vec::new()),
                    ],
                    created_at: text_at.clone(),
                    moved_at: Some(text_at),
                    removed_at: None,
                },
            )],
            created_at: TimeTicket::initial(),
            moved_at: None,
            removed_at: None,
        }
    }
}
