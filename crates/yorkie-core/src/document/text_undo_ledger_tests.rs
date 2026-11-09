//! Document-level tests of Text undo with a removal ledger.
//!
//! The first group ports JS `test/unit/document/text_undo_removal_ledger_test.ts`
//! (`84068f70`), itself a port of Go's
//! `pkg/document/text_undo_removal_ledger_test.go` (yorkie#2310); the test
//! names follow the JS ones. The second ports JS
//! `test/unit/document/text_restore_after_gc_test.ts`.
//!
//! Where JS passes a change pack or a snapshot through protobuf, these pass
//! it through the `wire` projection `yorkie-protocol` encodes from; the
//! converter tests replay the key cases over protobuf bytes.

use super::adv_sync::{min_version_vector, snapshot_pack, through_wire};
use super::Document;
use crate::change::{ChangePack, Checkpoint};
use crate::crdt::element::CrdtElement;
use crate::crdt::rga_tree_split::RgaTreeSplitNode;
use crate::crdt::text::{CrdtText, TextValue};
use crate::wire::{WireChangePack, WireJsonElement};
use crate::{Result, VersionVector};
use serde_json::Value;
use std::collections::BTreeMap;

const A1: &str = "000000000000000000000001";
const A2: &str = "000000000000000000000002";
const A3: &str = "000000000000000000000003";

/// JS `send`: delivers the pending changes of `from` to each of `to`
/// through the wire, without acking them at `from`.
fn send(from: &Document, to: &mut [&mut Document]) -> Result<()> {
    let pack = through_wire(&from.create_change_pack())?;
    for doc in to {
        doc.apply_change_pack(&ChangePack::create(
            pack.document_key(),
            Checkpoint::new(0, 0),
            false,
            pack.changes().to_vec(),
            Some(VersionVector::new()),
            None,
        ))?;
    }
    Ok(())
}

/// JS `ack`: drops the changes `doc` has already sent from its local queue.
fn ack(doc: &mut Document) -> Result<()> {
    let pack = doc.create_change_pack();
    let last_seq = pack
        .changes()
        .last()
        .map_or(0, |change| change.id().client_seq());
    doc.apply_change_pack(&ChangePack::create(
        doc.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))
}

/// JS `exchange`.
fn exchange(a: &mut Document, b: &mut Document) -> Result<()> {
    send(a, &mut [b])?;
    send(b, &mut [a])?;
    ack(a)?;
    ack(b)
}

/// JS `replicas`: two replicas holding `content` in the Text under `t`.
fn replicas(content: &str) -> Result<(Document, Document)> {
    let mut a = Document::new("ledger");
    let mut b = Document::new("ledger");
    a.set_actor(A1);
    b.set_actor(A2);
    a.update(|root| root.set_text("t")?.edit(0, 0, content, None).map(|_| ()))?;
    exchange(&mut a, &mut b)?;
    Ok((a, b))
}

/// JS `fromSnapshot`: a third replica loaded from the snapshot of `src`, as
/// a client that attached late would be.
fn from_snapshot(src: &Document) -> Result<Document> {
    let mut doc = Document::new("ledger");
    doc.set_actor(A3);
    doc.apply_change_pack(&snapshot_pack("ledger", src)?)?;
    Ok(doc)
}

fn edit(doc: &mut Document, from: usize, to: usize, content: &str) -> Result<()> {
    doc.update(|root| {
        root.get_text_mut("t")?
            .edit(from, to, content, None)
            .map(|_| ())
    })
}

fn delete(doc: &mut Document, from: usize, to: usize) -> Result<()> {
    edit(doc, from, to, "")
}

fn undo(doc: &mut Document) -> Result<()> {
    doc.history_mut().undo()
}

fn redo(doc: &mut Document) -> Result<()> {
    doc.history_mut().redo()
}

/// JS `clearHistory`, which the Rust SDK does not expose.
fn clear_history(doc: &mut Document) {
    doc.history.clear_undo();
    doc.history.clear_redo();
}

