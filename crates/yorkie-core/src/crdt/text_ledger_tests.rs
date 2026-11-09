//! CRDT-level tests of the Text removal ledger, ported from JS
//! `test/unit/document/crdt/text_removal_ledger_test.ts` (`84068f70`) and
//! cross-checked with Go `pkg/document/crdt/text_restore_test.go`.

use super::rga_tree_split::{RestoreSpan, RgaTreeSplitNode, RgaTreeSplitNodeId};
use super::text::{CrdtText, TextValue};
use crate::{TimeTicket, VersionVector};
use std::collections::BTreeMap;

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

/// A restore span of the insertion `created_at` over `[start, end)`.
fn span(created_at: TimeTicket, start: usize, end: usize, content: &str) -> RestoreSpan<TextValue> {
    RestoreSpan {
        created_at,
        start,
        end,
        value: TextValue::create(content),
    }
}

/// Checks that the index tree agrees with the nodes: its length is the
/// visible length, and every index resolves (Go `CheckWeight`).
fn assert_weights(text: &CrdtText) -> crate::Result<()> {
    let visible = text.to_string().encode_utf16().count();
    assert_eq!(visible, text.len(), "index length vs visible length");
    for index in 0..=visible {
        text.index_to_pos(index)?;
    }
    Ok(())
}

/// The piece of `text` at absolute offset `offset` of insertion `created_at`.
fn piece_at<'a>(
    text: &'a CrdtText,
    created_at: &TimeTicket,
    offset: usize,
) -> &'a RgaTreeSplitNode<TextValue> {
    text.nodes()
        .find(|n| n.created_at() == created_at && n.id().offset() == offset)
        .expect("a piece at that offset")
}

/// A node removed by `removal` and cancelled at `cancel`: removed in the
/// view of a vector that knew the removal and not the cancel, live in the
/// view of one that knew both (JS `isRemovedInView`).
#[test]
fn reads_a_ledgered_node_in_a_view() {
    let mut n = node(None);
    n.record_removal(a(2));
    assert!(n.cancel_removal(&a(2), a(3)));
    assert!(n.ledger().0);
    assert!(!n.is_removed());

    assert!(n.is_removed_in_view(&vector([(ACTOR_A, 2)])));
    assert!(!n.is_removed_in_view(&vector([(ACTOR_A, 3)])));
    assert!(!n.is_removed_in_view(&vector([(ACTOR_A, 1)])));
}

/// JS `cancelRemoval`: a cancel takes out one deletion and the node is live
/// only once no uncancelled deletion is left; cancelling the same deletion
/// again changes nothing.
#[test]
fn cancels_one_removal_at_a_time() {
    let mut n = node(None);
    n.record_removal(a(2));
    n.record_removal(b(3));
    assert_eq!(Some(&b(3)), n.removed_at());

    assert!(!n.cancel_removal(&b(3), b(4)));
    assert_eq!(Some(&a(2)), n.removed_at());
    assert_eq!(None, n.uncancelled_removal_by(b(0).actor_id()));
    assert_eq!(Some(&a(2)), n.uncancelled_removal_by(a(0).actor_id()));

    assert!(n.cancel_removal(&a(2), a(5)));
    assert!(!n.is_removed());
    assert!(n.cancel_removal(&a(2), a(6)));
    assert_eq!(2, n.ledger().2.len());
    assert_eq!(vec![a(2), b(3), b(4), a(5)], n.removal_events());
}

/// JS `deleteNodes` live-in-view rule (2145-2232), on the node a peer saw
/// removed and an undo revived here: a deletion whose vector knew the
/// removal but not its undo leaves the node alone, and one that knew both
/// removes it.
#[test]
fn keeps_a_node_the_deleter_saw_removed_and_an_undo_revived() -> crate::Result<()> {
    let mut saw_removed = seeded_text(a(1))?;
    saw_removed.edit_by_index(2, 3, "", None, a(2), None)?;
    saw_removed.restore(&[span(a(1), 2, 3, "2")], &a(3), None)?;
    assert_eq!("0123456789", saw_removed.to_string());
    let mut saw_live = saw_removed.deepcopy();
    let range = saw_removed.index_range_to_pos_range(2, 3)?;

    let kept = saw_removed.edit_by_range(
        range.clone(),
        "",
        None,
        b(4),
        Some(&vector([(ACTOR_A, 2), (ACTOR_B, 4)])),
    )?;
    assert!(kept.removed_nodes.is_empty());
    assert_eq!("0123456789", saw_removed.to_string());

    let removed = saw_live.edit_by_range(
        range,
        "",
        None,
        b(4),
        Some(&vector([(ACTOR_A, 3), (ACTOR_B, 4)])),
    )?;
    assert_eq!(1, removed.removed_nodes.len());
    assert_eq!("013456789", saw_live.to_string());
    Ok(())
}

