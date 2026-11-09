//! CRDT-level tests of the Text removal ledger, ported from JS
//! `test/unit/document/crdt/text_removal_ledger_test.ts` (`84068f70`) and
//! cross-checked with Go `pkg/document/crdt/text_restore_test.go`.

use super::rga_tree_split::{RgaTreeSplitNode, RgaTreeSplitNodeId};
use super::text::{CrdtText, TextValue};
use crate::{TimeTicket, VersionVector};

const ACTOR_A: &str = "000000000000000000000001";
const ACTOR_B: &str = "000000000000000000000002";

fn a(lamport: i64) -> TimeTicket {
    TimeTicket::new(lamport, 0, ACTOR_A)
}

fn b(lamport: i64) -> TimeTicket {
    TimeTicket::new(lamport, 0, ACTOR_B)
}

fn vector<const N: usize>(entries: [(&str, i64); N]) -> VersionVector {
    let mut vector = VersionVector::new();
    for (actor_id, lamport) in entries {
        vector.set(actor_id, lamport);
    }
    vector
}

/// A text node created by actor A at lamport 1, removed at `removed_at`
/// (JS `node`).
fn node(removed_at: Option<TimeTicket>) -> RgaTreeSplitNode<TextValue> {
    let mut node = RgaTreeSplitNode::new(RgaTreeSplitNodeId::new(a(1), 0), TextValue::create("x"));
    node.set_removed_at(removed_at);
    node
}

/// "0123456789" as a single insertion created at `seed` (JS `seededText`).
fn seeded_text(seed: TimeTicket) -> crate::Result<CrdtText> {
    let mut text = CrdtText::create(TimeTicket::initial());
    text.edit_by_index(0, 0, "0123456789", None, seed, None)?;
    assert_eq!("0123456789", text.to_string());
    Ok(text)
}

/// JS `RGATreeSplitNode removal events` / `pairs each actor events by place`.
#[test]
fn pairs_each_actor_events_by_place() -> crate::Result<()> {
    // A's removal holds the node; B removed it and undid that.
    let mut n = node(Some(a(5)));
    n.set_removal_events(vec![b(6), b(4)])?;
    let (ledgered, dels, cancels) = n.ledger();
    assert!(ledgered);
    assert_eq!(2, dels.len());
    assert_eq!(1, cancels.len());
    assert_eq!(b(4), cancels[0].deletion);
    assert_eq!(b(6), cancels[0].cancelled_at);
    assert_eq!(Some(&a(5)), n.removed_at());
    assert_eq!(vec![b(4), b(6)], n.removal_events());
    Ok(())
}

/// JS `RGATreeSplitNode removal events` / `rejects malformed events: %s`
/// (the `it.each` table).
#[test]
fn rejects_malformed_events() {
    let cases: [(&str, Option<TimeTicket>, Vec<TimeTicket>); 4] = [
        ("removed_at repeated", Some(a(5)), vec![a(5)]),
        ("event repeated", None, vec![a(5), a(5)]),
        ("removal without removedAt", None, vec![a(5)]),
        ("removed_at cancelled", Some(a(5)), vec![a(3)]),
    ];
    for (name, removed_at, events) in cases {
        let mut n = node(removed_at.clone());
        assert!(n.set_removal_events(events).is_err(), "{name}");
        // The node is left as it was.
        assert_eq!(removed_at.as_ref(), n.removed_at(), "{name}");
        assert!(n.removal_events().is_empty(), "{name}");
        assert!(!n.ledger().0, "{name}");
    }
}

/// JS `RGATreeSplitNode removal events` / `writes nothing when removedAt says
/// it all`.
#[test]
fn writes_nothing_when_removed_at_says_it_all() {
    let mut n = node(None);
    n.record_removal(a(5));
    assert!(n.removal_events().is_empty());
    assert!(!n.ledger().0);
}