fn text(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(crate::JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

fn text_of(doc: &Document) -> &CrdtText {
    match doc.crdt_root.object().get("t") {
        Some(CrdtElement::Text(text)) => text,
        other => panic!("expected a text at t, got {other:?}"),
    }
}

fn nodes(doc: &Document) -> impl Iterator<Item = &RgaTreeSplitNode<TextValue>> {
    text_of(doc).nodes()
}

/// JS `churn`: deletes the character at 1 and undoes it `n` times on `doc`
/// alone, so no peer sees the pairs and GC can prune none of them. It
/// returns the most removal events any node of the Text then carries.
fn churn(doc: &mut Document, n: usize) -> Result<usize> {
    for _ in 0..n {
        delete(doc, 1, 2)?;
        undo(doc)?;
    }
    Ok(nodes(doc)
        .map(|node| node.removal_events().len())
        .max()
        .unwrap_or(0))
}

/// JS `tombstones`: the removed nodes still in the Text.
fn tombstones(doc: &Document) -> usize {
    nodes(doc).filter(|node| node.is_removed()).count()
}

/// JS `events`.
fn events(doc: &Document) -> usize {
    nodes(doc).map(|node| node.removal_events().len()).sum()
}

/// JS `minVV`.
fn min_vv(docs: &[&Document]) -> VersionVector {
    min_version_vector(docs.iter().map(|doc| doc.version_vector()))
}

fn own_vv(doc: &Document) -> VersionVector {
    doc.version_vector().clone()
}

/// JS `keeps a concurrent deletion the undo never saw (%s)`, both orders.
#[test]
fn keeps_a_concurrent_deletion_the_undo_never_saw() -> Result<()> {
    for undoer_first in [true, false] {
        let (mut a, mut b) = replicas("abcdefgh")?;
        // b deletes `d` and undoes it; a concurrently deletes `defgh`.
        delete(&mut b, 3, 4)?;
        undo(&mut b)?;
        delete(&mut a, 3, 8)?;

        if undoer_first {
            send(&b, &mut [&mut a])?;
            send(&a, &mut [&mut b])?;
        } else {
            send(&a, &mut [&mut b])?;
            send(&b, &mut [&mut a])?;
        }

        // `d` was removed by a's deletion too: b's undo cancels only its own.
        assert_eq!("abc", text(&a), "undoer first: {undoer_first}");
        assert_eq!("abc", text(&b), "undoer first: {undoer_first}");
    }
    Ok(())
}

/// JS `does not remove text a concurrent undo revives (yorkie#2237)`.
#[test]
fn does_not_remove_text_a_concurrent_undo_revives() -> Result<()> {
    let (mut d1, mut d2) = replicas("efgh")?;
    delete(&mut d2, 1, 2)?;
    exchange(&mut d1, &mut d2)?;
    assert_eq!("egh", text(&d1));

    // d1 deletes `g` while d2 undoes its deletion of `f`.
    delete(&mut d1, 1, 2)?;
    undo(&mut d2)?;
    exchange(&mut d1, &mut d2)?;

    // d1 saw `f` removed, so its deletion leaves the revived `f` alone.
    assert_eq!("efh", text(&d1));
    assert_eq!("efh", text(&d2));
    Ok(())
}

/// JS `cancels the redo, not the original deletion, on the next undo`.
#[test]
fn cancels_the_redo_not_the_original_deletion_on_the_next_undo() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    // b: delete `d`, undo, redo, undo. Each undo has to cancel the removal
    // just before it: the second one cancels the redo.
    delete(&mut b, 3, 4)?;
    undo(&mut b)?;
    redo(&mut b)?;
    undo(&mut b)?;
    assert_eq!("abcdefgh", text(&b));
    // a concurrently removes `defgh`, which keeps `d` removed everywhere.
    delete(&mut a, 3, 8)?;
    exchange(&mut a, &mut b)?;

    assert_eq!("abc", text(&a));
    assert_eq!("abc", text(&b));
    Ok(())
}

/// JS `leaves alone a removed piece the deleter saw removed`.
#[test]
fn leaves_alone_a_removed_piece_the_deleter_saw_removed() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    delete(&mut b, 3, 4)?;
    exchange(&mut a, &mut b)?;
    // a removes `cef` around the removed `d`; b concurrently undoes.
    delete(&mut a, 2, 5)?;
    undo(&mut b)?;
    exchange(&mut a, &mut b)?;

    assert_eq!("abdgh", text(&a));
    assert_eq!("abdgh", text(&b));
    Ok(())
}

/// JS `keeps the ledger through a snapshot`.
#[test]
fn keeps_the_ledger_through_a_snapshot() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    delete(&mut b, 3, 4)?;
    delete(&mut a, 3, 8)?;
    send(&a, &mut [&mut b])?;

    // c loads b's snapshot, where `d` is removed by both deletions; it must
    // not forget a's part when b's undo arrives.
    let mut c = from_snapshot(&b)?;
    undo(&mut b)?;
    send(&b, &mut [&mut a, &mut c])?;

    assert_eq!("abc", text(&a));
    assert_eq!("abc", text(&b));
    assert_eq!("abc", text(&c));
    Ok(())
}