/// JS `Text removal ledger` / `cancels only the undoing actor removal`.
#[test]
fn cancels_only_the_undoing_actors_removal() -> crate::Result<()> {
    // A and B remove "2" concurrently, B with the later ticket. A's undo has
    // to cancel A's removal, not the latest one, and leave B's holding it.
    let mut text = seeded_text(a(1))?;
    let range = text.index_range_to_pos_range(2, 3)?;
    text.edit_by_index(2, 3, "", None, a(2), None)?;
    text.edit_by_range(range, "", None, b(3), Some(&vector([(ACTOR_A, 1)])))?;
    assert_eq!("013456789", text.to_string());

    text.restore(&[span(a(1), 2, 3, "2")], &a(4), None)?;
    assert_eq!("013456789", text.to_string());
    let n = piece_at(&text, &a(1), 2);
    assert_eq!(None, n.uncancelled_removal_by(a(0).actor_id()));
    assert_eq!(Some(&b(3)), n.uncancelled_removal_by(b(0).actor_id()));
    Ok(())
}

/// JS `Text removal ledger` / `reports nothing tombstoned by a redo of a
/// removed piece`; Go `TestRetombstoneReportsNothingTombstoned`.
#[test]
fn reports_nothing_tombstoned_by_a_redo_of_a_removed_piece() -> crate::Result<()> {
    // A redo that re-removes a span another actor already tombstoned -- by a
    // deletion the redo never saw -- splits that tombstone and hands back the
    // born-dead remainders as GC pairs, but nothing went live to dead. It
    // still records the redo on the piece, which the reverse names.
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    text.edit_by_index(2, 8, "", None, b(2), None)?;
    assert_eq!("0189", text.to_string());

    let result = text.retombstone(
        &[span(seed.clone(), 4, 6, "45")],
        &a(3),
        Some(&vector([(ACTOR_A, 3)])),
    )?;
    assert!(result.changes.is_empty());
    assert!(!result.gc_pairs.is_empty());
    assert_eq!(1, result.recorded.len());
    assert_eq!((4, 6), (result.recorded[0].start, result.recorded[0].end));

    let n = piece_at(&text, &seed, 4);
    assert_eq!(Some(&b(2)), n.uncancelled_removal_by(b(0).actor_id()));
    assert_eq!(Some(&a(3)), n.uncancelled_removal_by(a(0).actor_id()));
    Ok(())
}

/// Go `TestTextRestore` / `overlapping undo converges in both restore
/// orders`.
#[test]
fn converges_overlapping_undos_in_both_orders() -> crate::Result<()> {
    let seed = a(1);
    let d1 = || span(seed.clone(), 4, 6, "45");
    let d2 = || span(seed.clone(), 2, 8, "234567");
    let deleted = || -> crate::Result<CrdtText> {
        // Compute both ranges on the original, then apply both deletes.
        let mut text = seeded_text(seed.clone())?;
        let r1 = text.index_range_to_pos_range(4, 6)?;
        let r2 = text.index_range_to_pos_range(2, 8)?;
        text.edit_by_range(r1, "", None, a(2), None)?;
        text.edit_by_range(r2, "", None, a(3), None)?;
        assert_eq!("0189", text.to_string());
        Ok(text)
    };

    let mut first = deleted()?;
    first.restore(&[d1()], &a(4), None)?;
    first.restore(&[d2()], &a(5), None)?;

    let mut second = deleted()?;
    second.restore(&[d2()], &a(5), None)?;
    second.restore(&[d1()], &a(4), None)?;

    assert_eq!("0123456789", first.to_string());
    assert_eq!("0123456789", second.to_string());
    assert_weights(&first)?;
    assert_weights(&second)
}

