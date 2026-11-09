//! Re-issue of the tickets a never-synced document minted under one actor to
//! another actor (JS `src/api/reissue.ts`, Go `api/converter/reissue.go`).
//!
//! JS walks the protobuf message by reflection so that a new ticket field
//! cannot fall out of the walk. Rust walks the `wire` projection instead,
//! destructuring every struct and variant without `..`: a new field there
//! fails to compile here until the walk visits it. Nested Object, Array and
//! Tree values are structural in the projection
//! (`WireJsonElementSimple.element`), so there are no `JSONElement` bytes to
//! decode and re-encode as JS `walkNestedElement` does.

use crate::crdt::element::CrdtElement;
use crate::operation::Operation;
use crate::wire::{
    WireJsonElement, WireJsonElementSimple, WireNodeAttr, WireOperation, WireRestoreSpan,
    WireRgaNode, WireRhtNode, WireTextNode, WireTextNodeId, WireTextNodePos, WireTreeNode,
    WireTreeNodeId, WireTreePos,
};
use crate::{ActorId, Result, TimeTicket, VersionVector, INITIAL_LAMPORT};
use std::collections::BTreeMap;

/// Returns copies of the given operations in which every ticket issued by
/// the actor `from` names the actor `to` instead (JS `reissueOperations`).
/// Lamports and delimiters are kept, so the order among the tickets is
/// unchanged. A ticket with lamport 0 is never re-issued: that is the initial
/// ticket, the root object's and every sentinel node's identity, shared by
/// all replicas.
///
/// It goes through the wire projection on purpose: the operations come back
/// as the server would decode them (except a Text value, see
/// `TicketReissuer::reissue_text_value`). It is only sound when every ticket
/// naming `from` was issued locally and has never left this replica, which
/// is the case for a document that has never synced.
pub(crate) fn reissue_operations(
    ops: &[Operation],
    from: &ActorId,
    to: &ActorId,
) -> Result<Vec<Operation>> {
    let reissuer = TicketReissuer { from, to };
    ops.iter()
        .map(|op| {
            let mut wire = WireOperation::try_from(op)?;
            reissuer.walk_operation(&mut wire);
            let decoded = Operation::try_from(wire)?;
            reissuer.reissue_text_value(op, decoded)
        })
        .collect()
}

/// Returns a copy of the given vector whose entry for `from` is moved to
/// `to`, keeping the larger lamport if `to` already has one (JS
/// `reissueVersionVector` in `document.ts`).
pub(crate) fn reissue_version_vector(
    vector: &VersionVector,
    from: &ActorId,
    to: &ActorId,
) -> VersionVector {
    let mut reissued = vector.clone();
    let Some(lamport) = reissued.get(from.as_str()) else {
        return reissued;
    };

    reissued.unset(from.as_str());
    let lamport = reissued
        .get(to.as_str())
        .map_or(lamport, |existing| existing.max(lamport));
    reissued.set(to.clone(), lamport);
    reissued
}

/// Rewrites the actor of the tickets in a wire projection, in place (JS
/// `TicketReissuer`).
struct TicketReissuer<'a> {
    from: &'a ActorId,
    to: &'a ActorId,
}