/// JS `keeps a piece another deletion still holds out of GC`.
#[test]
fn keeps_a_piece_another_deletion_still_holds_out_of_gc() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    delete(&mut b, 3, 4)?;
    delete(&mut a, 3, 8)?;
    exchange(&mut a, &mut b)?;

    // Both deletions are known everywhere. b collects `efgh` but keeps `d`:
    // an undo of either deletion must find it held by the other.
    assert!(b.garbage_collect(&own_vv(&b)) > 0);
    assert_eq!(1, tombstones(&b));

    undo(&mut b)?;
    exchange(&mut a, &mut b)?;
    assert_eq!("abc", text(&a));
    assert_eq!("abc", text(&b));

    // Only a's deletion is left on `d`, so it can be collected now.
    assert!(b.garbage_collect(&own_vv(&b)) > 0);
    assert_eq!(0, tombstones(&b));
    Ok(())
}

/// JS `does not recreate a purged piece a redo never removed`.
#[test]
fn does_not_recreate_a_purged_piece_a_redo_never_removed() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    // b deletes `d` and undoes it while a deletes `d` too.
    delete(&mut b, 3, 4)?;
    undo(&mut b)?;
    delete(&mut a, 3, 4)?;
    exchange(&mut a, &mut b)?;
    assert_eq!("abcefgh", text(&b));

    // b's redo finds `d` already removed by a, so it records nothing there,
    // and the undo of the redo must not name `d` either: a has collected `d`
    // and would recreate it.
    redo(&mut b)?;
    exchange(&mut a, &mut b)?;
    assert!(a.garbage_collect(&own_vv(&a)) > 0);
    undo(&mut b)?;
    exchange(&mut a, &mut b)?;

    assert_eq!("abcefgh", text(&a));
    assert_eq!("abcefgh", text(&b));
    Ok(())
}

/// JS `revives a deletion nobody else made, also after a snapshot`.
#[test]
fn revives_a_deletion_nobody_else_made_also_after_a_snapshot() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    delete(&mut b, 3, 4)?;
    exchange(&mut a, &mut b)?;
    let mut c = from_snapshot(&a)?;
    undo(&mut b)?;
    send(&b, &mut [&mut a, &mut c])?;

    assert_eq!("abcdefgh", text(&a));
    assert_eq!("abcdefgh", text(&b));
    assert_eq!("abcdefgh", text(&c));
    Ok(())
}

/// JS `redoes only what the redoer saw live`.
#[test]
fn redoes_only_what_the_redoer_saw_live() -> Result<()> {
    let (mut a, mut b) = replicas("abcdef")?;
    // a and b delete `cd` concurrently; a learns of b's deletion.
    delete(&mut a, 2, 4)?;
    delete(&mut b, 2, 4)?;
    send(&b, &mut [&mut a])?;
    ack(&mut b)?;
    // a undoes and redoes: `cd` stays removed by b's deletion all along, so
    // a's redo removes nothing. b then undoes its deletion.
    undo(&mut a)?;
    redo(&mut a)?;
    undo(&mut b)?;
    exchange(&mut a, &mut b)?;

    // b revived `cd` before a's redo arrived; the redo, which saw `cd`
    // removed, leaves it alone there as it did on a.
    assert_eq!("abcdef", text(&a));
    assert_eq!("abcdef", text(&b));
    Ok(())
}

/// JS `propagates an undo that leaves the text as it was`.
#[test]
fn propagates_an_undo_that_leaves_the_text_as_it_was() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    // Both delete `a` concurrently; b learns of a's deletion first.
    delete(&mut a, 0, 1)?;
    delete(&mut b, 0, 1)?;
    send(&a, &mut [&mut b])?;
    ack(&mut a)?;

    // b's undo cancels its own deletion while a's still holds `a`, so b's
    // text does not change. Its change has to reach a all the same.
    undo(&mut a)?;
    undo(&mut b)?;
    assert_eq!("bcdefgh", text(&b));
    send(&b, &mut [&mut a])?;
    send(&a, &mut [&mut b])?;

    assert_eq!("abcdefgh", text(&a));
    assert_eq!("abcdefgh", text(&b));
    Ok(())
}

