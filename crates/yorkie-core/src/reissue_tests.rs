//! Tests of the ticket re-issue, ported from JS
//! `test/unit/api/reissue_test.ts` ("reissueOperations", `ed25355e`) and Go
//! `api/converter/reissue_test.go` (`1061bc35`).

use super::{reissue_operations, reissue_version_vector, TicketReissuer};
use crate::crdt::element::CrdtElement;
use crate::operation::Operation;
use crate::wire::{WireJsonElement, WireNodeAttr, WireOperation, WireTreeNode, WireTreeNodeId};
use crate::{
    ActorId, Document, JsonArray, JsonObject, Result, TimeTicket, TreeNodeInput, VersionVector,
    INITIAL_ACTOR_ID,
};
use std::collections::BTreeMap;

const ACTOR_A: &str = "000000000000000000000001";

fn initial() -> ActorId {
    ActorId::from(INITIAL_ACTOR_ID)
}

fn actor_a() -> ActorId {
    ActorId::from(ACTOR_A)
}

/// The operations of every local change of the document (JS `opsOf`).
fn ops_of(doc: &Document) -> Vec<Operation> {
    doc.create_change_pack()
        .changes()
        .iter()
        .flat_map(|change| change.operations().iter().cloned())
        .collect()
}

/// The Text a Set, Add or ArraySet carries, if any.
fn text_value(op: &Operation) -> Option<&CrdtElement> {
    let value = match op {
        Operation::Set(op) => op.value(),
        Operation::Add(op) => op.value(),
        Operation::ArraySet(op) => op.value(),
        _ => return None,
    };
    matches!(value, CrdtElement::Text(_)).then_some(value)
}

/// The `(actor, lamport)` of every ticket in a `Debug` rendering, in order.
/// Scanning the rendering sees every field without going through the walk
/// under test, as JS `ticketsOf` reflects over the protobuf message.
fn tickets_in(debug: &str) -> Vec<(String, i64)> {
    const LAMPORT: &str = "TimeTicket { lamport: ";
    const ACTOR: &str = "actor_id: ActorId(\"";
    let mut tickets = Vec::new();
    let mut rest = debug;
    while let Some(at) = rest.find(LAMPORT) {
        rest = &rest[at + LAMPORT.len()..];
        let lamport = rest[..rest.find(',').expect("a lamport")]
            .parse()
            .expect("a numeric lamport");
        rest = &rest[rest.find(ACTOR).expect("an actor") + ACTOR.len()..];
        let actor = &rest[..rest.find('"').expect("a quoted actor")];
        tickets.push((actor.to_owned(), lamport));
    }
    tickets
}

/// The tickets of the given operations as they travel (JS `wireTickets`),
/// plus those of a Text value's full encoding, which the operation's wire
/// form drops.
fn wire_tickets(ops: &[Operation]) -> Result<Vec<(String, i64)>> {
    let mut debug = String::new();
    for op in ops {
        debug.push_str(&format!("{:?}", WireOperation::try_from(op)?));
        if let Some(text) = text_value(op) {
            debug.push_str(&format!("{:?}", WireJsonElement::try_from(text)?));
        }
    }
    Ok(tickets_in(&debug))
}

/// Asserts that `after` is `before` with every ticket of the initial actor
/// but the lamport-0 ones re-issued to `ACTOR_A`.
fn assert_reissued(before: &[(String, i64)], after: &[(String, i64)]) {
    assert_eq!(before.len(), after.len());
    for ((actor, lamport), (before_actor, before_lamport)) in after.iter().zip(before) {
        assert_eq!(before_lamport, lamport);
        assert_eq!(INITIAL_ACTOR_ID, before_actor);
        // The lamport-0 ticket is the root's and every sentinel's identity,
        // shared by all replicas, and keeps the initial actor.
        let expected = if *lamport == 0 {
            INITIAL_ACTOR_ID
        } else {
            ACTOR_A
        };
        assert_eq!(expected, actor);
    }
}