impl TicketReissuer<'_> {
    /// Returns the decoded operation with its Text value replaced by a
    /// re-issued copy of the original one, content included. Any other
    /// operation is returned unchanged.
    ///
    /// The wire drops a Text value's content: it carries the Text alone and
    /// the Edits that fill it. A Set/Add/ArraySet that restores a removed
    /// Text -- the reverse of a Remove, run by Undo -- carries the content,
    /// and a later Edit in the same document may target its nodes. Such a
    /// value is re-issued through its full snapshot encoding instead, so the
    /// local root rebuilt from these operations keeps what the user sees.
    fn reissue_text_value(&self, orig: &Operation, mut decoded: Operation) -> Result<Operation> {
        let value = match orig {
            Operation::Set(op) => op.value(),
            Operation::Add(op) => op.value(),
            Operation::ArraySet(op) => op.value(),
            _ => return Ok(decoded),
        };
        if !matches!(value, CrdtElement::Text(_)) {
            return Ok(decoded);
        }

        let mut element = WireJsonElement::try_from(value)?;
        self.walk_element(&mut element);
        let text = CrdtElement::try_from(element)?;

        // The decoded operation keeps its key, parent, prev, created_at and
        // executed_at; only the value is swapped (JS rebuilds it with
        // `create` from those same fields).
        match &mut decoded {
            Operation::Set(op) => *op.value_mut() = text,
            Operation::Add(op) => *op.value_mut() = text,
            Operation::ArraySet(op) => *op.value_mut() = text,
            _ => {}
        }
        Ok(decoded)
    }

    /// Re-issues the ticket iff it names `from` with a non-zero lamport.
    fn ticket(&self, ticket: &mut TimeTicket) {
        if ticket.lamport() != INITIAL_LAMPORT && ticket.actor_id() == self.from {
            *ticket = ticket.set_actor(self.to.clone());
        }
    }

    fn opt_ticket(&self, ticket: &mut Option<TimeTicket>) {
        if let Some(ticket) = ticket {
            self.ticket(ticket);
        }
    }

    fn walk_operation(&self, operation: &mut WireOperation) {
        match operation {
            WireOperation::Set {
                parent_created_at,
                key: _,
                value,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.walk_element_simple(value);
                self.ticket(executed_at);
            }
            WireOperation::Add {
                parent_created_at,
                prev_created_at,
                value,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.ticket(prev_created_at);
                self.walk_element_simple(value);
                self.ticket(executed_at);
            }
            WireOperation::Move {
                parent_created_at,
                prev_created_at,
                created_at,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.ticket(prev_created_at);
                self.ticket(created_at);
                self.ticket(executed_at);
            }
            WireOperation::Remove {
                parent_created_at,
                created_at,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.ticket(created_at);
                self.ticket(executed_at);
            }
            WireOperation::Edit {
                parent_created_at,
                from,
                to,
                content: _,
                attributes: _,
                executed_at,
                restore_spans,
                restore_mode: _,
                retombstone_spans,
            } => {
                self.ticket(parent_created_at);
                self.walk_text_pos(from);
                self.walk_text_pos(to);
                self.ticket(executed_at);
                for span in restore_spans.iter_mut().chain(retombstone_spans) {
                    self.walk_span(span);
                }
            }
            WireOperation::Style {
                parent_created_at,
                from,
                to,
                attributes: _,
                attributes_to_remove: _,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.walk_text_pos(from);
                self.walk_text_pos(to);
                self.ticket(executed_at);
            }
            // `actor` is the dedup counter's caller-supplied identity, a
            // string the JS walk does not reach either.
            WireOperation::Increase {
                parent_created_at,
                value,
                executed_at,
                actor: _,
            } => {
                self.ticket(parent_created_at);
                self.walk_element_simple(value);
                self.ticket(executed_at);
            }
            WireOperation::TreeEdit {
                parent_created_at,
                from,
                to,
                contents,
                split_level: _,
                executed_at,
                split_tickets,
            } => {
                self.ticket(parent_created_at);
                self.walk_tree_pos(from);
                self.walk_tree_pos(to);
                for node in contents.iter_mut().flatten().flatten() {
                    self.walk_tree_node(node);
                }
                self.ticket(executed_at);
                for ticket in split_tickets {
                    self.ticket(ticket);
                }
            }
            WireOperation::TreeStyle {
                parent_created_at,
                from,
                to,
                attributes: _,
                attributes_to_remove: _,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.walk_tree_pos(from);
                self.walk_tree_pos(to);
                self.ticket(executed_at);
            }
            WireOperation::ArraySet {
                parent_created_at,
                created_at,
                value,
                executed_at,
            } => {
                self.ticket(parent_created_at);
                self.ticket(created_at);
                self.walk_element_simple(value);
                self.ticket(executed_at);
            }
        }
    }

    /// Walks an element as a Set/Add/ArraySet/Increase carries it. `value`
    /// holds a primitive or counter value, never a ticket; an Object, Array
    /// or Tree travels in `element` (JS `walkNestedElement`).
    fn walk_element_simple(&self, element: &mut WireJsonElementSimple) {
        let WireJsonElementSimple {
            created_at,
            moved_at,
            removed_at,
            value_type: _,
            value: _,
            element,
        } = element;
        self.ticket(created_at);
        self.opt_ticket(moved_at);
        self.opt_ticket(removed_at);
        if let Some(element) = element {
            self.walk_element(element);
        }
    }

    fn walk_element(&self, element: &mut WireJsonElement) {
        match element {
            WireJsonElement::Object {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => {
                // Map keys are left alone (JS): an object member is keyed by
                // name.
                for WireRhtNode { key: _, element } in nodes {
                    self.walk_element(element);
                }
                self.walk_meta(created_at, moved_at, removed_at);
            }
            WireJsonElement::Array {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => {
                for WireRgaNode {
                    element,
                    position_created_at,
                    position_moved_at,
                    position_removed_at,
                } in nodes
                {
                    if let Some(element) = element {
                        self.walk_element(element);
                    }
                    self.opt_ticket(position_created_at);
                    self.opt_ticket(position_moved_at);
                    self.opt_ticket(position_removed_at);
                }
                self.walk_meta(created_at, moved_at, removed_at);
            }
            WireJsonElement::Primitive {
                value_type: _,
                value: _,
                created_at,
                moved_at,
                removed_at,
            } => self.walk_meta(created_at, moved_at, removed_at),
            WireJsonElement::Text {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => {
                for node in nodes {
                    self.walk_text_node(node);
                }
                self.walk_meta(created_at, moved_at, removed_at);
            }
            WireJsonElement::Counter {
                value_type: _,
                value: _,
                hll_registers: _,
                created_at,
                moved_at,
                removed_at,
            } => self.walk_meta(created_at, moved_at, removed_at),
            WireJsonElement::Tree {
                nodes,
                created_at,
                moved_at,
                removed_at,
            } => {
                for node in nodes {
                    self.walk_tree_node(node);
                }
                self.walk_meta(created_at, moved_at, removed_at);
            }
        }
    }

    fn walk_meta(
        &self,
        created_at: &mut TimeTicket,
        moved_at: &mut Option<TimeTicket>,
        removed_at: &mut Option<TimeTicket>,
    ) {
        self.ticket(created_at);
        self.opt_ticket(moved_at);
        self.opt_ticket(removed_at);
    }

    fn walk_text_node(&self, node: &mut WireTextNode) {
        let WireTextNode {
            id,
            value: _,
            removed_at,
            ins_prev_id,
            attributes,
            removal_events,
        } = node;
        self.walk_text_node_id(id);
        self.opt_ticket(removed_at);
        if let Some(ins_prev_id) = ins_prev_id {
            self.walk_text_node_id(ins_prev_id);
        }
        self.walk_attrs(attributes);
        for ticket in removal_events {
            self.ticket(ticket);
        }
    }

    fn walk_tree_node(&self, node: &mut WireTreeNode) {
        let WireTreeNode {
            id,
            node_type: _,
            value: _,
            removed_at,
            ins_prev_id,
            ins_next_id,
            depth: _,
            attributes,
            merged_from,
            merged_at,
        } = node;
        self.walk_tree_node_id(id);
        self.opt_ticket(removed_at);
        for id in [ins_prev_id, ins_next_id, merged_from]
            .into_iter()
            .flatten()
        {
            self.walk_tree_node_id(id);
        }
        self.walk_attrs(attributes);
        self.opt_ticket(merged_at);
    }

    /// Walks a restore span. Its attributes are a bare map the decoder
    /// stamps with the operation's `executed_at`; they hold no ticket.
    fn walk_span(&self, span: &mut WireRestoreSpan) {
        let WireRestoreSpan {
            created_at,
            start: _,
            end: _,
            content: _,
            attributes: _,
        } = span;
        self.ticket(created_at);
    }

    /// Walks node attributes. Their keys are names and are left alone (JS),
    /// even one that reads like the actor.
    fn walk_attrs(&self, attributes: &mut BTreeMap<String, WireNodeAttr>) {
        for WireNodeAttr {
            value: _,
            updated_at,
            is_removed: _,
        } in attributes.values_mut()
        {
            self.ticket(updated_at);
        }
    }

    fn walk_text_pos(&self, pos: &mut WireTextNodePos) {
        let WireTextNodePos {
            id,
            relative_offset: _,
        } = pos;
        self.walk_text_node_id(id);
    }

    fn walk_text_node_id(&self, id: &mut WireTextNodeId) {
        let WireTextNodeId {
            created_at,
            offset: _,
        } = id;
        self.ticket(created_at);
    }

    fn walk_tree_pos(&self, pos: &mut WireTreePos) {
        let WireTreePos {
            parent_id,
            left_sibling_id,
        } = pos;
        self.walk_tree_node_id(parent_id);
        self.walk_tree_node_id(left_sibling_id);
    }

    fn walk_tree_node_id(&self, id: &mut WireTreeNodeId) {
        let WireTreeNodeId {
            created_at,
            offset: _,
        } = id;
        self.ticket(created_at);
    }
}

#[cfg(test)]
#[path = "reissue_tests.rs"]
mod tests;