/// JS `redo leaves alone a piece removed in its view`.
#[test]
fn redo_leaves_alone_a_piece_removed_in_its_view() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    delete(&mut a, 0, 1)?;
    delete(&mut b, 0, 3)?;
    send(&b, &mut [&mut a])?;
    ack(&mut b)?;
    // b undoes before a hears of it, so in a's view `a` stays removed by b's
    // deletion: a's undo and redo leave it removed, and the redo must not
    // remove it on b, where b's undo already revived it.
    undo(&mut b)?;
    undo(&mut a)?;
    redo(&mut a)?;
    send(&a, &mut [&mut b])?;
    send(&b, &mut [&mut a])?;

    assert_eq!(text(&b), text(&a));
    assert_eq!("abcdefgh", text(&a));
    Ok(())
}

/// JS `recreates a purged piece before the piece that follows it`.
#[test]
fn recreates_a_purged_piece_before_the_piece_that_follows_it() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefghij")?;
    delete(&mut a, 1, 3)?;
    delete(&mut a, 0, 1)?;
    // b splits `defghij` by removing `e`, so two pieces of the insertion
    // survive after the gap a's undo will recreate.
    delete(&mut b, 4, 5)?;
    send(&a, &mut [&mut b])?;
    ack(&mut a)?;
    assert!(b.garbage_collect(&own_vv(&b)) > 0);

    // a revives `a` and removes `d`. On b, `a` must come back before `d`:
    // placed after a later piece, a's removal would run from `a` to the end
    // of the text.
    undo(&mut a)?;
    delete(&mut a, 1, 2)?;
    exchange(&mut a, &mut b)?;

    assert_eq!("afghij", text(&a));
    assert_eq!("afghij", text(&b));
    Ok(())
}

/// JS `keeps a piece until its cancelled removals are known everywhere`.
#[test]
fn keeps_a_piece_until_its_cancelled_removals_are_known_everywhere() -> Result<()> {
    let (mut a, mut b) = replicas("abc")?;
    let mut c = from_snapshot(&a)?;
    // a and b remove `b` concurrently; c hears only of a's removal and
    // removes the text around it.
    delete(&mut a, 1, 2)?;
    delete(&mut b, 1, 2)?;
    send(&a, &mut [&mut c])?;
    delete(&mut c, 0, 2)?;

    // a undoes its removal. b learns all of it, so only b's own removal
    // holds `b` there; c still knows nothing of a's undo.
    send(&a, &mut [&mut b])?;
    ack(&mut a)?;
    undo(&mut a)?;
    send(&a, &mut [&mut b])?;
    send(&b, &mut [&mut a, &mut c])?;
    ack(&mut b)?;
    let vv = min_vv(&[&a, &b, &c]);
    b.garbage_collect(&vv);

    // b's undo revives `b`; c's removal, which saw `b` removed by a, must
    // leave it alone on b as on a, whether or not b purged it.
    undo(&mut b)?;
    send(&b, &mut [&mut a, &mut c])?;
    send(&c, &mut [&mut a, &mut b])?;
    send(&a, &mut [&mut c])?;

    assert_eq!("b", text(&a));
    assert_eq!("b", text(&b));
    assert_eq!("b", text(&c));
    Ok(())
}

/// JS `links a recreated piece to the nearest surviving neighbours`.
#[test]
fn links_a_recreated_piece_to_the_nearest_surviving_neighbours() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefghij")?;
    // a removes `g`, then `h`; b collects both.
    delete(&mut a, 6, 7)?;
    delete(&mut a, 6, 7)?;
    send(&a, &mut [&mut b])?;
    ack(&mut a)?;
    assert!(b.garbage_collect(&own_vv(&b)) > 0);

    // b removes `f`, the piece before the gap, and a learns of it.
    delete(&mut b, 5, 6)?;
    send(&b, &mut [&mut a])?;
    ack(&mut b)?;

    // a's undo recreates `h` on b, which then collects `f` too: `f` still
    // pointed past the recreated `h`, so purging it must not hand `ij` an
    // insertion predecessor that skips `h`.
    undo(&mut a)?;
    send(&a, &mut [&mut b])?;
    ack(&mut a)?;
    let vv = min_vv(&[&a, &b]);
    assert!(b.garbage_collect(&vv) > 0);

    // An edit ending right after `h` resolves through `ij`'s predecessor.
    delete(&mut a, 5, 6)?;
    send(&a, &mut [&mut b])?;

    assert_eq!("abcdeij", text(&a));
    assert_eq!("abcdeij", text(&b));
    Ok(())
}

