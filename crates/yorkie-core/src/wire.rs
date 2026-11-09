use crate::change::{Change, ChangeId, ChangePack, Checkpoint};
use crate::crdt::array::CrdtArray;
use crate::crdt::counter::{CounterType, CrdtCounter};
use crate::crdt::element::CrdtElement;
use crate::crdt::element_rht::ElementRht;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveType, PrimitiveValue};
use crate::crdt::rga_tree_list::RgaTreeList;
use crate::crdt::rga_tree_split::{
    RestoreSpan, RgaTreeSplit, RgaTreeSplitNode, RgaTreeSplitNodeId, RgaTreeSplitPos,
};
use crate::crdt::rht::Rht;
use crate::crdt::split_links::drop_split_links_in_element;
use crate::crdt::text::{CrdtText, TextValue};
use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId, TreePos};
use crate::operation::{
    AddOperation, ArraySetOperation, EditOperation, IncreaseOperation, MoveOperation, Operation,
    RemoveOperation, RestoreMode, SetOperation, StyleOperation, TreeEditOperation,
    TreeStyleOperation,
};
use crate::presence::{Presence, PresenceChange};
use crate::{ActorId, Result, TimeTicket, VersionVector, YorkieError};
use std::collections::{BTreeMap, BTreeSet};

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
    pub element: Option<Box<WireJsonElement>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireJsonElement {
    Object {
        nodes: Vec<WireRhtNode>,
        created_at: TimeTicket,
        moved_at: Option<TimeTicket>,
        removed_at: Option<TimeTicket>,
    },
    Array {
        nodes: Vec<WireRgaNode>,
        created_at: TimeTicket,
        moved_at: Option<TimeTicket>,
        removed_at: Option<TimeTicket>,
    },
    Primitive {
        value_type: WireValueType,
        value: Vec<u8>,
        created_at: TimeTicket,
        moved_at: Option<TimeTicket>,
        removed_at: Option<TimeTicket>,
    },
    Text {
        nodes: Vec<WireTextNode>,
        created_at: TimeTicket,
        moved_at: Option<TimeTicket>,
        removed_at: Option<TimeTicket>,
    },
    Counter {
        value_type: WireValueType,
        value: Vec<u8>,
        hll_registers: Vec<u8>,
        created_at: TimeTicket,
        moved_at: Option<TimeTicket>,
        removed_at: Option<TimeTicket>,
    },
    Tree {
        nodes: Vec<WireTreeNode>,
        created_at: TimeTicket,
        moved_at: Option<TimeTicket>,
        removed_at: Option<TimeTicket>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireRhtNode {
    pub key: String,
    pub element: WireJsonElement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireRgaNode {
    pub element: Option<WireJsonElement>,
    pub position_created_at: Option<TimeTicket>,
    pub position_moved_at: Option<TimeTicket>,
    pub position_removed_at: Option<TimeTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireNodeAttr {
    pub value: String,
    pub updated_at: TimeTicket,
    pub is_removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextNodeId {
    pub created_at: TimeTicket,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextNodePos {
    pub id: WireTextNodeId,
    pub relative_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextNode {
    pub id: WireTextNodeId,
    pub value: String,
    pub removed_at: Option<TimeTicket>,
    pub ins_prev_id: Option<WireTextNodeId>,
    pub attributes: BTreeMap<String, WireNodeAttr>,
    /// The node's removal ledger beyond `removed_at`, in ticket order
    /// (proto `TextNode.removal_events`). Empty when `removed_at` says it
    /// all, so such a node encodes to the bytes it did before the ledger.
    pub removal_events: Vec<TimeTicket>,
}

/// A run of characters from one insertion, addressed by identity: the
/// interval `[start, end)`, in UTF-16 code units, of the insertion created at
/// `created_at`, with a copy of its content and attributes (proto
/// `RestoreSpan`). The attributes travel as a bare map; the decoder stamps
/// them with the operation's `executed_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireRestoreSpan {
    pub created_at: TimeTicket,
    pub start: usize,
    pub end: usize,
    pub content: String,
    pub attributes: BTreeMap<String, String>,
}

/// The direction of an identity edit (proto `RestoreMode`): `Restore`
/// revives `restore_spans` and re-removes `retombstone_spans`;
/// `Retombstone`, the redo, does the opposite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireRestoreMode {
    Restore,
    Retombstone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTreeNodeId {
    pub created_at: TimeTicket,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTreePos {
    pub parent_id: WireTreeNodeId,
    pub left_sibling_id: WireTreeNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTreeNode {
    pub id: WireTreeNodeId,
    pub node_type: String,
    pub value: String,
    pub removed_at: Option<TimeTicket>,
    pub ins_prev_id: Option<WireTreeNodeId>,
    pub ins_next_id: Option<WireTreeNodeId>,
    pub depth: i32,
    pub attributes: BTreeMap<String, WireNodeAttr>,
    pub merged_from: Option<WireTreeNodeId>,
    pub merged_at: Option<TimeTicket>,
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
    Edit {
        parent_created_at: TimeTicket,
        from: WireTextNodePos,
        to: WireTextNodePos,
        content: String,
        attributes: BTreeMap<String, String>,
        executed_at: TimeTicket,
        /// The content an undo or redo revives, by identity. With
        /// `retombstone_spans`, empty on every edit but an identity one, whose
        /// positional fields are a zero-width, empty-content edit: its
        /// content travels only in the spans.
        restore_spans: Vec<WireRestoreSpan>,
        /// Set exactly when a span list is non-empty.
        restore_mode: Option<WireRestoreMode>,
        /// The content an undo or redo re-removes, by identity.
        retombstone_spans: Vec<WireRestoreSpan>,
    },
    Style {
        parent_created_at: TimeTicket,
        from: WireTextNodePos,
        to: WireTextNodePos,
        attributes: BTreeMap<String, String>,
        attributes_to_remove: Vec<String>,
        executed_at: TimeTicket,
    },
    Increase {
        parent_created_at: TimeTicket,
        value: WireJsonElementSimple,
        executed_at: TimeTicket,
        actor: Option<String>,
    },
    TreeEdit {
        parent_created_at: TimeTicket,
        from: WireTreePos,
        to: WireTreePos,
        contents: Option<Vec<Vec<WireTreeNode>>>,
        split_level: usize,
        executed_at: TimeTicket,
        /// The tickets issued for the elements the split created, in issue
        /// order; empty when the sender wrote none.
        split_tickets: Vec<TimeTicket>,
    },
    TreeStyle {
        parent_created_at: TimeTicket,
        from: WireTreePos,
        to: WireTreePos,
        attributes: BTreeMap<String, String>,
        attributes_to_remove: Vec<String>,
        executed_at: TimeTicket,
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

/// Wire projection of a presence: each value is its JSON encoding, as JS
/// `JSON.stringify` writes it into proto `Presence.data`.
pub type WirePresence = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WirePresenceChange {
    Put(WirePresence),
    /// Receive-only; see `PresenceChange::Patch`.
    Patch(WirePresence),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireChange {
    pub id: WireChangeId,
    pub message: Option<String>,
    pub operations: Vec<WireOperation>,
    pub presence_change: Option<WirePresenceChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireChangePack {
    pub document_key: String,
    pub checkpoint: Checkpoint,
    pub is_removed: bool,
    pub changes: Vec<WireChange>,
    pub snapshot: Option<Vec<u8>>,
    pub snapshot_root: Option<WireJsonElement>,
    /// Snapshot presences keyed by the actor ID string.
    pub snapshot_presences: BTreeMap<String, WirePresence>,
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
            snapshot_root: pack
                .snapshot_root()
                .map(|object| WireJsonElement::try_from(&CrdtElement::object(object.clone())))
                .transpose()?,
            snapshot_presences: pack
                .snapshot_presences()
                .iter()
                .map(|(actor_id, presence)| Ok((actor_id.to_string(), presence_to_wire(presence)?)))
                .collect::<Result<_>>()?,
            version_vector: pack.version_vector().cloned(),
        })
    }
}

impl TryFrom<WireChangePack> for ChangePack {
    type Error = YorkieError;

    fn try_from(pack: WireChangePack) -> Result<Self> {
        let changes = pack
            .changes
            .into_iter()
            .map(Change::try_from)
            .collect::<Result<Vec<_>>>()?;

        let snapshot_root = pack
            .snapshot_root
            .map(CrdtElement::try_from)
            .transpose()?
            .map(|element| match element {
                CrdtElement::Object(object) => Ok(*object),
                _ => Err(YorkieError::UnsupportedProtocolConversion(
                    "snapshot root must be object",
                )),
            })
            .transpose()?;

        let snapshot_presences = pack
            .snapshot_presences
            .iter()
            .map(|(actor_id, presence)| {
                Ok((
                    ActorId::from(actor_id.as_str()),
                    presence_from_wire(presence)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        Ok(Self::create_with_snapshot_root(
            pack.document_key,
            pack.checkpoint,
            pack.is_removed,
            changes,
            pack.version_vector,
            pack.snapshot,
            snapshot_root,
        )
        .with_snapshot_presences(snapshot_presences))
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
            presence_change: change
                .presence_change()
                .map(WirePresenceChange::try_from)
                .transpose()?,
        })
    }
}

impl TryFrom<WireChange> for Change {
    type Error = YorkieError;

    fn try_from(change: WireChange) -> Result<Self> {
        let operations = change
            .operations
            .into_iter()
            .map(Operation::try_from)
            .collect::<Result<Vec<_>>>()?;

        let presence_change = change
            .presence_change
            .as_ref()
            .map(PresenceChange::try_from)
            .transpose()?;

        Ok(
            Self::create(ChangeId::from(change.id), operations, change.message)
                .with_presence_change(presence_change),
        )
    }
}

impl TryFrom<&PresenceChange> for WirePresenceChange {
    type Error = YorkieError;

    fn try_from(change: &PresenceChange) -> Result<Self> {
        Ok(match change {
            PresenceChange::Put(presence) => Self::Put(presence_to_wire(presence)?),
            PresenceChange::Patch(presence) => Self::Patch(presence_to_wire(presence)?),
            PresenceChange::Clear => Self::Clear,
        })
    }
}

impl TryFrom<&WirePresenceChange> for PresenceChange {
    type Error = YorkieError;

    fn try_from(change: &WirePresenceChange) -> Result<Self> {
        Ok(match change {
            WirePresenceChange::Put(presence) => Self::Put(presence_from_wire(presence)?),
            WirePresenceChange::Patch(presence) => Self::Patch(presence_from_wire(presence)?),
            WirePresenceChange::Clear => Self::Clear,
        })
    }
}

fn presence_to_wire(presence: &Presence) -> Result<WirePresence> {
    presence
        .iter()
        .map(|(key, value)| {
            serde_json::to_string(value)
                .map(|json| (key.clone(), json))
                .map_err(|_| YorkieError::UnsupportedProtocolConversion("presence value"))
        })
        .collect()
}

fn presence_from_wire(presence: &WirePresence) -> Result<Presence> {
    presence
        .iter()
        .map(|(key, json)| {
            serde_json::from_str(json)
                .map(|value| (key.clone(), value))
                .map_err(|_| {
                    YorkieError::UnsupportedProtocolConversion("presence value is not JSON")
                })
        })
        .collect()
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

impl From<WireChangeId> for ChangeId {
    fn from(id: WireChangeId) -> Self {
        Self::new(
            id.client_seq,
            id.server_seq,
            id.lamport,
            id.actor_id,
            id.version_vector,
        )
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
            Operation::Edit(operation) => {
                let restore_spans = wire_restore_spans_from_domain(operation.restore_spans());
                let retombstone_spans =
                    wire_restore_spans_from_domain(operation.retombstone_spans());
                // JS `toOperation`: the mode travels only with a span, so an
                // ordinary edit encodes to the bytes it always did.
                let restore_mode = (!restore_spans.is_empty() || !retombstone_spans.is_empty())
                    .then(|| match operation.restore_mode() {
                        Some(RestoreMode::Retombstone) => WireRestoreMode::Retombstone,
                        Some(RestoreMode::Restore) | None => WireRestoreMode::Restore,
                    });
                Ok(Self::Edit {
                    parent_created_at: operation.parent_created_at().clone(),
                    from: WireTextNodePos::from(operation.range_from()),
                    to: WireTextNodePos::from(operation.to_pos()),
                    content: operation.content().to_owned(),
                    attributes: operation.attributes().clone(),
                    executed_at: operation.executed_at()?.clone(),
                    restore_spans,
                    restore_mode,
                    retombstone_spans,
                })
            }
            Operation::Style(operation) => Ok(Self::Style {
                parent_created_at: operation.parent_created_at().clone(),
                from: WireTextNodePos::from(operation.range_from()),
                to: WireTextNodePos::from(operation.to_pos()),
                attributes: operation.attributes().clone(),
                attributes_to_remove: operation.attributes_to_remove().to_vec(),
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Increase(operation) => Ok(Self::Increase {
                parent_created_at: operation.parent_created_at().clone(),
                value: WireJsonElementSimple::try_from(operation.value())?,
                executed_at: operation.executed_at()?.clone(),
                actor: (!operation.actor().is_empty()).then(|| operation.actor().to_owned()),
            }),
            Operation::TreeEdit(operation) => Ok(Self::TreeEdit {
                parent_created_at: operation.parent_created_at().clone(),
                from: WireTreePos::from(operation.range_from()),
                to: WireTreePos::from(operation.to_pos()),
                contents: operation
                    .contents()
                    .map(|contents| contents.iter().map(wire_tree_nodes_from_domain).collect()),
                split_level: operation.split_level(),
                executed_at: operation.executed_at()?.clone(),
                split_tickets: operation.split_tickets().to_vec(),
            }),
            Operation::TreeStyle(operation) => Ok(Self::TreeStyle {
                parent_created_at: operation.parent_created_at().clone(),
                from: WireTreePos::from(operation.range_from()),
                to: WireTreePos::from(operation.to_pos()),
                attributes: operation.attributes().clone(),
                attributes_to_remove: operation.attributes_to_remove().to_vec(),
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::ArraySet(operation) => Ok(Self::ArraySet {
                parent_created_at: operation.parent_created_at().clone(),
                created_at: operation.created_at().clone(),
                value: WireJsonElementSimple::try_from(operation.value())?,
                executed_at: operation.executed_at()?.clone(),
            }),
        }
    }
}

impl TryFrom<WireOperation> for Operation {
    type Error = YorkieError;

    fn try_from(operation: WireOperation) -> Result<Self> {
        match operation {
            WireOperation::Set {
                parent_created_at,
                key,
                value,
                executed_at,
            } => Ok(Self::Set(SetOperation::create(
                key,
                element_payload_from_wire(value)?,
                parent_created_at,
                Some(executed_at),
            ))),
            WireOperation::Add {
                parent_created_at,
                prev_created_at,
                value,
                executed_at,
            } => Ok(Self::Add(AddOperation::create(
                parent_created_at,
                prev_created_at,
                element_payload_from_wire(value)?,
                Some(executed_at),
            ))),
            WireOperation::Move {
                parent_created_at,
                prev_created_at,
                created_at,
                executed_at,
            } => Ok(Self::Move(MoveOperation::create(
                parent_created_at,
                prev_created_at,
                created_at,
                Some(executed_at),
            ))),
            WireOperation::Remove {
                parent_created_at,
                created_at,
                executed_at,
            } => Ok(Self::Remove(RemoveOperation::new(
                parent_created_at,
                created_at,
                Some(executed_at),
            ))),
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
                // JS `fromOperation`: spans are read only when a list is
                // non-empty, and then the edit is an undo or redo by
                // identity; a mode other than retombstone means restore.
                if restore_spans.is_empty() && retombstone_spans.is_empty() {
                    return Ok(Self::Edit(EditOperation::create(
                        parent_created_at,
                        RgaTreeSplitPos::from(from),
                        RgaTreeSplitPos::from(to),
                        content,
                        attributes,
                        Some(executed_at),
                    )));
                }

                let mode = match restore_mode {
                    Some(WireRestoreMode::Retombstone) => RestoreMode::Retombstone,
                    Some(WireRestoreMode::Restore) | None => RestoreMode::Restore,
                };
                let restore_spans = restore_spans_from_wire(restore_spans, &executed_at)?;
                let retombstone_spans = restore_spans_from_wire(retombstone_spans, &executed_at)?;
                Ok(Self::Edit(
                    EditOperation::new(
                        parent_created_at,
                        RgaTreeSplitPos::from(from),
                        RgaTreeSplitPos::from(to),
                        content,
                        attributes,
                        Some(executed_at),
                        true,
                    )
                    .with_identity(restore_spans, mode, retombstone_spans),
                ))
            }
            WireOperation::Style {
                parent_created_at,
                from,
                to,
                attributes,
                attributes_to_remove,
                executed_at,
            } => {
                let operation = if attributes.is_empty() {
                    StyleOperation::create_remove_style_operation(
                        parent_created_at,
                        RgaTreeSplitPos::from(from),
                        RgaTreeSplitPos::from(to),
                        attributes_to_remove,
                        Some(executed_at),
                    )
                } else {
                    StyleOperation::new(
                        parent_created_at,
                        RgaTreeSplitPos::from(from),
                        RgaTreeSplitPos::from(to),
                        attributes,
                        attributes_to_remove,
                        Some(executed_at),
                    )
                };
                Ok(Self::Style(operation))
            }
            WireOperation::Increase {
                parent_created_at,
                value,
                executed_at,
                actor,
            } => {
                let value = CrdtElement::try_from(value)?;
                let operation = if let Some(actor) = actor {
                    IncreaseOperation::create_with_actor(
                        parent_created_at,
                        value,
                        Some(executed_at),
                        actor,
                    )
                } else {
                    IncreaseOperation::create(parent_created_at, value, Some(executed_at))
                };
                Ok(Self::Increase(operation))
            }
            WireOperation::TreeEdit {
                parent_created_at,
                from,
                to,
                contents,
                split_level,
                executed_at,
                split_tickets,
            } => {
                let contents = contents
                    .map(|contents| {
                        contents
                            .into_iter()
                            .map(wire_tree_nodes_to_domain)
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?;
                let mut operation = TreeEditOperation::create(
                    parent_created_at,
                    TreePos::from(from),
                    TreePos::from(to),
                    contents,
                    split_level,
                    Some(executed_at),
                );
                operation.set_split_tickets(split_tickets);
                Ok(Self::TreeEdit(operation))
            }
            WireOperation::TreeStyle {
                parent_created_at,
                from,
                to,
                attributes,
                attributes_to_remove,
                executed_at,
            } => {
                let operation = if attributes_to_remove.is_empty() {
                    TreeStyleOperation::create(
                        parent_created_at,
                        TreePos::from(from),
                        TreePos::from(to),
                        attributes,
                        Some(executed_at),
                    )
                } else {
                    TreeStyleOperation::create_tree_remove_style_operation(
                        parent_created_at,
                        TreePos::from(from),
                        TreePos::from(to),
                        attributes_to_remove,
                        Some(executed_at),
                    )
                };
                Ok(Self::TreeStyle(operation))
            }
            WireOperation::ArraySet {
                parent_created_at,
                created_at,
                value,
                executed_at,
            } => Ok(Self::ArraySet(ArraySetOperation::create(
                parent_created_at,
                created_at,
                element_payload_from_wire(value)?,
                Some(executed_at),
            ))),
        }
    }
}

/// Decodes the element a Set, Add or ArraySet carries, without the
/// split-sibling links of any tree in it (JS `fromElementSimple`). Snapshots
/// keep theirs: they are the document's own state.
fn element_payload_from_wire(element: WireJsonElementSimple) -> Result<CrdtElement> {
    let mut element = CrdtElement::try_from(element)?;
    drop_split_links_in_element(&mut element);
    Ok(element)
}

impl TryFrom<&CrdtElement> for WireJsonElementSimple {
    type Error = YorkieError;

    fn try_from(element: &CrdtElement) -> Result<Self> {
        let value_type = match element {
            CrdtElement::Primitive(value) => primitive_value_type(value.value()),
            CrdtElement::Counter(value) => counter_value_type(value.counter_type()),
            CrdtElement::Object(_) => WireValueType::JsonObject,
            CrdtElement::Array(_) => WireValueType::JsonArray,
            CrdtElement::Text(_) => WireValueType::Text,
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
                CrdtElement::Object(_)
                | CrdtElement::Array(_)
                | CrdtElement::Text(_)
                | CrdtElement::Tree(_) => Vec::new(),
            },
            element: match element {
                CrdtElement::Object(_) | CrdtElement::Array(_) | CrdtElement::Tree(_) => {
                    Some(Box::new(WireJsonElement::try_from(element)?))
                }
                CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Text(_) => None,
            },
        })
    }
}

impl TryFrom<WireJsonElementSimple> for CrdtElement {
    type Error = YorkieError;

    fn try_from(element: WireJsonElementSimple) -> Result<Self> {
        if let Some(full_element) = element.element {
            return CrdtElement::try_from(*full_element);
        }

        match element.value_type {
            WireValueType::JsonObject => Ok(with_element_meta(
                CrdtElement::object(CrdtObject::create(element.created_at)),
                element.moved_at,
                element.removed_at,
            )),
            WireValueType::JsonArray => Ok(with_element_meta(
                CrdtElement::array(CrdtArray::create(element.created_at)),
                element.moved_at,
                element.removed_at,
            )),
            WireValueType::Text => Ok(with_element_meta(
                CrdtElement::text(CrdtText::create(element.created_at)),
                element.moved_at,
                element.removed_at,
            )),
            WireValueType::Tree => Err(YorkieError::UnsupportedProtocolConversion(
                "empty tree element simple",
            )),
            WireValueType::IntegerCnt | WireValueType::LongCnt | WireValueType::IntegerDedupCnt => {
                let counter_type = wire_value_type_to_counter_type(element.value_type)?;
                let value = CrdtCounter::value_from_bytes(counter_type, &element.value)?;
                Ok(with_element_meta(
                    CrdtElement::counter(CrdtCounter::create(
                        counter_type,
                        value,
                        element.created_at,
                    )),
                    element.moved_at,
                    element.removed_at,
                ))
            }
            _ => {
                let primitive_type = wire_value_type_to_primitive_type(element.value_type)?;
                let value = CrdtPrimitive::value_from_bytes(primitive_type, &element.value)?;
                Ok(with_element_meta(
                    CrdtElement::primitive(CrdtPrimitive::new(value, element.created_at)),
                    element.moved_at,
                    element.removed_at,
                ))
            }
        }
    }
}

impl TryFrom<&CrdtElement> for WireJsonElement {
    type Error = YorkieError;

    fn try_from(element: &CrdtElement) -> Result<Self> {
        match element {
            CrdtElement::Object(object) => wire_object_from_domain(object),
            CrdtElement::Array(array) => wire_array_from_domain(array),
            CrdtElement::Primitive(primitive) => Ok(wire_primitive_from_domain(primitive)),
            CrdtElement::Text(text) => Ok(wire_text_from_domain(text)),
            CrdtElement::Counter(counter) => Ok(wire_counter_from_domain(counter)),
            CrdtElement::Tree(tree) => Ok(wire_tree_from_domain(tree)),
        }
    }
}

fn wire_object_from_domain(object: &CrdtObject) -> Result<WireJsonElement> {
    Ok(WireJsonElement::Object {
        nodes: object
            .iter_all()
            .map(|(key, element)| {
                Ok(WireRhtNode {
                    key: key.to_owned(),
                    element: WireJsonElement::try_from(element)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        created_at: object.created_at().clone(),
        moved_at: object.moved_at().cloned(),
        removed_at: object.removed_at().cloned(),
    })
}

fn wire_array_from_domain(array: &CrdtArray) -> Result<WireJsonElement> {
    Ok(WireJsonElement::Array {
        nodes: array
            .iter_all_nodes()
            .map(|node| {
                Ok(WireRgaNode {
                    element: node.element().map(WireJsonElement::try_from).transpose()?,
                    position_created_at: if node.element().is_none()
                        || node.position_moved_at().is_some()
                    {
                        Some(node.position_created_at().clone())
                    } else {
                        None
                    },
                    position_moved_at: node.position_moved_at().cloned(),
                    position_removed_at: node.removed_at().cloned(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
        created_at: array.created_at().clone(),
        moved_at: array.moved_at().cloned(),
        removed_at: array.removed_at().cloned(),
    })
}

fn wire_primitive_from_domain(primitive: &CrdtPrimitive) -> WireJsonElement {
    WireJsonElement::Primitive {
        value_type: primitive_value_type(primitive.value()),
        value: primitive.to_bytes(),
        created_at: primitive.created_at().clone(),
        moved_at: primitive.moved_at().cloned(),
        removed_at: primitive.removed_at().cloned(),
    }
}

fn wire_text_from_domain(text: &CrdtText) -> WireJsonElement {
    // The decoder resolves `ins_prev_id` against the nodes it has already
    // decoded (JS `fromText`; Go rejects a link it cannot find), so a link is
    // written only when its target was emitted earlier in this very list
    // (JS `toTextNodes`). A split product's `ins_prev` always precedes it,
    // so this drops nothing a snapshot or a re-issue needs; what it drops is
    // a dangling link into a node no longer in this split, which no decoder
    // could have resolved.
    let mut emitted = BTreeSet::new();
    let nodes = text
        .nodes()
        .map(|node| {
            let wire = wire_text_node_from_domain(node, &emitted);
            emitted.insert(node.id().clone());
            wire
        })
        .collect();
    WireJsonElement::Text {
        nodes,
        created_at: text.created_at().clone(),
        moved_at: text.moved_at().cloned(),
        removed_at: text.removed_at().cloned(),
    }
}

fn wire_counter_from_domain(counter: &CrdtCounter) -> WireJsonElement {
    WireJsonElement::Counter {
        value_type: counter_value_type(counter.counter_type()),
        value: counter.to_bytes(),
        hll_registers: counter.hll_bytes().unwrap_or_default(),
        created_at: counter.created_at().clone(),
        moved_at: counter.moved_at().cloned(),
        removed_at: counter.removed_at().cloned(),
    }
}

fn wire_tree_from_domain(tree: &CrdtTree) -> WireJsonElement {
    WireJsonElement::Tree {
        nodes: wire_tree_nodes_from_domain(tree.root()),
        created_at: tree.created_at().clone(),
        moved_at: tree.moved_at().cloned(),
        removed_at: tree.removed_at().cloned(),
    }
}

impl TryFrom<WireJsonElement> for CrdtElement {
    type Error = YorkieError;

    fn try_from(element: WireJsonElement) -> Result<Self> {
        match element {
            WireJsonElement::Object {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => crdt_object_from_wire(nodes, created_at, moved_at, removed_at),
            WireJsonElement::Array {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => crdt_array_from_wire(nodes, created_at, moved_at, removed_at),
            WireJsonElement::Primitive {
                value_type,
                value,
                created_at,
                moved_at,
                removed_at,
            } => crdt_primitive_from_wire(value_type, value, created_at, moved_at, removed_at),
            WireJsonElement::Text {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => crdt_text_from_wire(nodes, created_at, moved_at, removed_at),
            WireJsonElement::Counter {
                value_type,
                value,
                hll_registers,
                created_at,
                moved_at,
                removed_at,
            } => crdt_counter_from_wire(
                value_type,
                value,
                hll_registers,
                created_at,
                moved_at,
                removed_at,
            ),
            WireJsonElement::Tree {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => crdt_tree_from_wire(nodes, created_at, moved_at, removed_at),
        }
    }
}

fn crdt_object_from_wire(
    nodes: Vec<WireRhtNode>,
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> Result<CrdtElement> {
    let mut members = ElementRht::new();
    for node in nodes {
        members.set_internal(node.key, CrdtElement::try_from(node.element)?);
    }

    Ok(with_element_meta(
        CrdtElement::object(CrdtObject::new(created_at, members)),
        moved_at,
        removed_at,
    ))
}

fn crdt_array_from_wire(
    nodes: Vec<WireRgaNode>,
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> Result<CrdtElement> {
    let mut elements = RgaTreeList::new();
    for node in nodes {
        match (node.element, node.position_moved_at) {
            (None, _) => {
                let position_created_at =
                    node.position_created_at
                        .ok_or(YorkieError::UnsupportedProtocolConversion(
                            "dead RGA node without position_created_at",
                        ))?;
                let position_removed_at =
                    node.position_removed_at
                        .ok_or(YorkieError::UnsupportedProtocolConversion(
                            "dead RGA node without position_removed_at",
                        ))?;
                elements.add_dead_position(position_created_at, position_removed_at);
            }
            (Some(element), Some(position_moved_at)) => {
                let position_created_at =
                    node.position_created_at
                        .ok_or(YorkieError::UnsupportedProtocolConversion(
                            "moved RGA node without position_created_at",
                        ))?;
                elements.add_moved_element(
                    CrdtElement::try_from(element)?,
                    position_created_at,
                    position_moved_at,
                );
            }
            (Some(element), None) => {
                elements.add(CrdtElement::try_from(element)?)?;
            }
        }
    }

    Ok(with_element_meta(
        CrdtElement::array(CrdtArray::new(created_at, elements)),
        moved_at,
        removed_at,
    ))
}

fn crdt_primitive_from_wire(
    value_type: WireValueType,
    value: Vec<u8>,
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> Result<CrdtElement> {
    let primitive_type = wire_value_type_to_primitive_type(value_type)?;
    let value = CrdtPrimitive::value_from_bytes(primitive_type, &value)?;
    Ok(with_element_meta(
        CrdtElement::primitive(CrdtPrimitive::new(value, created_at)),
        moved_at,
        removed_at,
    ))
}

fn crdt_text_from_wire(
    nodes: Vec<WireTextNode>,
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> Result<CrdtElement> {
    Ok(with_element_meta(
        CrdtElement::text(text_from_wire_nodes(created_at, nodes)?),
        moved_at,
        removed_at,
    ))
}

fn crdt_counter_from_wire(
    value_type: WireValueType,
    value: Vec<u8>,
    hll_registers: Vec<u8>,
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> Result<CrdtElement> {
    let counter_type = wire_value_type_to_counter_type(value_type)?;
    let counter_value = CrdtCounter::value_from_bytes(counter_type, &value)?;
    let mut counter = CrdtCounter::create(counter_type, counter_value, created_at);
    if counter.is_dedup() && !hll_registers.is_empty() {
        counter.restore_hll(&hll_registers)?;
    }
    Ok(with_element_meta(
        CrdtElement::counter(counter),
        moved_at,
        removed_at,
    ))
}

fn crdt_tree_from_wire(
    nodes: Vec<WireTreeNode>,
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> Result<CrdtElement> {
    Ok(with_element_meta(
        CrdtElement::tree(CrdtTree::new(wire_tree_nodes_to_domain(nodes)?, created_at)),
        moved_at,
        removed_at,
    ))
}

impl From<&RgaTreeSplitNodeId> for WireTextNodeId {
    fn from(id: &RgaTreeSplitNodeId) -> Self {
        Self {
            created_at: id.created_at().clone(),
            offset: id.offset(),
        }
    }
}

impl From<WireTextNodeId> for RgaTreeSplitNodeId {
    fn from(id: WireTextNodeId) -> Self {
        Self::new(id.created_at, id.offset)
    }
}

impl From<&RgaTreeSplitPos> for WireTextNodePos {
    fn from(pos: &RgaTreeSplitPos) -> Self {
        Self {
            id: WireTextNodeId::from(pos.id()),
            relative_offset: pos.relative_offset(),
        }
    }
}

impl From<WireTextNodePos> for RgaTreeSplitPos {
    fn from(pos: WireTextNodePos) -> Self {
        Self::new(RgaTreeSplitNodeId::from(pos.id), pos.relative_offset)
    }
}

impl From<&TreeNodeId> for WireTreeNodeId {
    fn from(id: &TreeNodeId) -> Self {
        Self {
            created_at: id.created_at().clone(),
            offset: id.offset(),
        }
    }
}

impl From<WireTreeNodeId> for TreeNodeId {
    fn from(id: WireTreeNodeId) -> Self {
        Self::new(id.created_at, id.offset)
    }
}

impl From<&TreePos> for WireTreePos {
    fn from(pos: &TreePos) -> Self {
        Self {
            parent_id: WireTreeNodeId::from(pos.parent_id()),
            left_sibling_id: WireTreeNodeId::from(pos.left_sibling_id()),
        }
    }
}

impl From<WireTreePos> for TreePos {
    fn from(pos: WireTreePos) -> Self {
        Self::new(
            TreeNodeId::from(pos.parent_id),
            TreeNodeId::from(pos.left_sibling_id),
        )
    }
}

fn wire_restore_spans_from_domain(spans: &[RestoreSpan<TextValue>]) -> Vec<WireRestoreSpan> {
    spans
        .iter()
        .map(|span| WireRestoreSpan {
            created_at: span.created_at.clone(),
            start: span.start,
            end: span.end,
            content: span.value.content().to_owned(),
            attributes: span.value.get_attributes(),
        })
        .collect()
}

/// Decodes the spans of an identity edit (JS `fromPbSpan`). `restore`
/// slices a span's content by its offsets, so `[start, end)` has to be an
/// ordered interval exactly covering the content, in UTF-16 code units; a
/// span that is not fails the whole operation here instead of deep in the
/// restore path. The attributes, a bare map on the wire, are stamped with
/// the operation's `executed_at`, as the server does when it recreates a
/// node from a span.
fn restore_spans_from_wire(
    spans: Vec<WireRestoreSpan>,
    executed_at: &TimeTicket,
) -> Result<Vec<RestoreSpan<TextValue>>> {
    spans
        .into_iter()
        .map(|span| {
            if span.end < span.start || span.end - span.start != span.content.encode_utf16().count()
            {
                return Err(YorkieError::InvalidArgument(
                    "malformed restore span: invalid range".to_owned(),
                ));
            }
            let mut value = TextValue::create(span.content);
            for (key, attr) in span.attributes {
                value.set_attr(key, attr, executed_at.clone());
            }
            Ok(RestoreSpan {
                created_at: span.created_at,
                start: span.start,
                end: span.end,
                value,
            })
        })
        .collect()
}

fn wire_text_node_from_domain(
    node: &RgaTreeSplitNode<TextValue>,
    emitted: &BTreeSet<RgaTreeSplitNodeId>,
) -> WireTextNode {
    WireTextNode {
        id: WireTextNodeId::from(node.id()),
        value: node.value().content().to_owned(),
        removed_at: node.removed_at().cloned(),
        ins_prev_id: node
            .ins_prev_id()
            .filter(|id| emitted.contains(*id))
            .map(WireTextNodeId::from),
        attributes: text_attrs_from_domain(node.value().attributes()),
        removal_events: node.removal_events(),
    }
}

/// JS `toTextNodes` (since yorkie-js-sdk#1364) and Go `toTextNodes`: every
/// attribute node, a removed one with its `is_removed`, as the tree path
/// writes them.
fn text_attrs_from_domain(attrs: &Rht) -> BTreeMap<String, WireNodeAttr> {
    attrs
        .iter()
        .map(|node| {
            (
                node.key().to_owned(),
                WireNodeAttr {
                    value: node.value().to_owned(),
                    updated_at: node.updated_at().clone(),
                    is_removed: node.is_removed(),
                },
            )
        })
        .collect()
}

fn tree_attrs_from_domain(attrs: Option<&Rht>) -> BTreeMap<String, WireNodeAttr> {
    attrs
        .into_iter()
        .flat_map(Rht::iter)
        .map(|node| {
            (
                node.key().to_owned(),
                WireNodeAttr {
                    value: node.value().to_owned(),
                    updated_at: node.updated_at().clone(),
                    is_removed: node.is_removed(),
                },
            )
        })
        .collect()
}

fn text_from_wire_nodes(created_at: TimeTicket, nodes: Vec<WireTextNode>) -> Result<CrdtText> {
    let initial_head = RgaTreeSplitNode::new(
        RgaTreeSplitNodeId::new(TimeTicket::initial(), 0),
        TextValue::create(""),
    );
    let mut split = RgaTreeSplit::new(initial_head);
    let mut prev_id = split.initial_head().id().clone();
    // A Text can arrive inside a client-supplied Set/Add/ArraySet payload,
    // where its links are as untrusted as the tree links
    // `drop_split_links_in_element` strips. They are kept, since such a
    // payload can be a copy of real document state whose split pieces a
    // later Edit targets, but each is resolved by exact ID against the nodes
    // decoded so far (JS `fromText`). Every producer of a link points it at
    // an earlier piece of the same insertion, so a link naming another
    // insertion, or an offset at or above the node's own, is one no local
    // edit made. Any link that does not resolve is dropped, as JS does; Go
    // rejects the element instead.
    let mut decoded = BTreeSet::new();

    for node in nodes {
        let mut text_node = RgaTreeSplitNode::new(
            RgaTreeSplitNodeId::from(node.id),
            TextValue::new(node.value, text_attrs_to_domain(node.attributes)),
        );
        text_node.set_removed_at(node.removed_at);
        // JS `fromTextNode` logs a ledger `set_removal_events` refuses and
        // drops it, leaving the node with `removed_at` alone: failing here
        // would stop a client from attaching or from draining the pull that
        // carries one peer's bad node, a document wedged for good rather
        // than one node decoded coarsely. `yorkie-core` has no logger, so the
        // drop is silent (see the porting gaps).
        let _ = text_node.set_removal_events(node.removal_events);
        let inserted_id = split.insert_after_id(&prev_id, text_node)?;
        let ins_prev_id = node.ins_prev_id.map(RgaTreeSplitNodeId::from).filter(|id| {
            id.has_same_created_at(&inserted_id)
                && id.offset() < inserted_id.offset()
                && decoded.contains(id)
        });
        if let Some(ins_prev_id) = ins_prev_id {
            // JS `setInsPrev` links both ways, so a later split of the
            // target relinks this node as it does on the encoding replica.
            if let Some(target) = split.find_node_mut_by_id(&ins_prev_id) {
                target.set_ins_next_id(Some(inserted_id.clone()));
            }
            if let Some(current) = split.find_node_mut_by_id(&inserted_id) {
                current.set_ins_prev_id(Some(ins_prev_id));
            }
        }
        decoded.insert(inserted_id.clone());
        prev_id = inserted_id;
    }

    Ok(CrdtText::new(created_at, split))
}

/// JS `fromTextNode` (since yorkie-js-sdk#1364): `setInternal` keeps each
/// attribute as written, a tombstone included, value and all. A tombstone
/// decoded live would carry the removal's ticket and win LWW against every
/// older write. Go's `SetInternal` clears a removed attribute's value; JS
/// keeps it, and so does Rust.
fn text_attrs_to_domain(attrs: BTreeMap<String, WireNodeAttr>) -> Rht {
    let mut rht = Rht::new();
    for (key, attr) in attrs {
        rht.set_internal(key, attr.value, attr.updated_at, attr.is_removed);
    }
    rht
}

fn tree_attrs_to_domain(attrs: BTreeMap<String, WireNodeAttr>) -> Option<Rht> {
    if attrs.is_empty() {
        return None;
    }

    let mut rht = Rht::new();
    for (key, attr) in attrs {
        rht.set_internal(key, attr.value, attr.updated_at, attr.is_removed);
    }
    Some(rht)
}

fn wire_tree_nodes_from_domain(root: &TreeNode) -> Vec<WireTreeNode> {
    let mut nodes = Vec::new();
    push_wire_tree_nodes(root, 0, &mut nodes);
    nodes
}

fn push_wire_tree_nodes(node: &TreeNode, depth: i32, nodes: &mut Vec<WireTreeNode>) {
    for child in node.all_children() {
        push_wire_tree_nodes(child, depth + 1, nodes);
    }

    nodes.push(WireTreeNode {
        id: WireTreeNodeId::from(node.id()),
        node_type: node.node_type().to_owned(),
        value: node.value().to_owned(),
        removed_at: node.removed_at().cloned(),
        ins_prev_id: node.ins_prev_id().map(WireTreeNodeId::from),
        ins_next_id: node.ins_next_id().map(WireTreeNodeId::from),
        depth,
        attributes: tree_attrs_from_domain(node.attrs()),
        merged_from: node.merged_from().map(WireTreeNodeId::from),
        merged_at: node.merged_at().cloned(),
    });
}

fn wire_tree_nodes_to_domain(nodes: Vec<WireTreeNode>) -> Result<TreeNode> {
    if nodes.is_empty() {
        return Err(YorkieError::UnsupportedProtocolConversion(
            "empty tree nodes",
        ));
    }

    let mut stack: Vec<(i32, TreeNode)> = Vec::new();
    for node in nodes {
        let depth = node.depth;
        let mut children = Vec::new();
        while stack
            .last()
            .map(|(child_depth, _)| *child_depth > depth)
            .unwrap_or(false)
        {
            children.push(stack.pop().expect("stack last checked").1);
        }
        children.reverse();

        let mut tree_node = wire_tree_node_to_domain(node)?;
        for child in children {
            tree_node.append(child);
        }
        stack.push((depth, tree_node));
    }

    if stack.len() != 1 {
        return Err(YorkieError::UnsupportedProtocolConversion(
            "malformed tree nodes",
        ));
    }

    Ok(stack.pop().expect("stack length checked").1)
}

fn wire_tree_node_to_domain(node: WireTreeNode) -> Result<TreeNode> {
    let mut tree_node = TreeNode::new(
        TreeNodeId::from(node.id),
        node.node_type,
        tree_attrs_to_domain(node.attributes),
        node.value,
        Vec::new(),
    );
    tree_node.set_removed_at(node.removed_at);
    tree_node.set_ins_prev_id(node.ins_prev_id.map(TreeNodeId::from));
    tree_node.set_ins_next_id(node.ins_next_id.map(TreeNodeId::from));
    tree_node.set_merged_from(node.merged_from.map(TreeNodeId::from));
    tree_node.set_merged_at(node.merged_at);
    Ok(tree_node)
}

fn with_element_meta(
    mut element: CrdtElement,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
) -> CrdtElement {
    element.set_moved_at(moved_at);
    element.set_removed_at(removed_at);
    element
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

fn wire_value_type_to_primitive_type(value_type: WireValueType) -> Result<PrimitiveType> {
    match value_type {
        WireValueType::Null => Ok(PrimitiveType::Null),
        WireValueType::Boolean => Ok(PrimitiveType::Boolean),
        WireValueType::Integer => Ok(PrimitiveType::Integer),
        WireValueType::Long => Ok(PrimitiveType::Long),
        WireValueType::Double => Ok(PrimitiveType::Double),
        WireValueType::String => Ok(PrimitiveType::String),
        WireValueType::Bytes => Ok(PrimitiveType::Bytes),
        WireValueType::Date => Ok(PrimitiveType::Date),
        _ => Err(YorkieError::UnsupportedProtocolConversion(
            "non-primitive value type",
        )),
    }
}

fn wire_value_type_to_counter_type(value_type: WireValueType) -> Result<CounterType> {
    match value_type {
        WireValueType::IntegerCnt => Ok(CounterType::Integer),
        WireValueType::LongCnt => Ok(CounterType::Long),
        WireValueType::IntegerDedupCnt => Ok(CounterType::IntegerDedup),
        _ => Err(YorkieError::UnsupportedProtocolConversion(
            "non-counter value type",
        )),
    }
}

#[cfg(test)]
#[path = "wire_text_link_tests.rs"]
mod text_link_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::counter::CounterValue;
    use crate::crdt::root::CrdtRoot;

    #[test]
    fn roundtrips_full_json_element_like_converter_root_bytes() -> Result<()> {
        // The full root fixture recursively walks object, array, text, and
        // counter values, so give the test a stable stack independent of the
        // test harness default.
        std::thread::Builder::new()
            .name("wire-root-roundtrip".to_owned())
            .stack_size(16 * 1024 * 1024)
            .spawn(roundtrips_full_json_element_like_converter_root_bytes_inner)
            .expect("thread should start")
            .join()
            .expect("thread should finish")
    }

    fn roundtrips_full_json_element_like_converter_root_bytes_inner() -> Result<()> {
        let mut root = CrdtRoot::create();

        let object_at = ticket(1);
        root.set_object_member(
            &TimeTicket::initial(),
            "k1",
            CrdtElement::object(CrdtObject::create(object_at.clone())),
            object_at.clone(),
        )?;
        root.set_object_member(
            &object_at,
            "k1-1",
            primitive_bool(true, ticket(2)),
            ticket(2),
        )?;
        root.set_object_member(
            &object_at,
            "k1-2",
            primitive_i32(2147483647, ticket(3)),
            ticket(3),
        )?;
        root.set_object_member(&object_at, "k1-5", primitive_str("4", ticket(4)), ticket(4))?;

        let array_at = ticket(5);
        root.set_object_member(
            &TimeTicket::initial(),
            "k2",
            CrdtElement::array(CrdtArray::create(array_at.clone())),
            array_at.clone(),
        )?;
        root.insert_array_element(
            &array_at,
            &TimeTicket::initial(),
            primitive_bool(true, ticket(6)),
            ticket(6),
        )?;
        root.insert_array_element(
            &array_at,
            &ticket(6),
            primitive_i32(2147483647, ticket(7)),
            ticket(7),
        )?;
        root.insert_array_element(
            &array_at,
            &ticket(7),
            primitive_str("4", ticket(8)),
            ticket(8),
        )?;

        let text_at = ticket(9);
        let mut text = CrdtText::create(text_at.clone());
        text.edit_by_index(0, 0, "\u{314e}", None, ticket(10), None)?;
        text.edit_by_index(0, 1, "\u{d558}", None, ticket(11), None)?;
        text.edit_by_index(0, 1, "\u{d55c}", None, ticket(12), None)?;
        text.edit_by_index(0, 1, "\u{d558}", None, ticket(13), None)?;
        text.edit_by_index(1, 1, "\u{b290}", None, ticket(14), None)?;
        text.edit_by_index(1, 2, "\u{b298}", None, ticket(15), None)?;
        let mut attrs = BTreeMap::new();
        attrs.insert("bold".to_owned(), "true".to_owned());
        attrs.insert("indent".to_owned(), "2".to_owned());
        attrs.insert("italic".to_owned(), "false".to_owned());
        attrs.insert("color".to_owned(), "red".to_owned());
        text.set_style_by_index(0, 2, attrs, ticket(16), None)?;
        root.set_object_member(
            &TimeTicket::initial(),
            "k3",
            CrdtElement::text(text),
            text_at.clone(),
        )?;

        let counter_at = ticket(17);
        let mut counter = CrdtCounter::create(
            CounterType::Integer,
            CounterValue::Integer(0),
            counter_at.clone(),
        );
        counter.increase(&CrdtPrimitive::new(PrimitiveValue::Integer(1), ticket(18)))?;
        counter.increase(&CrdtPrimitive::new(PrimitiveValue::Integer(2), ticket(19)))?;
        counter.increase(&CrdtPrimitive::new(PrimitiveValue::Integer(3), ticket(20)))?;
        root.set_object_member(
            &TimeTicket::initial(),
            "k4",
            CrdtElement::counter(counter),
            counter_at.clone(),
        )?;

        let roundtripped = roundtrip_root_json_element(&root)?;

        assert_eq!(root.to_sorted_json(), roundtripped.to_sorted_json());
        assert_eq!(
            r#"{"k1":{"k1-1":true,"k1-2":2147483647,"k1-5":"4"},"k2":[true,2147483647,"4"],"k3":[{"attrs":{"bold":true,"color":"red","indent":2,"italic":false},"val":"하"},{"attrs":{"bold":true,"color":"red","indent":2,"italic":false},"val":"늘"}],"k4":6}"#,
            roundtripped.to_sorted_json()
        );
        assert_eq!(
            "\u{d558}\u{b298}",
            roundtripped
                .text_by_created_at(&text_at)
                .unwrap()
                .to_string()
        );
        assert_eq!(
            CounterValue::Integer(6),
            roundtripped
                .counter_by_created_at(&counter_at)
                .unwrap()
                .value()
        );
        Ok(())
    }

    #[test]
    fn roundtrips_array_json_element_like_converter_array_bytes() -> Result<()> {
        let mut array = CrdtArray::create(ticket(1));
        array.insert_after(
            &TimeTicket::initial(),
            primitive_str("1", ticket(2)),
            Some(ticket(2)),
        )?;
        array.insert_after(&ticket(2), primitive_str("2", ticket(3)), Some(ticket(3)))?;
        array.insert_after(&ticket(3), primitive_str("3", ticket(4)), Some(ticket(4)))?;

        let element = CrdtElement::array(array);
        let wire = WireJsonElement::try_from(&element)?;
        let roundtripped = CrdtElement::try_from(wire)?;

        assert_eq!(r#"["1","2","3"]"#, element.to_sorted_json());
        assert_eq!(r#"["1","2","3"]"#, roundtripped.to_sorted_json());
        Ok(())
    }

    #[test]
    fn carries_tree_edit_split_tickets_through_wire_shape() -> Result<()> {
        let pos = TreePos::new(tree_node_id(1, 0), tree_node_id(2, 1));
        let mut operation =
            TreeEditOperation::create(ticket(1), pos.clone(), pos, None, 1, Some(ticket(10)));
        let split_tickets = vec![TimeTicket::new(10, 4, "000000000000000000000001")];
        operation.set_split_tickets(split_tickets.clone());

        let wire = WireOperation::try_from(&Operation::TreeEdit(operation))?;
        let WireOperation::TreeEdit {
            split_tickets: wire_tickets,
            ..
        } = &wire
        else {
            panic!("expected a tree edit");
        };
        assert_eq!(&split_tickets, wire_tickets);
        let Operation::TreeEdit(decoded) = Operation::try_from(wire)? else {
            panic!("expected a tree edit");
        };
        assert_eq!(split_tickets, decoded.split_tickets());
        Ok(())
    }

    #[test]
    fn roundtrips_tree_nodes_through_wire_shape() -> Result<()> {
        let root = TreeNode::create_element(
            tree_node_id(1, 0),
            "r",
            None,
            vec![
                TreeNode::create_element(
                    tree_node_id(2, 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(tree_node_id(3, 0), "hello")],
                ),
                TreeNode::create_element(
                    tree_node_id(4, 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(tree_node_id(5, 0), "world")],
                ),
            ],
        );

        let wire = wire_tree_nodes_from_domain(&root);
        let roundtripped = wire_tree_nodes_to_domain(wire)?;

        assert_eq!(root.to_json(), roundtripped.to_json());
        assert_eq!(root.to_xml(), roundtripped.to_xml());
        Ok(())
    }

    #[test]
    fn roundtrips_tree_json_element_like_converter_tree_bytes() -> Result<()> {
        let tree = CrdtTree::create(
            TreeNode::create_element(
                tree_node_id(1, 0),
                "r",
                None,
                vec![
                    TreeNode::create_element(
                        tree_node_id(2, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(tree_node_id(3, 0), "hello")],
                    ),
                    TreeNode::create_element(
                        tree_node_id(4, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(tree_node_id(5, 0), "world")],
                    ),
                ],
            ),
            ticket(10),
        );
        let element = CrdtElement::tree(tree);
        let CrdtElement::Tree(source_tree) = &element else {
            return Err(YorkieError::UnsupportedProtocolConversion("expected tree"));
        };
        let wire = WireJsonElement::try_from(&element)?;
        let roundtripped = CrdtElement::try_from(wire)?;

        assert_eq!(r#"<r><p>hello</p><p>world</p></r>"#, source_tree.to_xml());
        assert_eq!(element.to_sorted_json(), roundtripped.to_sorted_json());
        let CrdtElement::Tree(tree) = roundtripped else {
            return Err(YorkieError::UnsupportedProtocolConversion("expected tree"));
        };
        assert_eq!(r#"<r><p>hello</p><p>world</p></r>"#, tree.to_xml());
        Ok(())
    }

    #[test]
    fn preserves_tree_edit_style_state_across_root_roundtrip() -> Result<()> {
        let tree_at = ticket(1);
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                tree_node_id(2, 0),
                "r",
                None,
                vec![
                    TreeNode::create_element(
                        tree_node_id(3, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(tree_node_id(4, 0), "12")],
                    ),
                    TreeNode::create_element(
                        tree_node_id(5, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(tree_node_id(6, 0), "34")],
                    ),
                ],
            ),
            tree_at.clone(),
        );

        let range = (tree.path_to_pos(&[0, 1])?, tree.path_to_pos(&[1, 1])?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(10), None)?;
        let style_range = (tree.find_pos(0, true)?, tree.find_pos(1, true)?);
        tree.style_by_range_with_changes(
            style_range.clone(),
            BTreeMap::from([
                ("b".to_owned(), "t".to_owned()),
                ("i".to_owned(), "t".to_owned()),
            ]),
            ticket(11),
            None,
        )?;
        assert_eq!(r#"<r><p b="t" i="t">14</p></r>"#, tree.to_xml());
        tree.remove_style_by_range_with_changes(style_range, &["i".to_owned()], ticket(12), None)?;
        assert_eq!(r#"<r><p b="t">14</p></r>"#, tree.to_xml());

        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("tree", CrdtElement::tree(tree))],
        ));
        let roundtripped = roundtrip_root_json_element(&root)?;
        let tree = roundtripped.tree_by_created_at(&tree_at).unwrap();

        assert_eq!(r#"<r><p b="t">14</p></r>"#, tree.to_xml());
        assert_eq!(tree.node_map_len(), tree.nodes().count());
        assert_eq!(4, tree.root().len());
        Ok(())
    }

    #[test]
    fn rebuilds_a_chained_merge_from_a_snapshot() -> Result<()> {
        // JS `tree_merge_anchor_test.ts` "converges when insert is anchored
        // in a chained merge", its snapshot round trip: a replica loading the
        // tree after the chained merge rebuilds the same nodes, tombstones
        // included, and the same forwarding pointers.
        let tree_at = ticket(1);
        let paragraph = |lamport: i64, value: &str| {
            TreeNode::create_element(
                tree_node_id(lamport, 0),
                "p",
                None,
                vec![TreeNode::create_text(tree_node_id(lamport + 1, 0), value)],
            )
        };
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                tree_node_id(2, 0),
                "r",
                None,
                vec![paragraph(3, "ab"), paragraph(5, "cd"), paragraph(7, "ef")],
            ),
            tree_at.clone(),
        );
        let merge_p1 = (tree.find_pos(2, true)?, tree.find_pos(6, true)?);
        let merge_p2 = (tree.find_pos(6, true)?, tree.find_pos(10, true)?);
        let insert = tree.find_pos(9, true)?;
        let actor = |actor: &str| TimeTicket::new(20, 0, actor);
        let known = |actor_id: &str| {
            let mut version_vector = VersionVector::new();
            version_vector.set("000000000000000000000001", 9);
            version_vector.set(actor_id, 20);
            version_vector
        };
        tree.edit_by_range_with_changes(
            merge_p2,
            None,
            0,
            actor("000000000000000000000003"),
            Some(&known("000000000000000000000003")),
        )?;
        tree.edit_by_range_with_changes(
            merge_p1,
            None,
            0,
            actor("000000000000000000000002"),
            Some(&known("000000000000000000000002")),
        )?;
        tree.edit_by_range_with_changes(
            (insert.clone(), insert),
            Some(vec![TreeNode::create_element(
                TreeNodeId::new(actor("000000000000000000000004"), 0),
                "x",
                None,
                Vec::new(),
            )]),
            0,
            actor("000000000000000000000004"),
            Some(&known("000000000000000000000004")),
        )?;
        assert_eq!("<r><p>a<x></x>f</p></r>", tree.to_xml());

        let nodes = |tree: &CrdtTree| {
            tree.nodes()
                .map(|node| {
                    (
                        node.to_xml(),
                        node.len(),
                        node.is_removed(),
                        node.merged_into().cloned(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let runtime = nodes(&tree);
        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("t", CrdtElement::tree(tree))],
        ));
        let roundtripped = roundtrip_root_json_element(&root)?;
        assert_eq!(
            runtime,
            nodes(roundtripped.tree_by_created_at(&tree_at).unwrap())
        );
        Ok(())
    }

    #[test]
    fn preserves_tree_merge_state_across_root_roundtrip() -> Result<()> {
        let tree_at = ticket(1);
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                tree_node_id(2, 0),
                "root",
                None,
                vec![
                    TreeNode::create_element(
                        tree_node_id(3, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(tree_node_id(4, 0), "a")],
                    ),
                    TreeNode::create_element(
                        tree_node_id(5, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(tree_node_id(6, 0), "b")],
                    ),
                ],
            ),
            tree_at.clone(),
        );
        let range = (tree.find_pos(2, true)?, tree.find_pos(4, true)?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(10), None)?;
        assert_eq!(r#"<root><p>ab</p></root>"#, tree.to_xml());

        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("t", CrdtElement::tree(tree))],
        ));
        let roundtripped = roundtrip_root_json_element(&root)?;
        let tree = roundtripped.tree_by_created_at(&tree_at).unwrap();

        assert_eq!(r#"<root><p>ab</p></root>"#, tree.to_xml());
        let first_p = tree.root().all_children().next().unwrap();
        let moved_child = first_p
            .all_children()
            .find(|child| child.merged_from().is_some())
            .expect("moved child should carry merge source");
        assert!(moved_child.merged_at().is_some());
        let second_p = tree.root().all_children().nth(1).unwrap();
        assert!(second_p.is_removed());
        assert_eq!(Some(first_p.id()), second_p.merged_into());
        Ok(())
    }

    #[test]
    fn preserves_object_gc_elements_across_json_element_roundtrip() -> Result<()> {
        let mut root = CrdtRoot::create();
        let object_at = ticket(1);
        let first_at = ticket(2);
        let second_at = ticket(3);

        root.set_object_member(
            &TimeTicket::initial(),
            "o",
            CrdtElement::object(CrdtObject::create(object_at.clone())),
            object_at.clone(),
        )?;
        root.set_object_member(
            &object_at,
            "1",
            primitive_str("a", first_at.clone()),
            first_at,
        )?;
        root.set_object_member(
            &object_at,
            "1",
            primitive_str("b", second_at.clone()),
            second_at.clone(),
        )?;

        assert_eq!(r#"{"o":{"1":"b"}}"#, root.to_sorted_json());
        assert_eq!(1, root.stats().gc_elements);
        assert_eq!(
            Some(&second_at),
            root.find_by_created_at(&ticket(2)).unwrap().removed_at()
        );

        let roundtripped = roundtrip_root_json_element(&root)?;

        assert_eq!(r#"{"o":{"1":"b"}}"#, roundtripped.to_sorted_json());
        assert_eq!(1, roundtripped.stats().gc_elements);
        assert_eq!(1, roundtripped.get_garbage_len());
        assert_eq!(
            Some(&second_at),
            roundtripped
                .find_by_created_at(&ticket(2))
                .unwrap()
                .removed_at()
        );
        Ok(())
    }

    /// JS `restore_converter_test.ts` "round-trips a restore operation over
    /// the wire" and "round-trips the companion retombstone_spans of a
    /// replace reverse", at the wire projection: both span lists and the
    /// mode come back, and the decoded edit is an undo.
    #[test]
    fn carries_an_identity_edit_through_wire_shape() -> Result<()> {
        let operation = identity_edit(
            vec![restore_span(2, 4, "CD"), restore_span(4, 6, "45")],
            RestoreMode::Restore,
            vec![restore_span(0, 2, "12")],
        );

        let wire = WireOperation::try_from(&Operation::Edit(operation.clone()))?;
        let WireOperation::Edit {
            content,
            restore_spans,
            restore_mode,
            retombstone_spans,
            ..
        } = &wire
        else {
            panic!("expected an edit");
        };
        assert_eq!("", content);
        assert_eq!(Some(WireRestoreMode::Restore), *restore_mode);
        assert_eq!(
            vec![(2, 4, "CD"), (4, 6, "45")],
            restore_spans
                .iter()
                .map(|span| (span.start, span.end, span.content.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(1, retombstone_spans.len());
        assert_eq!(ticket(1), retombstone_spans[0].created_at);

        let Operation::Edit(decoded) = Operation::try_from(wire)? else {
            panic!("expected an edit");
        };
        assert_eq!(operation, decoded);
        Ok(())
    }

    /// JS `restore_converter_test.ts` "round-trips a retombstone operation".
    #[test]
    fn carries_a_retombstone_edit_through_wire_shape() -> Result<()> {
        let operation = identity_edit(
            vec![restore_span(4, 6, "45")],
            RestoreMode::Retombstone,
            Vec::new(),
        );

        let wire = WireOperation::try_from(&Operation::Edit(operation.clone()))?;
        assert!(matches!(
            &wire,
            WireOperation::Edit {
                restore_mode: Some(WireRestoreMode::Retombstone),
                ..
            }
        ));
        let Operation::Edit(decoded) = Operation::try_from(wire)? else {
            panic!("expected an edit");
        };
        assert_eq!(Some(RestoreMode::Retombstone), decoded.restore_mode());
        assert_eq!(operation, decoded);
        Ok(())
    }

    /// JS `restore_converter_test.ts` "keeps span attributes, stamped with
    /// executedAt": the wire carries a bare map, so the decoder stamps it
    /// with the operation's `executed_at`.
    #[test]
    fn stamps_span_attributes_with_executed_at() -> Result<()> {
        let mut styled = restore_span(0, 1, "X");
        styled.value.set_attr("b", "1", ticket(1));
        let operation = identity_edit(Vec::new(), RestoreMode::Restore, vec![styled]);

        let wire = WireOperation::try_from(&Operation::Edit(operation))?;
        let Operation::Edit(decoded) = Operation::try_from(wire)? else {
            panic!("expected an edit");
        };
        let value = &decoded.retombstone_spans()[0].value;
        assert_eq!(
            BTreeMap::from([("b".to_owned(), "1".to_owned())]),
            value.get_attributes()
        );
        for node in value.attributes().iter() {
            assert_eq!(&ticket(4), node.updated_at());
        }
        Ok(())
    }

    /// JS `restore_converter_test.ts` "leaves ordinary edits without a
    /// restore payload": no spans and no mode, so the bytes stay as they
    /// were, and the decoded edit is no undo.
    #[test]
    fn leaves_an_ordinary_edit_without_a_restore_payload() -> Result<()> {
        let operation = EditOperation::create(
            TimeTicket::initial(),
            head_pos(),
            head_pos(),
            "hi",
            BTreeMap::new(),
            Some(ticket(4)),
        );

        let wire = WireOperation::try_from(&Operation::Edit(operation.clone()))?;
        assert!(matches!(
            &wire,
            WireOperation::Edit { restore_spans, restore_mode: None, retombstone_spans, .. }
                if restore_spans.is_empty() && retombstone_spans.is_empty()
        ));
        let Operation::Edit(decoded) = Operation::try_from(wire)? else {
            panic!("expected an edit");
        };
        assert_eq!(None, decoded.restore_mode());
        assert_eq!(operation, decoded);
        Ok(())
    }

    /// JS `toOperation` writes the mode only with a span: an identity edit
    /// left with no spans goes out, and comes back, as a plain edit.
    #[test]
    fn writes_no_mode_without_a_span() -> Result<()> {
        let operation = identity_edit(Vec::new(), RestoreMode::Retombstone, Vec::new());

        let wire = WireOperation::try_from(&Operation::Edit(operation))?;
        assert!(matches!(
            &wire,
            WireOperation::Edit {
                restore_mode: None,
                ..
            }
        ));
        let Operation::Edit(decoded) = Operation::try_from(wire)? else {
            panic!("expected an edit");
        };
        assert_eq!(None, decoded.restore_mode());
        Ok(())
    }

    /// JS `restore_converter_test.ts` "rejects a malformed span" (inverted
    /// interval, interval not covering the content). A missing `created_at`
    /// and a negative start cannot reach the wire projection; the protocol
    /// converter rejects them. The length is in UTF-16 code units: an emoji
    /// is two.
    #[test]
    fn rejects_a_malformed_span_in_wire_shape() -> Result<()> {
        let decode = |span: WireRestoreSpan| {
            Operation::try_from(WireOperation::Edit {
                parent_created_at: TimeTicket::initial(),
                from: WireTextNodePos::from(&head_pos()),
                to: WireTextNodePos::from(&head_pos()),
                content: String::new(),
                attributes: BTreeMap::new(),
                executed_at: ticket(4),
                restore_spans: vec![span],
                restore_mode: Some(WireRestoreMode::Restore),
                retombstone_spans: Vec::new(),
            })
        };
        let span = |start, end, content: &str| WireRestoreSpan {
            created_at: ticket(1),
            start,
            end,
            content: content.to_owned(),
            attributes: BTreeMap::new(),
        };

        assert!(decode(span(0, 2, "\u{1F600}")).is_ok());
        for malformed in [
            span(6, 4, "45"),
            span(4, 9, "45"),
            span(0, 1, "\u{1F600}"),
            span(0, 4, "\u{1F600}"),
        ] {
            assert!(matches!(
                decode(malformed.clone()),
                Err(YorkieError::InvalidArgument(_))
            ));
        }
        Ok(())
    }

    /// JS `toTextNodes` writes no `removal_events` for a node whose
    /// `removed_at` says it all, so a character removed once and never
    /// restored encodes as before.
    #[test]
    fn writes_no_removal_events_when_removed_at_says_it_all() -> Result<()> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "abc", None, ticket(2), None)?;
        text.edit_by_index(1, 2, "", None, ticket(3), None)?;

        let WireJsonElement::Text { nodes, .. } = wire_text_from_domain(&text) else {
            panic!("expected a text");
        };
        assert_eq!(Some(&ticket(3)), nodes[1].removed_at.as_ref());
        assert!(nodes.iter().all(|node| node.removal_events.is_empty()));
        Ok(())
    }

    /// JS `toTextNodes`/`fromTextNode`: a ledger beyond `removed_at` survives
    /// a snapshot, in ticket order.
    #[test]
    fn carries_a_removal_ledger_through_wire_shape() -> Result<()> {
        let text = ledgered_text()?;
        let WireJsonElement::Text { nodes, .. } = wire_text_from_domain(&text) else {
            panic!("expected a text");
        };
        assert_eq!(vec![ticket(3), ticket(4)], nodes[1].removal_events);

        let CrdtElement::Text(decoded) = CrdtElement::try_from(wire_text_from_domain(&text))?
        else {
            panic!("expected a text");
        };
        assert_eq!(
            text.nodes()
                .map(|node| node.removal_events())
                .collect::<Vec<_>>(),
            decoded
                .nodes()
                .map(|node| node.removal_events())
                .collect::<Vec<_>>()
        );
        assert!(decoded.has_cancelled_removals());
        Ok(())
    }

    /// JS `fromTextNode`: a ledger `setRemovalEvents` refuses is dropped and
    /// the node keeps `removed_at` alone; the snapshot still decodes.
    #[test]
    fn drops_a_removal_ledger_that_breaks_the_rules() -> Result<()> {
        let node = |removed_at: Option<TimeTicket>, removal_events: Vec<TimeTicket>| WireTextNode {
            id: WireTextNodeId {
                created_at: ticket(2),
                offset: 0,
            },
            value: "a".to_owned(),
            removed_at,
            ins_prev_id: None,
            attributes: BTreeMap::new(),
            removal_events,
        };
        for (removed_at, events) in [
            // repeats removed_at
            (Some(ticket(5)), vec![ticket(5)]),
            // a removal that does not reproduce removed_at
            (None, vec![ticket(3)]),
        ] {
            let wire = WireJsonElement::Text {
                nodes: vec![node(removed_at.clone(), events)],
                created_at: ticket(1),
                moved_at: None,
                removed_at: None,
            };
            let CrdtElement::Text(text) = CrdtElement::try_from(wire)? else {
                panic!("expected a text");
            };
            let decoded = text.nodes().next().expect("the decoded node");
            assert_eq!(removed_at.as_ref(), decoded.removed_at());
            assert!(decoded.removal_events().is_empty());
        }
        Ok(())
    }

    /// JS `CRDTRoot` registers every element of a loaded snapshot, ledger
    /// holders included, so GC prunes a ledger a snapshot carried in.
    #[test]
    fn prunes_a_removal_ledger_a_snapshot_carried_in() -> Result<()> {
        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("t", CrdtElement::text(ledgered_text()?))],
        ));
        let mut root = roundtrip_root_json_element(&root)?;
        let events = |root: &CrdtRoot| {
            root.text_by_created_at(&ticket(1))
                .expect("the text")
                .nodes()
                .flat_map(|node| node.removal_events())
                .collect::<Vec<_>>()
        };
        assert_eq!(vec![ticket(3), ticket(4)], events(&root));

        let mut vector = VersionVector::new();
        vector.set("000000000000000000000001", 5);
        root.garbage_collect(&vector);
        assert!(events(&root).is_empty());
        Ok(())
    }

    /// JS `should round-trip a tombstoned text attribute`
    /// (snapshot_converter_test.ts, yorkie-js-sdk#1364): `toTextNodes` writes
    /// a text attribute's `isRemoved` and `fromTextNode` restores it, so the
    /// tombstone an undone style leaves stays a tombstone and a GC pair.
    ///
    /// The document is the one of the JS `removed_text_attr_snapshot_pack`
    /// fixture (`tools/interop/text_undo_fixtures_test.ts`), and its Text
    /// projects to the attribute JS wrote; `yorkie-protocol` checks that
    /// this projection encodes to the fixture's bytes.
    #[test]
    fn roundtrips_a_tombstoned_text_attribute() -> Result<()> {
        const A1: &str = "000000000000000000000001";
        let mut doc = crate::Document::new("text-undo-interop");
        doc.set_actor(A1);
        doc.update(|root| root.set_text("t")?.edit(0, 0, "hello", None).map(|_| ()))?;
        doc.update(|root| {
            let bold = BTreeMap::from([("bold".to_owned(), serde_json::Value::from("true"))]);
            root.get_text_mut("t")?.set_style(0, 5, bold)
        })?;

        // `bold` did not exist before, so the reverse removes it, leaving a
        // tombstone rather than dropping it.
        assert!(doc.history().can_undo());
        doc.history_mut().undo()?;

        let live_root = doc.crdt_root_for_test();
        assert_eq!(r#"{"t":[{"val":"hello"}]}"#, live_root.to_sorted_json());
        assert_eq!(1, doc.garbage_len());
        let baseline = live_root.deepcopy();

        let WireJsonElement::Object { nodes: members, .. } =
            WireJsonElement::try_from(&live_root.root_element())?
        else {
            panic!("expected an object");
        };
        let WireJsonElement::Text { nodes, .. } = &members[0].element else {
            panic!("expected a text");
        };
        assert_eq!(
            vec![BTreeMap::from([(
                "bold".to_owned(),
                WireNodeAttr {
                    value: "\"true\"".to_owned(),
                    updated_at: TimeTicket::new(3, 1, A1),
                    is_removed: true,
                },
            )])],
            nodes
                .iter()
                .map(|node| node.attributes.clone())
                .collect::<Vec<_>>()
        );

        let mut rebuilt = roundtrip_root_json_element(live_root)?;
        assert_eq!(live_root.to_sorted_json(), rebuilt.to_sorted_json());
        assert_eq!(doc.garbage_len(), rebuilt.get_garbage_len());
        assert_eq!(doc.doc_size().gc, rebuilt.doc_size().gc);
        assert_eq!(baseline.doc_size().live, rebuilt.doc_size().live);

        let mut vector = VersionVector::new();
        vector.set(A1, i64::MAX);
        assert_eq!(1, rebuilt.garbage_collect(&vector));
        assert_eq!(0, rebuilt.get_garbage_len());
        Ok(())
    }

    /// `abc` whose `b` was removed at 3 and restored at 4.
    fn ledgered_text() -> Result<CrdtText> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "abc", None, ticket(2), None)?;
        text.edit_by_index(1, 2, "", None, ticket(3), None)?;
        let b = RestoreSpan {
            created_at: ticket(2),
            start: 1,
            end: 2,
            value: TextValue::create("b"),
        };
        text.restore(&[b], &ticket(4), None)?;
        Ok(text)
    }

    fn identity_edit(
        restore_spans: Vec<RestoreSpan<TextValue>>,
        mode: RestoreMode,
        retombstone_spans: Vec<RestoreSpan<TextValue>>,
    ) -> EditOperation {
        EditOperation::new(
            TimeTicket::initial(),
            head_pos(),
            head_pos(),
            "",
            BTreeMap::new(),
            Some(ticket(4)),
            true,
        )
        .with_identity(restore_spans, mode, retombstone_spans)
    }

    fn restore_span(start: usize, end: usize, content: &str) -> RestoreSpan<TextValue> {
        RestoreSpan {
            created_at: ticket(1),
            start,
            end,
            value: TextValue::create(content),
        }
    }

    fn head_pos() -> RgaTreeSplitPos {
        RgaTreeSplitPos::new(RgaTreeSplitNodeId::new(ticket(1), 0), 0)
    }

    fn roundtrip_root_json_element(root: &CrdtRoot) -> Result<CrdtRoot> {
        let root_element = root.root_element();
        let wire = WireJsonElement::try_from(&root_element)?;
        let element = CrdtElement::try_from(wire)?;
        let CrdtElement::Object(object) = element else {
            return Err(YorkieError::UnsupportedProtocolConversion(
                "root JSONElement must be object",
            ));
        };

        Ok(CrdtRoot::new(*object))
    }

    fn primitive_bool(value: bool, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::Boolean(value),
            created_at,
        ))
    }

    fn primitive_i32(value: i32, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::Integer(value),
            created_at,
        ))
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, "000000000000000000000001")
    }

    fn tree_node_id(lamport: i64, offset: usize) -> TreeNodeId {
        TreeNodeId::new(ticket(lamport), offset)
    }
}
