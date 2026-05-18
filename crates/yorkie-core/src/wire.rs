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

impl TryFrom<WireChangePack> for ChangePack {
    type Error = YorkieError;

    fn try_from(pack: WireChangePack) -> Result<Self> {
        let changes = pack
            .changes
            .into_iter()
            .map(Change::try_from)
            .collect::<Result<Vec<_>>>()?;

        Ok(Self::create(
            pack.document_key,
            pack.checkpoint,
            pack.is_removed,
            changes,
            pack.version_vector,
            pack.snapshot,
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
            CrdtElement::Object(object) => Ok(Self::Object {
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
            }),
            CrdtElement::Array(array) => Ok(Self::Array {
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
            }),
            CrdtElement::Primitive(primitive) => Ok(Self::Primitive {
                value_type: primitive_value_type(primitive.value()),
                value: primitive.to_bytes(),
                created_at: primitive.created_at().clone(),
                moved_at: primitive.moved_at().cloned(),
                removed_at: primitive.removed_at().cloned(),
            }),
            CrdtElement::Text(text) => Ok(Self::Text {
                nodes: text.nodes().map(wire_text_node_from_domain).collect(),
                created_at: text.created_at().clone(),
                moved_at: text.moved_at().cloned(),
                removed_at: text.removed_at().cloned(),
            }),
            CrdtElement::Counter(counter) => Ok(Self::Counter {
                value_type: counter_value_type(counter.counter_type()),
                value: counter.to_bytes(),
                hll_registers: counter.hll_bytes().unwrap_or_default(),
                created_at: counter.created_at().clone(),
                moved_at: counter.moved_at().cloned(),
                removed_at: counter.removed_at().cloned(),
            }),
            CrdtElement::Tree(tree) => Ok(Self::Tree {
                nodes: wire_tree_nodes_from_domain(tree.root()),
                created_at: tree.created_at().clone(),
                moved_at: tree.moved_at().cloned(),
                removed_at: tree.removed_at().cloned(),
            }),
        }
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
            } => {
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
            WireJsonElement::Array {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => {
                let mut elements = RgaTreeList::new();
                for node in nodes {
                    match (node.element, node.position_moved_at) {
                        (None, _) => {
                            let position_created_at = node.position_created_at.ok_or(
                                YorkieError::UnsupportedProtocolConversion(
                                    "dead RGA node without position_created_at",
                                ),
                            )?;
                            let position_removed_at = node.position_removed_at.ok_or(
                                YorkieError::UnsupportedProtocolConversion(
                                    "dead RGA node without position_removed_at",
                                ),
                            )?;
                            elements.add_dead_position(position_created_at, position_removed_at);
                        }
                        (Some(element), Some(position_moved_at)) => {
                            let position_created_at = node.position_created_at.ok_or(
                                YorkieError::UnsupportedProtocolConversion(
                                    "moved RGA node without position_created_at",
                                ),
                            )?;
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
            WireJsonElement::Primitive {
                value_type,
                value,
                created_at,
                moved_at,
                removed_at,
            } => {
                let primitive_type = wire_value_type_to_primitive_type(value_type)?;
                let value = CrdtPrimitive::value_from_bytes(primitive_type, &value)?;
                Ok(with_element_meta(
                    CrdtElement::primitive(CrdtPrimitive::new(value, created_at)),
                    moved_at,
                    removed_at,
                ))
            }
            WireJsonElement::Text {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => Ok(with_element_meta(
                CrdtElement::text(text_from_wire_nodes(created_at, nodes)?),
                moved_at,
                removed_at,
            )),
            WireJsonElement::Counter {
                value_type,
                value,
                hll_registers,
                created_at,
                moved_at,
                removed_at,
            } => {
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
            WireJsonElement::Tree {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => Ok(with_element_meta(
                CrdtElement::tree(CrdtTree::new(wire_tree_nodes_to_domain(nodes)?, created_at)),
                moved_at,
                removed_at,
            )),
        }
    }
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