/// JS `keeps a recreated piece out of a deletion that saw it removed`.
#[test]
fn keeps_a_recreated_piece_out_of_a_deletion_that_saw_it_removed() -> Result<()> {
    let (mut a, mut b) = replicas("abc")?;
    let mut c = from_snapshot(&a)?;
    delete(&mut b, 1, 2)?;
    send(&b, &mut [&mut a, &mut c])?;
    ack(&mut b)?;
    let vv = min_vv(&[&a, &b, &c]);
    assert!(a.garbage_collect(&vv) > 0);

    // b undoes; c, which saw `b` removed but not the undo, removes the text
    // around it. a recreates `b` from the undo, so the recreated node has to
    // answer c's view as b's own ledger does.
    undo(&mut b)?;
    delete(&mut c, 0, 2)?;
    send(&b, &mut [&mut a, &mut c])?;
    send(&c, &mut [&mut a, &mut b])?;

    assert_eq!("b", text(&a));
    assert_eq!("b", text(&b));
    assert_eq!("b", text(&c));
    Ok(())
}

/// JS `loads a snapshot whose removal events break the rules`.
#[test]
fn loads_a_snapshot_whose_removal_events_break_the_rules() -> Result<()> {
    let (mut a, _) = replicas("abc")?;
    delete(&mut a, 1, 2)?;
    let mut wire = WireChangePack::try_from(&snapshot_pack("ledger", &a)?)?;

    // Repeat removed_at as an event: no undo can produce that.
    let Some(WireJsonElement::Object { nodes, .. }) = &mut wire.snapshot_root else {
        panic!("expected an object root");
    };
    let WireJsonElement::Text { nodes, .. } = &mut nodes[0].element else {
        panic!("expected a text");
    };
    let mut removed = 0;
    for node in nodes {
        if let Some(removed_at) = &node.removed_at {
            node.removal_events = vec![removed_at.clone()];
            removed += 1;
        }
    }
    assert!(removed > 0);

    // The bad ledger is dropped, not thrown on: the snapshot decode is the
    // attach, offline-restore and pull path, and a failure there wedges the
    // document for good over one node. The node keeps removed_at, so the
    // text still reads the same.
    let mut loaded = Document::new("ledger");
    loaded.set_actor(A3);
    loaded.apply_change_pack(&ChangePack::try_from(wire)?)?;
    assert_eq!("ac", text(&loaded));
    assert_eq!(0, events(&loaded));
    Ok(())
}

/// JS `reads back a snapshot whose ledger it grew itself`. It also guards
/// the ledger index: rebuilt per walk, this churn turns quadratic.
#[test]
fn reads_back_a_snapshot_whose_ledger_it_grew_itself() -> Result<()> {
    let (mut a, _) = replicas("abc")?;

    // A server holding this document writes the ledger into every snapshot
    // it builds, and a client has to load it whole. Churn past 4096 events,
    // the bound Go puts only on pushed payloads: a reader that bounded
    // snapshots too would drop this ledger and diverge from its peers.
    let grown = churn(&mut a, (1 << 11) + 8)?;
    assert!(grown > 1 << 12, "grew only {grown} events");
    let b = from_snapshot(&a)?;
    assert_eq!("abc", text(&b));
    assert_eq!(events(&a), events(&b));
    Ok(())
}

/// JS `drops cancelled removals every replica knows`.
#[test]
fn drops_cancelled_removals_every_replica_knows() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    for _ in 0..5 {
        delete(&mut a, 0, 8)?;
        undo(&mut a)?;
        exchange(&mut a, &mut b)?;
    }
    // Every removal is cancelled, but only a vector both replicas have seen
    // lets the pairs go; until then they decide later deletions.
    assert!(events(&a) > 0);
    let vv = min_vv(&[&a, &b]);
    a.garbage_collect(&vv);
    assert_eq!(0, events(&a));
    assert_eq!("abcdefgh", text(&a));
    Ok(())
}

/// JS `carries a cancelled removal through a snapshot`.
#[test]
fn carries_a_cancelled_removal_through_a_snapshot() -> Result<()> {
    let (mut a, mut b) = replicas("abcdefgh")?;
    // a deletes `d` and undoes it; the pair stays in the ledger while b has
    // not seen the cancel, so the snapshot has to carry both halves.
    delete(&mut a, 3, 4)?;
    undo(&mut a)?;
    assert!(events(&a) > 0);

    let mut c = from_snapshot(&a)?;
    assert!(events(&c) > 0);
    assert_eq!("abcdefgh", text(&c));

    // The replica loaded from that snapshot answers a later concurrent
    // deletion as the one that wrote it does: b never saw the undo, so its
    // deletion of `d` holds the piece removed on both.
    delete(&mut b, 3, 4)?;
    send(&a, &mut [&mut b])?;
    send(&b, &mut [&mut a, &mut c])?;
    assert_eq!("abcefgh", text(&a));
    assert_eq!(text(&a), text(&c));
    Ok(())
}