/// Go `TestTextRestore` / `only one side undone restores just its content`.
#[test]
fn restores_just_the_side_undone() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    let r1 = text.index_range_to_pos_range(4, 6)?;
    let r2 = text.index_range_to_pos_range(2, 8)?;
    text.edit_by_range(r1, "", None, a(2), None)?;
    text.edit_by_range(r2, "", None, a(3), None)?;

    // Undo only d1: "45" reappears while "23" and "67" stay removed by d2.
    text.restore(&[span(seed.clone(), 4, 6, "45")], &a(4), None)?;
    assert_eq!("014589", text.to_string());
    // Undo d2 as well: the full range comes back exactly once.
    text.restore(&[span(seed, 2, 8, "234567")], &a(5), None)?;
    assert_eq!("0123456789", text.to_string());
    assert_weights(&text)
}

/// Go `TestTextRestore` / `partial overlap [4,6) x [2,5)`.
#[test]
fn restores_a_partial_overlap() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    let r1 = text.index_range_to_pos_range(4, 6)?;
    let r2 = text.index_range_to_pos_range(2, 5)?;
    text.edit_by_range(r1, "", None, a(2), None)?;
    text.edit_by_range(r2, "", None, a(3), None)?;
    assert_eq!("016789", text.to_string());

    text.restore(&[span(seed.clone(), 4, 6, "45")], &a(4), None)?;
    text.restore(&[span(seed, 2, 5, "234")], &a(5), None)?;
    assert_eq!("0123456789", text.to_string());
    assert_weights(&text)
}

/// Go `TestTextRestore` / `redo (retombstone) then undo again`.
#[test]
fn redoes_and_undoes_again() -> crate::Result<()> {
    let seed = a(1);
    let d1 = || span(seed.clone(), 4, 6, "45");
    let mut text = seeded_text(seed.clone())?;
    text.edit_by_index(4, 6, "", None, a(2), None)?;
    assert_eq!("01236789", text.to_string());

    text.restore(&[d1()], &a(3), None)?;
    assert_eq!("0123456789", text.to_string());

    let redo = text.retombstone(&[d1()], &a(4), None)?;
    assert_eq!("01236789", text.to_string());
    assert_eq!(1, redo.gc_pairs.len());
    assert_eq!(1, redo.changes.len());
    assert_eq!((4, 6), (redo.changes[0].from, redo.changes[0].to));

    text.restore(&[d1()], &a(5), None)?;
    assert_eq!("0123456789", text.to_string());
    assert_weights(&text)
}

/// Go `TestTextRestore` / `duplicate restore is a no-op (idempotent)`.
#[test]
fn ignores_a_duplicate_restore() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    text.edit_by_index(4, 6, "", None, a(2), None)?;
    let first = text.restore(&[span(seed.clone(), 4, 6, "45")], &a(3), None)?;
    assert_eq!(1, first.revived.len());
    assert_eq!(1, first.changes.len());
    assert_eq!(4, first.changes[0].from);

    let duplicate = text.restore(&[span(seed, 4, 6, "45")], &a(3), None)?;
    assert!(duplicate.revived.is_empty());
    assert!(duplicate.changes.is_empty());
    assert_eq!("0123456789", text.to_string());
    assert_weights(&text)
}

/// JS `test/unit/document/text_restore_relink_test.ts` / `keeps a later
/// boundary edit correct after a purged interior fragment is recreated`
/// (yorkie-js-sdk#1327), at the CRDT level: the purge and the undo are the
/// text's own.
#[test]
fn keeps_a_later_boundary_edit_correct_after_a_purged_fragment_is_recreated() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    text.edit_by_index(4, 6, "", None, a(2), None)?;
    assert_eq!("01236789", text.to_string());

    // Purge the tombstone so the undo has to recreate "45".
    let purged = RgaTreeSplitNodeId::new(seed.clone(), 4).to_id_string();
    assert!(text.purge_gc_pair_by_id(&purged));

    let restored = text.restore(&[span(seed.clone(), 4, 6, "45")], &a(3), None)?;
    assert_eq!(1, restored.recreated.len());
    assert_eq!("0123456789", text.to_string());

    // An edit whose boundary sits at the recreated node's right edge.
    text.edit_by_index(6, 6, "X", None, a(4), None)?;
    assert_eq!("012345X6789", text.to_string());
    assert_weights(&text)
}