fn tree_input() -> TreeNodeInput {
    TreeNodeInput::element(
        "doc",
        vec![TreeNodeInput::element("p", vec![TreeNodeInput::text("ab")])],
    )
}

/// JS `re-issues the tickets nested in Object, Array and Tree values`; Go
/// `re-issue tickets of a nested value and keep the lamport-0 root ticket`.
#[test]
fn reissues_the_tickets_nested_in_object_array_and_tree_values() -> Result<()> {
    let mut doc = Document::new("d");
    doc.update(|root| {
        let mut nested = JsonObject::new();
        nested.set("n", 2)?;
        let mut arr = JsonArray::new();
        arr.push(1)?;
        arr.push(nested)?;
        let mut obj = JsonObject::new();
        obj.set("k", "v")?;
        obj.set("arr", arr)?;
        root.set("obj", obj)?;
        root.set_tree("tree", tree_input()).map(|_| ())
    })?;
    let ops = ops_of(&doc);
    let before = wire_tickets(&ops)?;
    assert!(before
        .iter()
        .any(|(actor, lamport)| actor == INITIAL_ACTOR_ID && *lamport > 0));
    assert!(before.iter().any(|(_, lamport)| *lamport == 0));

    let after = wire_tickets(&reissue_operations(&ops, &initial(), &actor_a())?)?;
    assert_reissued(&before, &after);
    Ok(())
}

/// JS `does not mutate the given operations`.
#[test]
fn does_not_mutate_the_given_operations() -> Result<()> {
    let mut doc = Document::new("d");
    doc.update(|root| {
        let mut obj = JsonObject::new();
        obj.set("k", "v")?;
        root.set("obj", obj)?;
        root.set_text("t")?.edit(0, 0, "hello", None).map(|_| ())
    })?;
    let ops = ops_of(&doc);
    let before = ops.clone();
    let before_wire = ops
        .iter()
        .map(WireOperation::try_from)
        .collect::<Result<Vec<_>>>()?;

    reissue_operations(&ops, &initial(), &actor_a())?;
    assert_eq!(before, ops);
    assert_eq!(
        before_wire,
        ops.iter()
            .map(WireOperation::try_from)
            .collect::<Result<Vec<_>>>()?
    );
    Ok(())
}

/// JS `keeps the content of a Text value`: the wire drops a Text value's
/// content; a Set restoring a removed Text carries it in the value itself.
#[test]
fn keeps_the_content_of_a_text_value() -> Result<()> {
    let mut doc = Document::new("d");
    doc.update(|root| root.set_text("t")?.edit(0, 0, "hello", None).map(|_| ()))?;
    doc.update(|root| {
        root.remove("t");
        Ok(())
    })?;
    doc.history_mut().undo()?;
    let restore = ops_of(&doc).pop().expect("the restoring set");
    let Operation::Set(set) = &restore else {
        panic!("expected a set, got {restore:?}");
    };
    let CrdtElement::Text(text) = set.value() else {
        panic!("expected a text");
    };
    assert_eq!("hello", text.to_string());

    let reissued = reissue_operations(std::slice::from_ref(&restore), &initial(), &actor_a())?;
    let [Operation::Set(reissued)] = reissued.as_slice() else {
        panic!("expected one set, got {reissued:?}");
    };
    let CrdtElement::Text(text) = reissued.value() else {
        panic!("expected a text");
    };
    assert_eq!("hello", text.to_string());
    assert_eq!(&actor_a(), text.created_at().actor_id());
    assert_eq!(set.key(), reissued.key());
    let tickets = tickets_in(&format!(
        "{:?}",
        WireJsonElement::try_from(reissued.value())?
    ));
    assert!(tickets.len() > 1);
    assert!(tickets
        .iter()
        .all(|(actor, lamport)| *lamport == 0 || actor == ACTOR_A));
    Ok(())
}