/// JS `reverses an undo that found nothing left to undo`.
#[test]
fn reverses_an_undo_that_found_nothing_left_to_undo() -> Result<()> {
    let (mut a, mut b) = replicas("ab")?;
    // b inserts `X`, then deletes `a` and undoes that deletion, leaving one
    // entry on each stack.
    edit(&mut b, 2, 2, "X")?;
    delete(&mut b, 0, 1)?;
    undo(&mut b)?;
    exchange(&mut a, &mut b)?;
    assert_eq!("abX", text(&b));

    // a removes `X`; b's undo of its own insertion then finds nothing live
    // to re-remove, so its reverse names no span.
    delete(&mut a, 2, 3)?;
    exchange(&mut a, &mut b)?;
    undo(&mut b)?;
    exchange(&mut a, &mut b)?;
    assert_eq!("ab", text(&b));

    // That reverse still has to be pushed: dropping it would leave the redo
    // stack one entry short, and this redo would re-run the deletion of `a`
    // instead of doing nothing.
    redo(&mut b)?;
    exchange(&mut a, &mut b)?;
    assert_eq!("ab", text(&b));
    assert_eq!("ab", text(&a));
    Ok(())
}

// --- Text restore after GC (JS `text_restore_after_gc_test.ts`) ---

/// JS `crossSync`: delivers each replica's pending changes to the other and
/// acknowledges them, without a server and without moving GC forward.
fn cross_sync(d1: &mut Document, d2: &mut Document) -> Result<()> {
    let p1 = through_wire(&d1.create_change_pack())?;
    let p2 = through_wire(&d2.create_change_pack())?;
    let pack = |key: &str, client_seq: u32, changes: Vec<crate::change::Change>| {
        ChangePack::create(
            key,
            Checkpoint::new(0, client_seq),
            false,
            changes,
            Some(VersionVector::new()),
            None,
        )
    };
    let last_seq = |p: &ChangePack| p.changes().last().map_or(0, |c| c.id().client_seq());
    d2.apply_change_pack(&pack(d2.key(), 0, p1.changes().to_vec()))?;
    d1.apply_change_pack(&pack(d1.key(), 0, p2.changes().to_vec()))?;
    d1.apply_change_pack(&pack(d1.key(), last_seq(&p1), Vec::new()))?;
    d2.apply_change_pack(&pack(d2.key(), last_seq(&p2), Vec::new()))
}

/// JS `maxVectorOf`: a vector that covers every ticket of the given actors.
fn max_vector_of(actors: &[&str]) -> VersionVector {
    let mut out = VersionVector::new();
    for actor in actors {
        out.set(*actor, i64::MAX);
    }
    out
}

fn restore_doc(key: &str, actor: &str) -> Document {
    let mut doc = Document::new(key);
    doc.set_actor(actor);
    doc
}

fn attrs<const N: usize>(entries: [(&str, &str); N]) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
        .collect()
}

/// The attributes of the `i`th run of `doc`'s text, empty when it has none.
fn attrs_at(doc: &Document, i: usize) -> BTreeMap<String, Value> {
    match doc.get_root().get("t") {
        Some(crate::JsonValue::Text(text)) => {
            text.values()[i].attributes.clone().unwrap_or_default()
        }
        other => panic!("expected a text at t, got {other:?}"),
    }
}

/// JS `recreates a purged multi-insertion run in document order on undo`
/// (wafflebase#629). Typing character by character makes each character its
/// own insertion, so none of the same-insertion anchor rungs fire on undo
/// after GC: each recreated fragment must chain after the one placed just
/// before it, or the run comes back reversed.
#[test]
fn recreates_a_purged_multi_insertion_run_in_document_order_on_undo() -> Result<()> {
    let mut doc = restore_doc("text-restore-after-gc", A1);
    doc.update(|root| root.set_text("t").map(|_| ()))?;

    let s = "hello my name is";
    for (i, ch) in s.chars().enumerate() {
        edit(&mut doc, i, i, &ch.to_string())?;
    }
    assert_eq!(s, text(&doc));

    delete(&mut doc, 6, 13)?; // delete "my name"
    assert_eq!("hello  is", text(&doc));

    // Purge the tombstones, so undo cannot un-tombstone in place and must
    // recreate from the spans.
    assert!(
        doc.garbage_collect(&max_vector_of(&[A1])) > 0,
        "the deleted run should be purged"
    );

    undo(&mut doc)?;
    assert_eq!(
        s,
        text(&doc),
        "a purged run must be recreated in document order, not reversed"
    );
    Ok(())
}

