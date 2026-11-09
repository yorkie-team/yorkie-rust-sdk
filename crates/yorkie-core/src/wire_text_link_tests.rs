//! Wire-projection tests of the Text insertion links (`ins_prev_id`), ported
//! from JS `test/unit/api/snapshot_converter_test.ts` (`ed25355e`).

use super::{wire_text_from_domain, WireJsonElement, WireTextNode};
use crate::crdt::element::CrdtElement;
use crate::crdt::rga_tree_split::{
    RgaTreeSplit, RgaTreeSplitNode, RgaTreeSplitNodeId, RgaTreeSplitPos,
};
use crate::crdt::text::{CrdtText, TextValue};
use crate::{Result, TimeTicket};
use std::collections::BTreeSet;

fn ticket(lamport: i64) -> TimeTicket {
    TimeTicket::new(lamport, 0, "000000000000000000000001")
}

fn node_id(lamport: i64, offset: usize) -> RgaTreeSplitNodeId {
    RgaTreeSplitNodeId::new(ticket(lamport), offset)
}

type Link = (
    RgaTreeSplitNodeId,
    Option<RgaTreeSplitNodeId>,
    Option<RgaTreeSplitNodeId>,
);

/// Every node's ID with its `ins_prev_id` and `ins_next_id` (JS `linksOf`).
fn links_of(text: &CrdtText) -> Vec<Link> {
    text.nodes()
        .map(|node| {
            (
                node.id().clone(),
                node.ins_prev_id().cloned(),
                node.ins_next_id().cloned(),
            )
        })
        .collect()
}

fn encode(text: &CrdtText) -> Vec<WireTextNode> {
    let WireJsonElement::Text { nodes, .. } = wire_text_from_domain(text) else {
        panic!("expected a text");
    };
    nodes
}

fn decode(nodes: Vec<WireTextNode>) -> Result<CrdtText> {
    let wire = WireJsonElement::Text {
        nodes,
        created_at: ticket(1),
        moved_at: None,
        removed_at: None,
    };
    let CrdtElement::Text(text) = CrdtElement::try_from(wire)? else {
        panic!("expected a text");
    };
    Ok(*text)
}

/// `abcdef` inserted at 2, whose `cd` was replaced by `x` at 3: split
/// pieces `ab`, `cd` and `ef` of one insertion, linked to each other.
fn split_text() -> Result<CrdtText> {
    let mut text = CrdtText::create(ticket(1));
    text.edit_by_index(0, 0, "abcdef", None, ticket(2), None)?;
    text.edit_by_index(2, 4, "x", None, ticket(3), None)?;
    Ok(text)
}

/// A Text built node by node, bypassing the decoder, with the given links.
fn built_text(
    nodes: &[(RgaTreeSplitNodeId, &str)],
    links: &[(RgaTreeSplitNodeId, RgaTreeSplitNodeId)],
) -> Result<CrdtText> {
    let head = RgaTreeSplitNode::new(
        RgaTreeSplitNodeId::new(TimeTicket::initial(), 0),
        TextValue::create(""),
    );
    let mut split = RgaTreeSplit::new(head);
    let mut prev = split.initial_head().id().clone();
    for (id, content) in nodes {
        prev = split.insert_after_id(
            &prev,
            RgaTreeSplitNode::new(id.clone(), TextValue::create(*content)),
        )?;
    }
    for (id, ins_prev_id) in links {
        split
            .find_node_mut_by_id(id)
            .expect("the linked node")
            .set_ins_prev_id(Some(ins_prev_id.clone()));
    }
    Ok(CrdtText::new(ticket(1), split))
}

/// JS `should round-trip the insertion links of split text nodes`: a
/// position naming a split node's original ID resolves through the links,
/// so a copy without them resolves the same position elsewhere.
#[test]
fn round_trips_the_insertion_links_of_split_text_nodes() -> Result<()> {
    let text = split_text()?;
    let before = links_of(&text);
    assert!(before
        .iter()
        .any(|(_, prev, next)| prev.is_some() || next.is_some()));

    let decoded = decode(encode(&text))?;
    assert_eq!(before, links_of(&decoded));
    Ok(())
}

/// JS `fromText` relinks through `setInsPrev`, which also sets the target's
/// `insNext`: a later split of the left piece then relinks the right one,
/// so a position at the right piece's start resolves as on the original.
#[test]
fn resolves_a_split_position_on_a_decoded_text_as_on_the_original() -> Result<()> {
    let mut original = split_text()?;
    let mut decoded = decode(encode(&original))?;

    let at_cd = RgaTreeSplitPos::new(node_id(2, 2), 0);
    for text in [&mut original, &mut decoded] {
        // Split `ab` into `a` and `b`, then insert at the start of `cd`.
        text.edit_by_index(1, 1, "y", None, ticket(4), None)?;
        text.edit_by_range((at_cd.clone(), at_cd.clone()), "z", None, ticket(5), None)?;
    }
    assert_eq!(original.to_string(), decoded.to_string());
    assert_eq!(links_of(&original), links_of(&decoded));
    Ok(())
}