/// A recreated piece relinks the insertion chain to the nearest surviving
/// pieces on both sides, so purging the piece before it later does not hand
/// the piece after it an `ins_prev` that skips it (JS `restore`).
#[test]
fn relinks_a_recreated_piece_to_the_nearest_surviving_pieces() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    text.edit_by_index(4, 6, "", None, a(2), None)?;
    text.edit_by_index(2, 4, "", None, a(3), None)?;
    assert_eq!("016789", text.to_string());
    for offset in [2, 4] {
        let id = RgaTreeSplitNodeId::new(seed.clone(), offset).to_id_string();
        assert!(text.purge_gc_pair_by_id(&id));
    }

    text.restore(&[span(seed.clone(), 4, 6, "45")], &a(4), None)?;
    assert_eq!("01456789", text.to_string());
    let recreated = piece_at(&text, &seed, 4);
    assert_eq!(Some(0), recreated.ins_prev_id().map(|id| id.offset()));
    assert_eq!(Some(6), recreated.ins_next_id().map(|id| id.offset()));
    assert_eq!(
        Some(4),
        piece_at(&text, &seed, 0)
            .ins_next_id()
            .map(|id| id.offset())
    );
    assert_eq!(
        Some(4),
        piece_at(&text, &seed, 6)
            .ins_prev_id()
            .map(|id| id.offset())
    );
    Ok(())
}

/// Go `TestTextRestoreAfterGCKeepsOrderAcrossInsertions` at the CRDT level: a
/// purged run of single-character insertions comes back in order, each
/// fragment chained after the one placed before it.
#[test]
fn recreates_a_purged_run_in_document_order() -> crate::Result<()> {
    let typed = "hello my name is";
    let mut text = CrdtText::create(TimeTicket::initial());
    for (i, ch) in typed.chars().enumerate() {
        text.edit_by_index(i, i, ch.to_string(), None, a(100 + i as i64), None)?;
    }
    let removed = text.edit_by_index(6, 13, "", None, a(200), None)?;
    for node in &removed.removed_nodes {
        assert!(text.purge_gc_pair_by_id(&node.id_string()));
    }
    assert_eq!("hello  is", text.to_string());

    let fallback = text.index_to_pos(6)?;
    let restored = text.restore(&removed.removed_spans, &a(201), Some(&fallback))?;
    assert_eq!(typed, text.to_string());
    assert_eq!(7, restored.recreated.len());
    let froms = restored.changes.iter().map(|c| c.from).collect::<Vec<_>>();
    assert_eq!((6..13).collect::<Vec<_>>(), froms);
    assert_weights(&text)
}

/// The stand-in removal a recreated piece records is at lamport 0 and
/// always cancelled, so it never becomes `removed_at`: JS `deleteNodes`
/// reads a lamport-0 vector entry as unknown where Rust reads it as known,
/// which is safe only while no `removed_at` is at lamport 0.
#[test]
fn never_makes_the_stand_in_removal_removed_at() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    text.edit_by_index(4, 6, "", None, a(2), None)?;
    let purged = RgaTreeSplitNodeId::new(seed.clone(), 4).to_id_string();
    assert!(text.purge_gc_pair_by_id(&purged));

    text.restore(&[span(seed.clone(), 4, 6, "45")], &a(3), None)?;
    let stand_in = TimeTicket::new(0, 0, a(0).actor_id().clone());
    let recreated = piece_at(&text, &seed, 4);
    assert_eq!(None, recreated.removed_at());
    assert_eq!(&[stand_in.clone()][..], recreated.ledger().1);
    assert_eq!(vec![stand_in.clone(), a(3)], recreated.removal_events());
    // A later deletion knows the purged removal, so the piece is live in its
    // view, as it was before the purge.
    assert!(!recreated.is_removed_in_view(&vector([(ACTOR_A, 3), (ACTOR_B, 1)])));

    // The ledger survives the snapshot round trip with `removed_at` unset.
    let mut copy = recreated.clone();
    copy.set_removed_at(None);
    copy.set_removal_events(recreated.removal_events())?;
    assert_eq!(None, copy.removed_at());

    // Removed again by the same actor and by another, it reports one of
    // those, never the stand-in, and undoing both leaves it live.
    let range = text.index_range_to_pos_range(4, 6)?;
    text.retombstone(&[span(seed.clone(), 4, 6, "45")], &a(4), None)?;
    text.edit_by_range(
        range,
        "",
        None,
        b(5),
        Some(&vector([(ACTOR_A, 3), (ACTOR_B, 1)])),
    )?;
    let n = piece_at(&text, &seed, 4);
    assert_eq!(Some(&b(5)), n.removed_at());
    assert_eq!(Some(&a(4)), n.uncancelled_removal_by(a(0).actor_id()));
    text.restore(&[span(seed.clone(), 4, 6, "45")], &a(6), None)?;
    assert_eq!(Some(&b(5)), piece_at(&text, &seed, 4).removed_at());
    text.restore(&[span(seed.clone(), 4, 6, "45")], &b(7), None)?;
    let n = piece_at(&text, &seed, 4);
    assert_eq!(None, n.removed_at());

    // Pruning drops the stand-in with its cancel, or keeps both.
    let mut unpruned = n.clone();
    unpruned.prune_stable(&vector([(ACTOR_A, 2), (ACTOR_B, 9)]));
    assert_eq!(None, unpruned.removed_at());
    assert!(unpruned.ledger().1.contains(&stand_in));
    let mut pruned = n.clone();
    pruned.prune_stable(&vector([(ACTOR_A, 9), (ACTOR_B, 9)]));
    assert_eq!(None, pruned.removed_at());
    assert!(!pruned.ledger().0);
    Ok(())
}