/// JS `single-insertion run is unaffected (one recreate)`.
#[test]
fn recreates_a_purged_single_insertion_run_on_undo() -> Result<()> {
    let mut doc = restore_doc("text-restore-after-gc-single", A1);
    doc.update(|root| root.set_text("t").map(|_| ()))?;
    edit(&mut doc, 0, 0, "hello my name is")?;
    delete(&mut doc, 6, 13)?;
    doc.garbage_collect(&max_vector_of(&[A1]));
    undo(&mut doc)?;
    assert_eq!("hello my name is", text(&doc));
    Ok(())
}

/// JS `converges when only the undoing replica purged the run (%s)`, both
/// cases. Which pieces survive is per-replica GC state, so where a restore
/// anchors must not depend on it: only the undoing replica has purged, so it
/// rebuilds the run through the anchor ladder while the peer revives its
/// tombstones in place. In the second case the text left of the run is
/// purged too, and the first fragment falls back to the operation's own
/// position.
#[test]
fn converges_when_only_the_undoing_replica_purged_the_run() -> Result<()> {
    for (name, purge_left) in [
        ("its neighbours survive", false),
        ("its left neighbour is purged too", true),
    ] {
        let mut d1 = restore_doc("text-restore-after-gc-peer", A1);
        let mut d2 = restore_doc("text-restore-after-gc-peer", A2);
        d1.update(|root| root.set_text("t").map(|_| ()))?;
        let s = "hello my name is";
        for (i, ch) in s.chars().enumerate() {
            edit(&mut d1, i, i, &ch.to_string())?;
        }
        cross_sync(&mut d1, &mut d2)?;
        clear_history(&mut d1);

        if purge_left {
            delete(&mut d2, 0, 6)?; // the peer deletes "hello "
            cross_sync(&mut d1, &mut d2)?;
        }
        let from = if purge_left { 0 } else { 6 };
        delete(&mut d1, from, from + 7)?; // delete "my name"
        cross_sync(&mut d1, &mut d2)?;

        assert!(d1.garbage_collect(&max_vector_of(&[A1, A2])) > 0, "{name}");
        undo(&mut d1)?;
        cross_sync(&mut d1, &mut d2)?;

        let want = if purge_left { "my name is" } else { s };
        assert_eq!(want, text(&d1), "{name}");
        assert_eq!(want, text(&d2), "{name}");
        assert_eq!(d1.to_sorted_json(), d2.to_sorted_json(), "{name}");
    }
    Ok(())
}

