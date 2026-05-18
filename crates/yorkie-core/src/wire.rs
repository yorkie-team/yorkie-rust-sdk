use crate::change::{Change, ChangeId, ChangePack, Checkpoint};
use crate::crdt::array::CrdtArray;
use crate::crdt::counter::{CounterType, CrdtCounter};
use crate::crdt::element::CrdtElement;
use crate::crdt::element_rht::ElementRht;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveType, PrimitiveValue};
use crate::crdt::rga_tree_list::RgaTreeList;
use crate::crdt::rga_tree_split::{
    RgaTreeSplit, RgaTreeSplitNode, RgaTreeSplitNodeId, RgaTreeSplitPos,
};
use crate::crdt::rht::Rht;
use crate::crdt::text::{CrdtText, TextValue};
use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId, TreePos};
use crate::operation::{
    AddOperation, ArraySetOperation, EditOperation, IncreaseOperation, MoveOperation, Operation,
    RemoveOperation, SetOperation, StyleOperation, TreeEditOperation, TreeStyleOperation,
};
use crate::{Result, TimeTicket, VersionVector, YorkieError};
use std::collections::BTreeMap;

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
    pub snapshot_root: Option<WireJsonElement>,
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

        Ok(Self::create_with_snapshot_root(
            pack.document_key,
            pack.checkpoint,
            pack.is_removed,
            changes,
            pack.version_vector,
            pack.snapshot,
            snapshot_root,
        ))
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

impl TryFrom<WireChange> for Change {
    type Error = YorkieError;

    fn try_from(change: WireChange) -> Result<Self> {
        let operations = change
            .operations
            .into_iter()
            .map(Operation::try_from)
            .collect::<Result<Vec<_>>>()?;

        Ok(Self::create(
            ChangeId::from(change.id),
            operations,
            change.message,
        ))
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
            Operation::Edit(operation) => Ok(Self::Edit {
                parent_created_at: operation.parent_created_at().clone(),
                from: WireTextNodePos::from(operation.from_pos()),
                to: WireTextNodePos::from(operation.to_pos()),
                content: operation.content().to_owned(),
                attributes: operation.attributes().clone(),
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::Style(operation) => Ok(Self::Style {
                parent_created_at: operation.parent_created_at().clone(),
                from: WireTextNodePos::from(operation.from_pos()),
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
                from: WireTreePos::from(operation.from_pos()),
                to: WireTreePos::from(operation.to_pos()),
                contents: operation
                    .contents()
                    .map(|contents| contents.iter().map(wire_tree_nodes_from_domain).collect()),
                split_level: operation.split_level(),
                executed_at: operation.executed_at()?.clone(),
            }),
            Operation::TreeStyle(operation) => Ok(Self::TreeStyle {
                parent_created_at: operation.parent_created_at().clone(),
                from: WireTreePos::from(operation.from_pos()),
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
                CrdtElement::try_from(value)?,
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
                CrdtElement::try_from(value)?,
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
            } => Ok(Self::Edit(EditOperation::create(
                parent_created_at,
                RgaTreeSplitPos::from(from),
                RgaTreeSplitPos::from(to),
                content,
                attributes,
                Some(executed_at),
            ))),
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
            } => {
                let contents = contents
                    .map(|contents| {
                        contents
                            .into_iter()
                            .map(wire_tree_nodes_to_domain)
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?;
                Ok(Self::TreeEdit(TreeEditOperation::create(
                    parent_created_at,
                    TreePos::from(from),
                    TreePos::from(to),
                    contents,
                    split_level,
                    Some(executed_at),
                )))
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
                CrdtElement::try_from(value)?,
                Some(executed_at),
            ))),
        }
    }
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
    WireJsonElement::Text {
        nodes: text.nodes().map(wire_text_node_from_domain).collect(),
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

fn wire_text_node_from_domain(node: &RgaTreeSplitNode<TextValue>) -> WireTextNode {
    WireTextNode {
        id: WireTextNodeId::from(node.id()),
        value: node.value().content().to_owned(),
        removed_at: node.removed_at().cloned(),
        ins_prev_id: node.ins_prev_id().map(WireTextNodeId::from),
        attributes: text_attrs_from_domain(node.value().attributes()),
    }
}

fn text_attrs_from_domain(attrs: &Rht) -> BTreeMap<String, WireNodeAttr> {
    attrs
        .iter()
        .map(|node| {
            (
                node.key().to_owned(),
                WireNodeAttr {
                    value: node.value().to_owned(),
                    updated_at: node.updated_at().clone(),
                    is_removed: false,
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

    for node in nodes {
        let mut text_node = RgaTreeSplitNode::new(
            RgaTreeSplitNodeId::from(node.id),
            TextValue::new(node.value, text_attrs_to_domain(node.attributes)),
        );
        text_node.set_removed_at(node.removed_at);
        let inserted_id = split.insert_after_id(&prev_id, text_node)?;
        if let Some(ins_prev_id) = node.ins_prev_id {
            let current = split.find_node_mut_by_id(&inserted_id).ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "node not found for {}",
                    inserted_id.to_test_string()
                ))
            })?;
            current.set_ins_prev_id(Some(RgaTreeSplitNodeId::from(ins_prev_id)));
        }
        prev_id = inserted_id;
    }

    Ok(CrdtText::new(created_at, split))
}

fn text_attrs_to_domain(attrs: BTreeMap<String, WireNodeAttr>) -> Rht {
    let mut rht = Rht::new();
    for (key, attr) in attrs {
        rht.set(key, attr.value, attr.updated_at);
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