/// JS `should only emit an insertion link a decoder can resolve`.
#[test]
fn only_emits_an_insertion_link_a_decoder_can_resolve() -> Result<()> {
    let mut text = split_text()?;
    text.edit_by_index(1, 2, "yz", None, ticket(4), None)?;

    let mut emitted = BTreeSet::new();
    let mut linked = 0;
    for node in encode(&text) {
        if let Some(ins_prev_id) = node.ins_prev_id {
            linked += 1;
            assert!(
                emitted.contains(&RgaTreeSplitNodeId::from(ins_prev_id.clone())),
                "forward or dangling ins_prev_id: {ins_prev_id:?}"
            );
        }
        emitted.insert(RgaTreeSplitNodeId::from(node.id));
    }
    assert!(linked > 0);
    Ok(())
}

/// JS `toTextNodes` (the `emitted` set): a link into a node that is not in
/// this split, such as a purged tombstone, is not written.
#[test]
fn drops_a_dangling_insertion_link_on_encode() -> Result<()> {
    let text = built_text(
        &[(node_id(2, 0), "ab"), (node_id(2, 4), "ef")],
        &[(node_id(2, 4), node_id(2, 2))],
    )?;
    assert!(encode(&text).iter().all(|node| node.ins_prev_id.is_none()));
    Ok(())
}

/// JS `should drop an insertion link that names no decoded node`.
#[test]
fn drops_an_insertion_link_that_names_no_decoded_node() -> Result<()> {
    let mut nodes = encode(&split_text()?);
    // Point the first node at the last one: a link no decoder can have
    // resolved by the time the first node is read.
    nodes[0].ins_prev_id = Some(nodes[nodes.len() - 1].id.clone());

    let decoded = decode(nodes)?;
    let first = decoded.nodes().next().expect("the first node");
    assert_eq!(None, first.ins_prev_id());
    Ok(())
}

/// JS `should drop an insertion link that names another insertion`.
#[test]
fn drops_an_insertion_link_that_names_another_insertion() -> Result<()> {
    let mut text = CrdtText::create(ticket(1));
    text.edit_by_index(0, 0, "abc", None, ticket(2), None)?;
    text.edit_by_index(3, 3, "def", None, ticket(3), None)?;

    let mut nodes = encode(&text);
    assert_eq!(2, nodes.len());
    // The second insertion decodes after the first, so this link resolves
    // by exact ID; only the same-insertion check rejects it.
    nodes[1].ins_prev_id = Some(nodes[0].id.clone());

    let decoded = decode(nodes)?;
    let second = decoded.nodes().nth(1).expect("the second node");
    assert_eq!(None, second.ins_prev_id());
    Ok(())
}

/// JS `fromText`: a link to an already-decoded piece of the same insertion
/// at a higher offset is one no local edit makes, and is dropped.
#[test]
fn drops_an_insertion_link_to_a_higher_offset() -> Result<()> {
    let mut nodes = encode(&built_text(
        &[(node_id(2, 4), "ef"), (node_id(2, 2), "cd")],
        &[],
    )?);
    nodes[1].ins_prev_id = Some(nodes[0].id.clone());

    let decoded = decode(nodes)?;
    assert!(decoded.nodes().all(|node| node.ins_prev_id().is_none()));
    assert!(decoded.nodes().all(|node| node.ins_next_id().is_none()));
    Ok(())
}

/// JS `splitNode` rejects a negative offset: a position resolved through a
/// forged link into a higher-offset node errors instead of underflowing.
#[test]
fn rejects_a_position_resolved_through_a_forged_higher_offset_link() -> Result<()> {
    let mut text = built_text(
        &[
            (node_id(2, 0), "ab"),
            (node_id(2, 2), "cd"),
            (node_id(2, 4), "ef"),
        ],
        &[(node_id(2, 2), node_id(2, 4))],
    )?;
    let at_cd = RgaTreeSplitPos::new(node_id(2, 2), 0);
    let range = (at_cd.clone(), at_cd);

    assert!(text.find_indexes_from_range(&range).is_err());
    assert!(text
        .edit_by_range(range, "z", None, ticket(5), None)
        .is_err());
    assert_eq!("abcdef", text.to_string());
    Ok(())
}