/// JS `leaves an attribute named like the actor alone`.
#[test]
fn leaves_an_attribute_named_like_the_actor_alone() -> Result<()> {
    let mut doc = Document::new("d");
    doc.update(|root| root.set_tree("tree", tree_input()).map(|_| ()))?;
    doc.update(|root| {
        root.get_tree_mut("tree")?.style(
            0,
            1,
            BTreeMap::from([(INITIAL_ACTOR_ID.to_owned(), "v".into())]),
        )
    })?;
    let style = ops_of(&doc).pop().expect("the style");
    let Operation::TreeStyle(original) = &style else {
        panic!("expected a tree style, got {style:?}");
    };

    let reissued = reissue_operations(std::slice::from_ref(&style), &initial(), &actor_a())?;
    let [Operation::TreeStyle(reissued)] = reissued.as_slice() else {
        panic!("expected one tree style, got {reissued:?}");
    };
    assert_eq!(
        vec![INITIAL_ACTOR_ID],
        reissued
            .attributes()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert_eq!(original.attributes(), reissued.attributes());
    assert_eq!(
        &actor_a(),
        reissued.executed_at()?.actor_id(),
        "the ticket next to the attribute is re-issued"
    );
    Ok(())
}

/// No JS case: JS reaches these fields by reflection. An identity Edit's
/// `restore_spans` and `retombstone_spans`, a TreeEdit's `split_tickets` and
/// the `removal_events` of a restored Text value's node all come back
/// re-issued.
#[test]
fn reissues_the_tickets_of_spans_split_tickets_and_removal_events() -> Result<()> {
    let mut doc = Document::new("d");
    // An undone replace: restores `b` and re-removes `X`, by identity.
    doc.update(|root| root.set_text("t")?.edit(0, 0, "abc", None).map(|_| ()))?;
    doc.update(|root| root.get_text_mut("t")?.edit(1, 2, "X", None).map(|_| ()))?;
    doc.history_mut().undo()?;
    // A split of the text node and of `p`.
    doc.update(|root| root.set_tree("tree", tree_input()).map(|_| ()))?;
    doc.update(|root| {
        root.get_tree_mut("tree")?
            .edit_by_path(&[0, 1], &[0, 1], Vec::new(), 1)
    })?;
    // A styled Text whose `b` was removed and restored, removed and restored
    // whole.
    doc.update(|root| {
        root.set_text("u")?
            .edit(
                0,
                0,
                "abc",
                Some(BTreeMap::from([("b".to_owned(), "1".into())])),
            )
            .map(|_| ())
    })?;
    doc.update(|root| root.get_text_mut("u")?.edit(1, 2, "", None).map(|_| ()))?;
    doc.history_mut().undo()?;
    doc.update(|root| {
        root.remove("u");
        Ok(())
    })?;
    doc.history_mut().undo()?;

    let ops = ops_of(&doc);
    let reissued = reissue_operations(&ops, &initial(), &actor_a())?;
    assert_reissued(&wire_tickets(&ops)?, &wire_tickets(&reissued)?);

    let wire = reissued
        .iter()
        .map(WireOperation::try_from)
        .collect::<Result<Vec<_>>>()?;
    let mut spans = 0;
    let mut split_tickets = 0;
    for op in &wire {
        match op {
            WireOperation::Edit {
                restore_spans,
                retombstone_spans,
                ..
            } if !restore_spans.is_empty() && !retombstone_spans.is_empty() => {
                for span in restore_spans.iter().chain(retombstone_spans) {
                    assert_eq!(&actor_a(), span.created_at.actor_id());
                    spans += 1;
                }
            }
            WireOperation::TreeEdit {
                split_tickets: tickets,
                ..
            } if !tickets.is_empty() => {
                for ticket in tickets {
                    assert_eq!(&actor_a(), ticket.actor_id());
                    split_tickets += 1;
                }
            }
            _ => {}
        }
    }
    assert!(spans >= 2, "an identity edit with both span lists");
    assert!(split_tickets > 0, "a tree edit with split tickets");

    let restored = reissued.last().and_then(text_value).expect("the text");
    let WireJsonElement::Text { nodes, .. } = WireJsonElement::try_from(restored)? else {
        panic!("expected a text");
    };
    let events = nodes
        .iter()
        .flat_map(|node| &node.removal_events)
        .collect::<Vec<_>>();
    assert!(!events.is_empty(), "a node with a removal ledger");
    assert!(events
        .iter()
        .all(|ticket| ticket.lamport() == 0 || ticket.actor_id() == &actor_a()));
    assert!(nodes
        .iter()
        .flat_map(|node| node.attributes.values())
        .all(|attr| attr.updated_at.actor_id() == &actor_a()));
    assert!(nodes.iter().any(|node| !node.attributes.is_empty()));
    let CrdtElement::Text(text) = restored else {
        panic!("expected a text");
    };
    assert_eq!("abc", text.to_string());
    Ok(())
}

/// No JS case: a Tree node's merge tickets come only from a concurrent merge
/// applied remotely, which a never-synced document cannot hold, so the walk
/// is checked on a node built by hand.
#[test]
fn reissues_the_merge_tickets_of_a_tree_node() {
    let ticket = |lamport| TimeTicket::new(lamport, 0, INITIAL_ACTOR_ID);
    let id = |lamport| WireTreeNodeId {
        created_at: ticket(lamport),
        offset: 0,
    };
    let mut node = WireTreeNode {
        id: id(1),
        node_type: "p".to_owned(),
        value: String::new(),
        removed_at: Some(ticket(5)),
        ins_prev_id: Some(id(2)),
        ins_next_id: Some(id(3)),
        depth: 1,
        attributes: BTreeMap::from([(
            INITIAL_ACTOR_ID.to_owned(),
            WireNodeAttr {
                value: "v".to_owned(),
                updated_at: ticket(4),
                is_removed: false,
            },
        )]),
        merged_from: Some(id(0)),
        merged_at: Some(ticket(6)),
    };
    let before = tickets_in(&format!("{node:?}"));

    let (from, to) = (initial(), actor_a());
    TicketReissuer {
        from: &from,
        to: &to,
    }
    .walk_tree_node(&mut node);
    assert_reissued(&before, &tickets_in(&format!("{node:?}")));
    assert!(node.attributes.contains_key(INITIAL_ACTOR_ID));
}

fn vector(entries: &[(&str, i64)]) -> VersionVector {
    let mut vector = VersionVector::new();
    for (actor, lamport) in entries {
        vector.set(*actor, *lamport);
    }
    vector
}

const ACTOR_B: &str = "000000000000000000000002";

/// JS `reissueVersionVector` (`document.ts`): a vector without `from` comes
/// back as it was.
#[test]
fn reissue_version_vector_keeps_a_vector_without_from() {
    let given = vector(&[(ACTOR_B, 3)]);
    assert_eq!(
        given,
        reissue_version_vector(&given, &initial(), &actor_a())
    );
}

/// JS `reissueVersionVector`: `to`'s larger lamport wins.
#[test]
fn reissue_version_vector_keeps_the_larger_lamport_of_to() {
    let given = vector(&[(INITIAL_ACTOR_ID, 3), (ACTOR_A, 5), (ACTOR_B, 1)]);
    assert_eq!(
        vector(&[(ACTOR_A, 5), (ACTOR_B, 1)]),
        reissue_version_vector(&given, &initial(), &actor_a())
    );
}

/// JS `reissueVersionVector`: `from`'s lamport moves to `to`.
#[test]
fn reissue_version_vector_moves_from_to_to() {
    let given = vector(&[(INITIAL_ACTOR_ID, 7), (ACTOR_A, 5), (ACTOR_B, 1)]);
    assert_eq!(
        vector(&[(ACTOR_A, 7), (ACTOR_B, 1)]),
        reissue_version_vector(&given, &initial(), &actor_a())
    );
    assert_eq!(
        vector(&[(ACTOR_A, 7)]),
        reissue_version_vector(&vector(&[(INITIAL_ACTOR_ID, 7)]), &initial(), &actor_a())
    );
}