/// JS `cancelled` and `trackCancelled`: a restore that leaves a cancel
/// tracks the piece, a split of it tracks both parts, and `prune_ledgers`
/// drops what the vector makes stable.
#[test]
fn tracks_the_pieces_that_hold_a_cancel() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    assert!(!text.has_cancelled_removals());
    text.edit_by_index(2, 6, "", None, a(2), None)?;
    text.restore(&[span(seed.clone(), 2, 6, "2345")], &a(3), None)?;
    assert!(text.has_cancelled_removals());

    // Splitting the revived piece copies its ledger to both parts.
    text.edit_by_index(4, 4, "x", None, a(4), None)?;
    assert_eq!("0123x456789", text.to_string());
    assert_eq!(2, text.prune_ledgers(&vector([(ACTOR_A, 2)])));
    assert!(text.has_cancelled_removals());

    // A copy finds the same pieces by scanning.
    let copy = text.deepcopy();
    assert!(copy.has_cancelled_removals());

    assert_eq!(0, text.prune_ledgers(&vector([(ACTOR_A, 3)])));
    assert!(!text.has_cancelled_removals());
    assert!(text.nodes().all(|n| !n.ledger().0));
    Ok(())
}

/// JS `refreshSpans`: a region a piece still covers takes that piece's
/// current value, one span per piece, and a purged region keeps the
/// snapshot.
#[test]
fn refreshes_spans_against_current_pieces() -> crate::Result<()> {
    let seed = a(1);
    let mut text = seeded_text(seed.clone())?;
    let removed = text.edit_by_index(2, 8, "", None, a(2), None)?;
    assert_eq!(1, removed.removed_spans.len());

    // "45" comes back and is styled; "67" is purged.
    text.restore(&[span(seed.clone(), 4, 6, "45")], &a(3), None)?;
    assert_eq!("014589", text.to_string());
    let mut bold = BTreeMap::new();
    bold.insert("b".to_owned(), "true".to_owned());
    text.set_style_by_index(2, 4, bold.clone(), a(4), None)?;
    let purged = RgaTreeSplitNodeId::new(seed.clone(), 6).to_id_string();
    assert!(text.purge_gc_pair_by_id(&purged));

    let refreshed = text.refresh_spans(&removed.removed_spans);
    let shape = refreshed
        .iter()
        .map(|s| {
            (
                s.created_at.clone(),
                s.start,
                s.end,
                s.value.content().to_owned(),
                s.value.get_attributes(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vec![
            (seed.clone(), 2, 4, "23".to_owned(), BTreeMap::new()),
            (seed.clone(), 4, 6, "45".to_owned(), bold),
            (seed, 6, 8, "67".to_owned(), BTreeMap::new()),
        ],
        shape
    );
    // Read-only: the text is as it was.
    assert_eq!("014589", text.to_string());
    Ok(())
}