/// JS `keeps the attributes of a re-inserted run when redoing after GC`
/// (yorkie-js-sdk#1497). The reverse of an insertion records the inserted
/// run as a span; a replica that purged the node rebuilds it on redo from
/// that value alone, so the span carries the attributes the edit applied.
#[test]
fn keeps_the_attributes_of_a_re_inserted_run_when_redoing_after_gc() -> Result<()> {
    let mut d1 = restore_doc("text-redo-attrs-after-gc", A1);
    let mut d2 = restore_doc("text-redo-attrs-after-gc", A2);
    d1.update(|root| root.set_text("t")?.edit(0, 0, "ab", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    clear_history(&mut d1);

    d1.update(|root| {
        root.get_text_mut("t")?
            .edit(1, 1, "X", Some(attrs([("b", "1")])))
            .map(|_| ())
    })?;
    undo(&mut d1)?;
    cross_sync(&mut d1, &mut d2)?;

    // Only d1 purges the undone run, so its redo must recreate the node from
    // the span while d2 revives the tombstone it still holds.
    assert!(d1.garbage_collect(&max_vector_of(&[A1, A2])) > 0);
    redo(&mut d1)?;
    cross_sync(&mut d1, &mut d2)?;

    assert_eq!("aXb", text(&d1));
    assert_eq!(attrs([("b", "1")]), attrs_at(&d1, 1));
    assert_eq!(d1.to_sorted_json(), d2.to_sorted_json());
    Ok(())
}

/// JS `keeps a style applied after the insert when redoing after GC`
/// (yorkie-js-sdk#1497): the undo re-reads the span against the live
/// content before removing it, so the redo carries a style the peer added
/// after the insertion.
#[test]
fn keeps_a_style_applied_after_the_insert_when_redoing_after_gc() -> Result<()> {
    let mut d1 = restore_doc("text-redo-late-attrs-after-gc", A1);
    let mut d2 = restore_doc("text-redo-late-attrs-after-gc", A2);
    d1.update(|root| root.set_text("t")?.edit(0, 0, "ab", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    clear_history(&mut d1);

    edit(&mut d1, 1, 1, "X")?;
    cross_sync(&mut d1, &mut d2)?;

    // The peer styles the inserted run after the fact, so the span recorded
    // at insert time knows nothing about it.
    d2.update(|root| root.get_text_mut("t")?.set_style(1, 2, attrs([("b", "1")])))?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!(attrs([("b", "1")]), attrs_at(&d1, 1));

    undo(&mut d1)?;
    cross_sync(&mut d1, &mut d2)?;

    assert!(d1.garbage_collect(&max_vector_of(&[A1, A2])) > 0);
    redo(&mut d1)?;
    cross_sync(&mut d1, &mut d2)?;

    assert_eq!("aXb", text(&d1));
    assert_eq!(attrs([("b", "1")]), attrs_at(&d1, 1));
    assert_eq!(d1.to_sorted_json(), d2.to_sorted_json());
    Ok(())
}

/// JS `keeps the attributes across chained undo/redo with GC`: every round
/// purges the run again, so each redo recreates it from the span.
#[test]
fn keeps_the_attributes_across_chained_undo_redo_with_gc() -> Result<()> {
    let mut doc = restore_doc("text-redo-attrs-chained", A1);
    doc.update(|root| root.set_text("t")?.edit(0, 0, "ab", None).map(|_| ()))?;
    clear_history(&mut doc);
    doc.update(|root| {
        root.get_text_mut("t")?
            .edit(1, 1, "X", Some(attrs([("b", "1"), ("i", "1")])))
            .map(|_| ())
    })?;

    for round in 0..3 {
        undo(&mut doc)?;
        doc.garbage_collect(&max_vector_of(&[A1]));
        redo(&mut doc)?;

        assert_eq!("aXb", text(&doc), "round {round}");
        assert_eq!(
            attrs([("b", "1"), ("i", "1")]),
            attrs_at(&doc, 1),
            "round {round}"
        );
    }
    Ok(())
}

/// JS `keeps a style covering part of the insert when redoing after GC`:
/// the undo reports the pieces separately, so the redo recreates each with
/// its own attributes.
#[test]
fn keeps_a_style_covering_part_of_the_insert_when_redoing_after_gc() -> Result<()> {
    let mut d1 = restore_doc("text-redo-partial-attrs-after-gc", A1);
    let mut d2 = restore_doc("text-redo-partial-attrs-after-gc", A2);
    d1.update(|root| root.set_text("t")?.edit(0, 0, "ab", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    clear_history(&mut d1);

    edit(&mut d1, 1, 1, "XYZ")?;
    cross_sync(&mut d1, &mut d2)?;
    d2.update(|root| root.get_text_mut("t")?.set_style(2, 3, attrs([("b", "1")])))?;
    cross_sync(&mut d1, &mut d2)?;

    undo(&mut d1)?;
    cross_sync(&mut d1, &mut d2)?;
    d1.garbage_collect(&max_vector_of(&[A1, A2]));
    redo(&mut d1)?;
    cross_sync(&mut d1, &mut d2)?;

    assert_eq!("aXYZb", text(&d1));
    let Some(crate::JsonValue::Text(t)) = d1.get_root().get("t") else {
        panic!("expected a text at t");
    };
    let values = t
        .values()
        .into_iter()
        .map(|chunk| (chunk.content, chunk.attributes.unwrap_or_default()))
        .collect::<Vec<_>>();
    assert_eq!(
        vec![
            ("a".to_owned(), attrs([])),
            ("X".to_owned(), attrs([])),
            ("Y".to_owned(), attrs([("b", "1")])),
            ("Z".to_owned(), attrs([])),
            ("b".to_owned(), attrs([])),
        ],
        values
    );
    assert_eq!(d1.to_sorted_json(), d2.to_sorted_json());
    Ok(())
}

/// JS `keeps a plain re-inserted run unstyled when redoing after GC`.
#[test]
fn keeps_a_plain_re_inserted_run_unstyled_when_redoing_after_gc() -> Result<()> {
    let mut doc = restore_doc("text-redo-plain-after-gc", A1);
    doc.update(|root| root.set_text("t")?.edit(0, 0, "ab", None).map(|_| ()))?;
    clear_history(&mut doc);
    edit(&mut doc, 1, 1, "X")?;

    undo(&mut doc)?;
    doc.garbage_collect(&max_vector_of(&[A1]));
    redo(&mut doc)?;

    assert_eq!("aXb", text(&doc));
    assert!(attrs_at(&doc, 1).is_empty());
    Ok(())
}