/// JS `Text removal ledger` / `returns removed spans in document order`; Go
/// `TestTextEditRemovedSpansInDocumentOrder`.
#[test]
fn returns_removed_spans_in_document_order() -> crate::Result<()> {
    // Restore anchors each recreated fragment after the one before it, so the
    // reverse of an edit has to list them left to right.
    let mut text = seeded_text(a(1))?;
    for (i, pos) in [8, 6, 4, 2].into_iter().enumerate() {
        text.edit_by_index(pos, pos, "x", None, a(2 + i as i64), None)?;
    }
    assert_eq!("01x23x45x67x89", text.to_string());

    let result = text.edit_by_index(0, 14, "", None, a(10), None)?;
    let removed = result
        .removed_spans
        .iter()
        .map(|span| span.value.content())
        .collect::<String>();
    assert_eq!("01x23x45x67x89", removed);

    // Each span names the piece by its insertion and absolute offsets.
    let seeded = result
        .removed_spans
        .iter()
        .filter(|span| span.created_at == a(1))
        .map(|span| (span.start, span.end))
        .collect::<Vec<_>>();
    assert_eq!(vec![(0, 2), (2, 4), (4, 6), (6, 8), (8, 10)], seeded);
    Ok(())
}

/// JS `Text removal ledger` / `keeps one uncancelled removal per actor`; Go
/// `TestTextRemovalKeepsOneUncancelledRemovalPerActor`.
#[test]
fn keeps_one_uncancelled_removal_per_actor() -> crate::Result<()> {
    // A removal by an actor that already holds an uncancelled one is not
    // recorded: only a change that leaves its own earlier removal out of its
    // vector can send it.
    let mut text = seeded_text(a(1))?;
    text.edit_by_index(2, 4, "", None, a(2), None)?;

    // The same actor removes [1,3) of "01456789" again, with a vector that
    // knows the insertion but not its own removal of "23".
    let seen_seed = vector([(ACTOR_A, 1)]);
    text.edit_by_index(1, 3, "", None, a(5), Some(&seen_seed))?;

    let mut found = false;
    for n in text.nodes() {
        if n.id().offset() == 2 {
            found = true;
            assert_eq!(Some(&a(2)), n.removed_at());
            assert!(n.removal_events().is_empty());
        }
    }
    assert!(found);
    Ok(())
}

/// JS `ticketKnown` (`src/document/time/version_vector.ts` 173-186): an
/// absent or empty vector is local and knows every ticket; an actor missing
/// from a non-empty vector is unknown even at lamport 0.
#[test]
fn ticket_known_reads_an_empty_vector_as_local() {
    use crate::time::ticket_known;

    assert!(ticket_known(None, &a(7)));
    assert!(ticket_known(Some(&VersionVector::new()), &a(7)));
    assert!(ticket_known(Some(&vector([(ACTOR_A, 7)])), &a(7)));
    assert!(!ticket_known(Some(&vector([(ACTOR_A, 6)])), &a(7)));
    assert!(!ticket_known(Some(&vector([(ACTOR_B, 9)])), &a(0)));
}

/// JS `RGATreeSplit.deleteNodes` (2145-2237): a node a remote deletion saw
/// live but another actor removed records the deletion, whether it is kept
/// (an older deletion) or overwritten by LWW (a newer one), so replicas that
/// applied the two deletions in either order hold the same ledger. Only the
/// deletion that took the node from live to removed registers a GC pair.
#[test]
fn records_a_concurrent_removal_in_either_order() -> crate::Result<()> {
    let seen_seed = vector([(ACTOR_A, 1)]);
    let mut on_a = seeded_text(a(1))?;
    let mut on_b = on_a.deepcopy();
    let range = on_a.index_range_to_pos_range(2, 3)?;

    // A removes "2" at a(3) after B removed it at b(2), concurrently.
    let local = on_a.edit_by_range(range.clone(), "", None, a(3), None)?;
    assert_eq!(1, local.removed_nodes.len());
    let kept = on_a.edit_by_range(range.clone(), "", None, b(2), Some(&seen_seed))?;
    assert!(kept.removed_nodes.is_empty());
    assert!(kept.removed_spans.is_empty());

    // B applies them the other way round: A's later deletion overwrites B's.
    on_b.edit_by_range(range.clone(), "", None, b(2), None)?;
    let overwrite = on_b.edit_by_range(range, "", None, a(3), Some(&vector([(ACTOR_A, 2)])))?;
    assert!(overwrite.removed_nodes.is_empty());
    assert_eq!(1, overwrite.removed_spans.len());
    assert_eq!("2", overwrite.removed_spans[0].value.content());

    for text in [&on_a, &on_b] {
        assert_eq!("013456789", text.to_string());
        let node = text
            .nodes()
            .find(|n| n.id().offset() == 2)
            .expect("the removed piece is kept as a tombstone");
        assert_eq!(Some(&a(3)), node.removed_at());
        assert_eq!(vec![b(2)], node.removal_events());
    }
    Ok(())
}
