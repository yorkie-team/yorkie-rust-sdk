//! Tests for garbage collection on a document.
//!
//! The first group checks the wiring: `apply_change_pack` collects with the
//! pack's version vector, as JS `Document.applyChangePack` does, and never
//! touches a tombstone that vector does not cover, such as one a pending
//! local change made.
//!
//! The second group checks that collection does not reorder what survives
//! it: a tombstone is purged only once the node behind it is causally stable,
//! so a replica that collected places a concurrent insert where a replica
//! that did not would. JS reference: `test/unit/document/gc_rga_barrier_test.ts`
//! (`GC successor barrier`), a port of Go `gc_rga_barrier_test.go` (yorkie
//! `ba82ed91`). Every vector that authorises a purge is one the server could
//! compute: the element-wise min of the replicas' own vectors.

use super::Document;
use crate::change::{Change, ChangePack, Checkpoint};
use crate::crdt::root::CrdtRoot;
use crate::wire::WireChange;
use crate::{JsonArray, Result, TreeNodeInput, VersionVector};

const A1: &str = "000000000000000000000001";
const A2: &str = "000000000000000000000002";

fn replicas() -> (Document, Document) {
    let mut d1 = Document::new("test-doc");
    let mut d2 = Document::new("test-doc");
    d1.set_actor(A1);
    d2.set_actor(A2);
    (d1, d2)
}

/// Applies `changes` to `d`, as a pull from the server would.
fn pull(d: &mut Document, changes: Vec<Change>) -> Result<()> {
    let pack = ChangePack::create(
        d.key(),
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    );
    d.apply_change_pack(&pack)
}

/// Hands `d`'s pending changes to the server, through the wire shape, and
/// acks them at `d`.
fn push(d: &mut Document) -> Result<Vec<Change>> {
    let pack = d.create_change_pack();
    let changes = pack
        .changes()
        .iter()
        .map(|change| Change::try_from(WireChange::try_from(change)?))
        .collect::<Result<Vec<_>>>()?;
    let last_seq = pack.changes().last().map_or(0, |c| c.id().client_seq());
    let ack = ChangePack::create(
        d.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    );
    d.apply_change_pack(&ack)?;
    Ok(changes)
}

/// Hands `from`'s pending changes to `to`: a push followed by a pull.
fn one_way_deliver(from: &mut Document, to: &mut Document) -> Result<()> {
    let changes = push(from)?;
    pull(to, changes)
}

fn cross_sync(d1: &mut Document, d2: &mut Document) -> Result<()> {
    let c1 = push(d1)?;
    let c2 = push(d2)?;
    pull(d2, c1)?;
    pull(d1, c2)
}

fn version_vector(d: &Document) -> VersionVector {
    d.version_vector().clone()
}

/// Go `time.MinVersionVector`: the element-wise min, 0 for a missing actor.
fn min_vv(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = VersionVector::new();
    for (actor, _) in a.iter().chain(b.iter()) {
        let lamport = a.get(actor.as_str()).unwrap_or(0);
        out.set(
            actor.clone(),
            lamport.min(b.get(actor.as_str()).unwrap_or(0)),
        );
    }
    out
}

/// A vector that covers every ticket of the given actors.
fn max_vector_of(actors: &[&str]) -> VersionVector {
    let mut out = VersionVector::new();
    for actor in actors {
        out.set(*actor, i64::MAX);
    }
    out
}

fn collect(d: &mut Document, vector: &VersionVector) -> usize {
    d.garbage_collect(vector)
}

/// Checks that holding a purge back is a delay, not a leak, and that the
/// garbage count matches a root rebuilt from the same content.
///
/// JS also compares the running `docSize` with the rebuilt one. That part is
/// left out: the Rust running size does not move a removed text node or a
/// move's ticket the way a rebuild counts them, with or without collection
/// (see the porting gaps).
fn assert_drains(d: &mut Document, name: &str) -> Result<()> {
    collect(d, &max_vector_of(&[A1, A2]));
    assert_eq!(0, d.garbage_len(), "{name} leaked garbage");
    let rebuilt = CrdtRoot::new(d.crdt_root.object().deepcopy());
    assert_eq!(
        rebuilt.get_garbage_len(),
        d.garbage_len(),
        "{name}: garbage"
    );
    Ok(())
}

fn bump(d: &mut Document, prefix: &str, n: i32) -> Result<()> {
    for i in 0..n {
        d.update(|root| root.set(format!("{prefix}-{i}"), i).map(|_| ()))?;
    }
    Ok(())
}

/// An ack whose vector covers what this document pushed.
fn ack_with_vector(d: &Document, client_seq: u32, vector: VersionVector) -> ChangePack {
    ChangePack::create(
        d.key(),
        Checkpoint::new(1, client_seq),
        false,
        Vec::new(),
        Some(vector),
        None,
    )
}

/// JS `should collect garbage` (`test/unit/document/gc_test.ts`):
/// `garbage_len` counts an element with its descendants, and
/// `garbage_collect` returns how many it purged.
#[test]
fn counts_and_collects_removed_elements_with_their_descendants() -> Result<()> {
    let (mut doc, _) = replicas();
    doc.update(|root| {
        root.set("1", 1)?;
        root.set("2", JsonArray::new())?;
        root.get_array_mut("2")?.push(1)?.push(2)?.push(3)?;
        root.set("3", 3)?;
        Ok(())
    })?;
    assert_eq!(r#"{"1":1,"2":[1,2,3],"3":3}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.remove("2");
        Ok(())
    })?;
    assert_eq!(r#"{"1":1,"3":3}"#, doc.to_sorted_json());
    assert_eq!(4, doc.garbage_len());
    assert_eq!(4, doc.garbage_collect(&max_vector_of(&[A1])));
    assert_eq!(0, doc.garbage_len());
    Ok(())
}

/// JS `Document.applyChangePack` step 03: a pack without a snapshot collects
/// with its version vector once everything in it has been applied.
#[test]
fn collects_with_the_vector_of_an_applied_change_pack() -> Result<()> {
    let (mut doc, _) = replicas();
    doc.update(|root| root.set("k", 1).map(|_| ()))?;
    doc.update(|root| {
        root.remove("k");
        Ok(())
    })?;
    assert_eq!(1, doc.garbage_len());

    let vector = version_vector(&doc);
    doc.apply_change_pack(&ack_with_vector(&doc, 2, vector))?;
    assert_eq!(0, doc.garbage_len());
    assert_eq!("{}", doc.to_sorted_json());
    Ok(())
}

/// The server's vector covers only what this document pushed, so a removal
/// made after that push, still pending, keeps its tombstone.
#[test]
fn keeps_a_tombstone_a_pending_local_change_made() -> Result<()> {
    let (mut doc, _) = replicas();
    doc.update(|root| root.set("k", 1).map(|_| ()))?;
    let pushed = version_vector(&doc);

    doc.update(|root| {
        root.remove("k");
        Ok(())
    })?;
    doc.apply_change_pack(&ack_with_vector(&doc, 1, pushed))?;

    assert_eq!(1, doc.garbage_len());
    assert!(doc.has_local_changes());
    Ok(())
}

/// A removed array and a removed element inside it are purged once each,
/// whichever of the two collection visits first. The child's ID sorts
/// before its parent's ("10:..." before "9:..."), so the child is visited
/// first. A later removal the vector does not cover stays, and the gc size
/// left over matches a root rebuilt from the same content.
#[test]
fn purges_a_removed_child_and_its_removed_parent_once() -> Result<()> {
    let (mut doc, _) = replicas();
    bump(&mut doc, "pad", 8)?;
    doc.update(|root| root.set("p", JsonArray::new()).map(|_| ()))?; // lamport 9
    doc.update(|root| root.get_array_mut("p")?.push(1).map(|_| ()))?; // 10
    doc.update(|root| {
        root.get_array_mut("p")?.delete(0);
        Ok(())
    })?; // 11
    doc.update(|root| {
        root.remove("p");
        Ok(())
    })?; // 12
    let mut covered = VersionVector::new();
    covered.set(A1, 12);

    // Garbage the vector does not cover, so an undercount would show.
    doc.update(|root| {
        root.remove("pad-0");
        Ok(())
    })?;
    let before = doc.garbage_len();
    assert_eq!(3, before);

    assert_eq!(2, doc.garbage_collect(&covered));
    assert_eq!(1, doc.garbage_len());
    let rebuilt = CrdtRoot::new(doc.crdt_root.object().deepcopy());
    assert_ne!(crate::crdt::element::DataSize::default(), doc.doc_size().gc);
    assert_eq!(rebuilt.doc_size().gc, doc.doc_size().gc);
    assert_eq!(rebuilt.get_garbage_len(), doc.garbage_len());

    assert_eq!(1, doc.garbage_collect(&max_vector_of(&[A1])));
    assert_eq!(0, doc.garbage_len());
    assert_eq!(crate::crdt::element::DataSize::default(), doc.doc_size().gc);
    Ok(())
}

/// Checks that the running gc size matches a root rebuilt from the same
/// content. Only gc: removing a text or tree node leaves its size in the
/// running `live` (see the porting gaps), so `live` cannot be compared here.
fn assert_gc_size_matches_rebuilt(d: &Document) {
    let rebuilt = CrdtRoot::new(d.crdt_root.object().deepcopy());
    assert_eq!(rebuilt.doc_size().gc, d.doc_size().gc, "gc vs rebuilt");
}

/// A GC pair whose container was purged before it is dropped, as JS
/// `CRDTRoot.collect` purges it through the detached parent it holds:
/// deleting `<p>ab</p>` leaves a pair for the paragraph and one for its text,
/// and purging the paragraph takes the text node with it.
#[test]
fn drops_the_gc_pairs_under_a_purged_tree_node() -> Result<()> {
    let (mut doc, _) = replicas();
    doc.update(|root| {
        root.set_tree(
            "t",
            TreeNodeInput::element(
                "doc",
                vec![
                    TreeNodeInput::element("p", vec![TreeNodeInput::text("ab")]),
                    TreeNodeInput::element("p", Vec::new()),
                ],
            ),
        )
        .map(|_| ())
    })?;
    doc.update(|root| root.get_tree_mut("t")?.edit(0, 4, Vec::new(), 0))?;
    assert_eq!("<doc><p></p></doc>", xml(&doc)?);
    assert_eq!(2, doc.garbage_len());

    assert_eq!(2, doc.garbage_collect(&max_vector_of(&[A1])));
    assert_eq!(0, doc.garbage_len());
    assert_eq!("<doc><p></p></doc>", xml(&doc)?);
    assert_eq!(crate::crdt::element::DataSize::default(), doc.doc_size().gc);
    assert_gc_size_matches_rebuilt(&doc);
    Ok(())
}

/// The GC pairs of a Text inside a removed object are dropped once the
/// object is purged, as JS drops them through the detached Text it holds.
#[test]
fn drops_the_gc_pairs_of_a_text_in_a_purged_object() -> Result<()> {
    let (mut doc, _) = replicas();
    doc.update(|root| {
        root.set("o", crate::JsonObject::new())?;
        root.get_object_mut("o")?
            .set_text("t")?
            .edit(0, 0, "abc", None)?;
        root.set("keep", 1)?;
        Ok(())
    })?;
    doc.update(|root| {
        root.get_object_mut("o")?
            .get_text_mut("t")?
            .edit(1, 2, "", None)
            .map(|_| ())
    })?;
    doc.update(|root| {
        root.remove("o");
        Ok(())
    })?;
    assert_eq!(r#"{"keep":1}"#, doc.to_sorted_json());
    assert_eq!(3, doc.garbage_len());

    doc.garbage_collect(&max_vector_of(&[A1]));
    assert_eq!(0, doc.garbage_len());
    assert_eq!(r#"{"keep":1}"#, doc.to_sorted_json());
    assert_eq!(crate::crdt::element::DataSize::default(), doc.doc_size().gc);
    assert_gc_size_matches_rebuilt(&doc);
    Ok(())
}

/// Checks that the running document size matches a root rebuilt from the
/// same content, for documents with no text or tree node removals.
fn assert_doc_size_matches_rebuilt(d: &Document, name: &str) {
    let rebuilt = CrdtRoot::new(d.crdt_root.object().deepcopy());
    assert_eq!(rebuilt.doc_size(), d.doc_size(), "{name}: doc_size");
}

/// Removing a container moves its descendants' size to gc with it, and
/// collecting it takes them out of gc, as JS `moveDescendantsToGC` and
/// `deregisterElement` do. A container's own size is only its metadata, so
/// moving the container alone left the members charged to live for good.
/// The inner object is removed first, so the outer removal must not move
/// the members under it a second time.
#[test]
fn moves_the_descendants_of_a_removed_container_to_gc() -> Result<()> {
    let (mut doc, _) = replicas();
    doc.update(|root| {
        root.set("o", crate::JsonObject::new())?;
        let o = root.get_object_mut("o")?;
        o.set("a", "x".repeat(10))?.set("b", "y".repeat(10))?;
        o.set("p", crate::JsonObject::new())?;
        o.get_object_mut("p")?.set("c", "z".repeat(10))?;
        root.set("keep", 1)?;
        Ok(())
    })?;
    assert_doc_size_matches_rebuilt(&doc, "created");

    doc.update(|root| {
        root.get_object_mut("o")?.remove("p");
        Ok(())
    })?;
    assert_doc_size_matches_rebuilt(&doc, "inner removed");
    let inner_removed = version_vector(&doc);

    doc.update(|root| {
        root.remove("o");
        Ok(())
    })?;
    assert_eq!(r#"{"keep":1}"#, doc.to_sorted_json());
    assert_doc_size_matches_rebuilt(&doc, "outer removed");

    collect(&mut doc, &inner_removed);
    assert_doc_size_matches_rebuilt(&doc, "inner collected");

    collect(&mut doc, &max_vector_of(&[A1]));
    assert_eq!(0, doc.garbage_len());
    assert_eq!(crate::crdt::element::DataSize::default(), doc.doc_size().gc);
    assert_doc_size_matches_rebuilt(&doc, "outer collected");
    Ok(())
}

/// The value that loses a concurrent set arrives already removed, and the
/// size registered for it held its removal ticket, so moving it to gc gives
/// live nothing back (JS `adoptRemovedElement`). A2 wins the tie, so the
/// loser is A1's value, which reaches d2 already removed.
#[test]
fn adopts_the_value_that_lost_a_concurrent_set() -> Result<()> {
    let (mut d1, mut d2) = replicas();
    d1.update(|root| root.set("k", 1).map(|_| ()))?;
    d2.update(|root| root.set("k", 2).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!(r#"{"k":2}"#, d1.to_sorted_json());
    assert_eq!(r#"{"k":2}"#, d2.to_sorted_json());

    for (doc, name) in [(&mut d1, "d1"), (&mut d2, "d2")] {
        assert_doc_size_matches_rebuilt(doc, name);
        collect(doc, &max_vector_of(&[A1, A2]));
        assert_eq!(0, doc.garbage_len(), "{name}: garbage");
        assert_doc_size_matches_rebuilt(doc, name);
    }
    Ok(())
}

/// A member a peer set into a container this replica had already removed
/// never went through a removal, so it is charged to live; collecting the
/// container takes it out of live, not gc, as JS `deregisterElement` takes
/// out of each side what that side was charged.
#[test]
fn collects_a_member_set_into_an_already_removed_container() -> Result<()> {
    let (mut d1, mut d2) = replicas();
    d1.update(|root| {
        root.set("o", crate::JsonObject::new())?;
        root.get_object_mut("o")?.set("a", "x".repeat(10))?;
        Ok(())
    })?;
    cross_sync(&mut d1, &mut d2)?;

    d1.update(|root| {
        root.remove("o");
        Ok(())
    })?;
    d2.update(|root| {
        root.get_object_mut("o")?
            .set("b", "y".repeat(10))
            .map(|_| ())
    })?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!("{}", d1.to_sorted_json());
    assert_eq!(d1.to_sorted_json(), d2.to_sorted_json());

    for (doc, name) in [(&mut d1, "d1"), (&mut d2, "d2")] {
        collect(doc, &max_vector_of(&[A1, A2]));
        assert_eq!(0, doc.garbage_len(), "{name}: garbage");
        assert_doc_size_matches_rebuilt(doc, name);
    }
    Ok(())
}

/// JS checks the size limit against live plus gc, and collection takes a
/// removed container out of both, so content replacing it that fits the
/// limit is accepted once the container is collected.
#[test]
fn accepts_an_update_within_the_limit_after_collecting_a_container() -> Result<()> {
    let (mut doc, _) = replicas();
    let content = |root: &mut crate::JsonObject, key: &str| -> Result<()> {
        root.set(key, crate::JsonObject::new())?;
        root.get_object_mut(key)?
            .set("a", "x".repeat(100))?
            .set("b", "y".repeat(100))?;
        Ok(())
    };
    doc.update(|root| content(root, "o"))?;
    let limit = doc.doc_size().total();

    doc.update(|root| {
        root.remove("o");
        Ok(())
    })?;
    collect(&mut doc, &max_vector_of(&[A1]));
    assert_eq!("{}", doc.to_sorted_json());

    doc.set_max_size_per_document(limit);
    doc.update(|root| content(root, "n"))?;
    assert_eq!(limit, doc.doc_size().total());
    Ok(())
}

/// A text edit made after collection anchors on what survived it, and a
/// replica that did not collect reaches the same text.
#[test]
fn edits_text_after_its_tombstones_were_collected() -> Result<()> {
    let (mut doc, mut replica) = replicas();
    doc.update(|root| root.set_text("t")?.edit(0, 0, "abc", None).map(|_| ()))?;
    doc.update(|root| root.get_text_mut("t")?.edit(1, 2, "", None).map(|_| ()))?;
    one_way_deliver(&mut doc, &mut replica)?;
    assert_eq!(1, doc.garbage_len());

    let vector = version_vector(&doc);
    doc.apply_change_pack(&ack_with_vector(&doc, 2, vector))?;
    assert_eq!(0, doc.garbage_len());

    doc.update(|root| root.get_text_mut("t")?.edit(1, 1, "x", None).map(|_| ()))?;
    doc.update(|root| root.get_text_mut("t")?.edit(2, 3, "", None).map(|_| ()))?;
    assert_eq!("ax", text(&doc));

    // The replica did not collect, and reaches the same text.
    one_way_deliver(&mut doc, &mut replica)?;
    assert_eq!(doc.to_sorted_json(), replica.to_sorted_json());
    Ok(())
}

/// JS `keeps insert order when a removed element was concurrently moved`.
#[test]
fn keeps_insert_order_when_a_removed_element_was_concurrently_moved() -> Result<()> {
    let (mut remover, mut mover) = replicas();

    remover.update(|root| {
        root.set("arr", JsonArray::new())?;
        root.get_array_mut("arr")?.push("a")?.push("w")?;
        Ok(())
    })?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!(r#"{"arr":["a","w"]}"#, mover.to_sorted_json());

    // Concurrent branch 1 (remover): remove "a".
    remover.update(|root| {
        root.get_array_mut("arr")?.delete(0);
        Ok(())
    })?;

    // Concurrent branch 2 (mover): move "a" after "w", then, after a few
    // unrelated changes so its ticket outruns the insert below, append "s"
    // anchored on "a"'s new position.
    mover.update(|root| {
        root.get_array_mut("arr")?
            .move_after_by_index(1, 0)
            .map(|_| ())
    })?;
    bump(&mut mover, "mover", 3)?;
    mover.update(|root| {
        let arr = root.get_array_mut("arr")?;
        let id = arr.get_element_by_index(1).expect("a").id().clone();
        arr.insert_after(&id, "s").map(|_| ())
    })?;
    assert_eq!(r#"["w","a","s"]"#, arr(&mover));

    // The remover pushes only the removal: it is causally stable, the move
    // and the append are not.
    one_way_deliver(&mut remover, &mut mover)?;
    assert_eq!(r#"["w","s"]"#, arr(&mover));
    let vv = min_vv(&version_vector(&remover), &version_vector(&mover));

    // The remover keeps editing without pulling: an insert anchored on "w",
    // with a ticket strictly between the move and the mover's append.
    bump(&mut remover, "remover", 1)?;
    remover.update(|root| {
        let arr = root.get_array_mut("arr")?;
        let id = arr.get_element_by_index(0).expect("w").id().clone();
        arr.insert_after(&id, "x").map(|_| ())
    })?;
    assert_eq!(r#"["w","x"]"#, arr(&remover));

    // The mover collects with that vector: the only difference between them.
    collect(&mut mover, &vv);

    one_way_deliver(&mut remover, &mut mover)?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!(remover.to_sorted_json(), mover.to_sorted_json());

    assert_drains(&mut remover, "remover")?;
    assert_drains(&mut mover, "mover")?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!(remover.to_sorted_json(), mover.to_sorted_json());
    Ok(())
}

/// JS `keeps text order after collecting a concurrently deleted node`.
#[test]
fn keeps_text_order_after_collecting_a_concurrently_deleted_node() -> Result<()> {
    let (mut remover, mut mover) = replicas();

    remover.update(|root| {
        let text = root.set_text("t")?;
        text.edit(0, 0, "A", None)?;
        text.edit(0, 0, "W", None)?;
        Ok(())
    })?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!("WA", text(&mover));

    // mover: bump its clock, then append "S" anchored right after "A".
    bump(&mut mover, "mover", 3)?;
    mover.update(|root| root.get_text_mut("t")?.edit(2, 2, "S", None).map(|_| ()))?;
    assert_eq!("WAS", text(&mover));

    // remover: delete "A". It reaches the mover, so it is causally stable.
    remover.update(|root| root.get_text_mut("t")?.edit(1, 2, "", None).map(|_| ()))?;
    one_way_deliver(&mut remover, &mut mover)?;
    assert_eq!("WS", text(&mover));
    let vv = min_vv(&version_vector(&remover), &version_vector(&mover));

    // remover keeps editing without pulling: insert "X" right after "W".
    bump(&mut remover, "remover", 1)?;
    remover.update(|root| root.get_text_mut("t")?.edit(1, 1, "X", None).map(|_| ()))?;
    assert_eq!("WX", text(&remover));

    collect(&mut mover, &vv);
    one_way_deliver(&mut remover, &mut mover)?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!(remover.to_sorted_json(), mover.to_sorted_json());

    assert_drains(&mut remover, "remover")?;
    assert_drains(&mut mover, "mover")?;
    Ok(())
}

/// JS `keeps tree order after collecting a concurrently deleted node`.
#[test]
fn keeps_tree_order_after_collecting_a_concurrently_deleted_node() -> Result<()> {
    let (mut remover, mut mover) = replicas();

    remover.update(|root| {
        let tree = root.set_tree(
            "t",
            TreeNodeInput::element("doc", vec![TreeNodeInput::element("p", Vec::new())]),
        )?;
        tree.edit(1, 1, vec![TreeNodeInput::text("A")], 0)?;
        tree.edit(1, 1, vec![TreeNodeInput::text("W")], 0)?;
        Ok(())
    })?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!("<doc><p>WA</p></doc>", xml(&mover)?);

    bump(&mut mover, "mover", 3)?;
    mover.update(|root| {
        root.get_tree_mut("t")?
            .edit(3, 3, vec![TreeNodeInput::text("S")], 0)
    })?;
    assert_eq!("<doc><p>WAS</p></doc>", xml(&mover)?);

    remover.update(|root| root.get_tree_mut("t")?.edit(2, 3, Vec::new(), 0))?;
    one_way_deliver(&mut remover, &mut mover)?;
    assert_eq!("<doc><p>WS</p></doc>", xml(&mover)?);
    let vv = min_vv(&version_vector(&remover), &version_vector(&mover));

    bump(&mut remover, "remover", 1)?;
    remover.update(|root| {
        root.get_tree_mut("t")?
            .edit(2, 2, vec![TreeNodeInput::text("X")], 0)
    })?;
    assert_eq!("<doc><p>WX</p></doc>", xml(&remover)?);

    collect(&mut mover, &vv);
    one_way_deliver(&mut remover, &mut mover)?;
    cross_sync(&mut remover, &mut mover)?;
    assert_eq!(xml(&remover)?, xml(&mover)?);

    assert_drains(&mut remover, "remover")?;
    assert_drains(&mut mover, "mover")?;
    Ok(())
}

fn xml(d: &Document) -> Result<String> {
    match d.get_root().get("t") {
        Some(crate::JsonValue::Tree(tree)) => tree.to_xml(),
        other => panic!("expected a tree at t, got {other:?}"),
    }
}

fn arr(d: &Document) -> String {
    d.crdt_root.object().get("arr").expect("arr").to_json()
}

fn text(d: &Document) -> String {
    match d.get_root().get("t") {
        Some(crate::JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

/// Counts the removed nodes still in the text at `t`.
fn text_tombstones(d: &Document) -> usize {
    match d.crdt_root.object().get("t") {
        Some(crate::crdt::element::CrdtElement::Text(text)) => {
            text.nodes().filter(|node| node.is_removed()).count()
        }
        other => panic!("expected a text at t, got {other:?}"),
    }
}

/// The size of the one tombstone left in the text at `t`, the piece `cd` two
/// concurrent deletions hold (JS `heldTombstoneSize`).
fn held_tombstone_size(d: &Document) -> crate::crdt::element::DataSize {
    match d.crdt_root.object().get("t") {
        Some(crate::crdt::element::CrdtElement::Text(text)) => text
            .nodes()
            .find(|node| node.is_removed())
            .map(|node| {
                assert_eq!("cd", node.value().content());
                node.data_size()
            })
            .unwrap_or_default(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

/// JS `test/unit/document/gc_split_leak_test.ts` "GC tombstone-split leak
/// (Text)" / `purges pieces split off a tombstoned text node` (`84068f70`):
/// a piece split off a removed node is born removed, so the split registers
/// it for collection instead of charging live. `cd` is removed by both
/// deletions, so it stays until an undo leaves one of them.
#[test]
fn purges_pieces_split_off_a_tombstoned_text_node() -> Result<()> {
    let (mut d1, mut d2) = replicas();
    d1.update(|root| root.set_text("t")?.edit(0, 0, "abcdef", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;

    // d1 removes the whole node; d2 concurrently removes a middle slice.
    // When d2's deletion reaches d1, it splits d1's tombstone into three
    // pieces, and the piece after the removed range is born removed.
    d1.update(|root| root.get_text_mut("t")?.edit(0, 6, "", None).map(|_| ()))?;
    d2.update(|root| root.get_text_mut("t")?.edit(2, 4, "", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!("", text(&d1));
    assert_eq!("", text(&d2));

    let purged1 = collect(&mut d1, &max_vector_of(&[A1, A2]));
    let purged2 = collect(&mut d2, &max_vector_of(&[A1, A2]));
    assert_eq!(purged1, purged2, "asymmetric purge for identical state");
    // Every other piece, the born-removed remainder included, is purged.
    for (d, name) in [(&d1, "d1"), (&d2, "d2")] {
        assert_eq!(1, text_tombstones(d), "{name}: tombstones");
        assert_eq!(1, d.garbage_len(), "{name}: garbage");
        assert_eq!(held_tombstone_size(d), d.doc_size().gc, "{name}: gc");
        assert_gc_size_matches_rebuilt(d);
    }
    // JS compares the whole docSize across the replicas. Live is left out:
    // a text removal does not move its size out of the running live size,
    // and the two replicas split in a different order (see the porting gaps).
    assert_eq!(d2.doc_size().gc, d1.doc_size().gc);
    Ok(())
}

/// JS `test/unit/document/gc_split_leak_test.ts` "GC tombstone-split leak
/// (Text)" / `keeps GC registration when a newer concurrent delete
/// overwrites a tombstone` (`84068f70`): d1's whole-range deletion is newer
/// than d2's slice deletion, so on d2 it overwrites d2's own tombstone by
/// LWW. That must not register the node a second time, which would cancel
/// its registration; `cd` is again held by both deletions.
#[test]
fn keeps_gc_registration_when_a_newer_concurrent_delete_overwrites_a_tombstone() -> Result<()> {
    let (mut d1, mut d2) = replicas();
    d1.update(|root| root.set_text("t")?.edit(0, 0, "abcdef", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;

    d1.update(|root| root.get_text_mut("t")?.edit(6, 6, "!", None).map(|_| ()))?;
    d1.update(|root| root.get_text_mut("t")?.edit(0, 7, "", None).map(|_| ()))?;
    d2.update(|root| root.get_text_mut("t")?.edit(2, 4, "", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!("", text(&d1));
    assert_eq!("", text(&d2));

    let purged1 = collect(&mut d1, &max_vector_of(&[A1, A2]));
    let purged2 = collect(&mut d2, &max_vector_of(&[A1, A2]));
    assert_eq!(purged1, purged2, "asymmetric purge for identical state");
    for (d, name) in [(&d1, "d1"), (&d2, "d2")] {
        assert_eq!(1, text_tombstones(d), "{name}: tombstones");
        assert_eq!(1, d.garbage_len(), "{name}: garbage");
        assert_gc_size_matches_rebuilt(d);
    }
    Ok(())
}

/// JS `4f1f0123` for a style: a style whose boundaries split a removed node
/// registers the born-removed pieces, as JS `CRDTText.setStyle` drains them
/// into its pairs.
#[test]
fn purges_pieces_a_style_split_off_a_tombstoned_text_node() -> Result<()> {
    let (mut d1, mut d2) = replicas();
    d1.update(|root| root.set_text("t")?.edit(0, 0, "abcdef", None).map(|_| ()))?;
    cross_sync(&mut d1, &mut d2)?;

    d1.update(|root| root.get_text_mut("t")?.edit(0, 6, "", None).map(|_| ()))?;
    d2.update(|root| {
        let mut bold = std::collections::BTreeMap::new();
        bold.insert("b".to_owned(), serde_json::Value::Bool(true));
        root.get_text_mut("t")?.set_style(2, 4, bold)
    })?;
    cross_sync(&mut d1, &mut d2)?;
    assert_eq!("", text(&d1));
    assert_eq!("", text(&d2));

    let purged1 = collect(&mut d1, &max_vector_of(&[A1, A2]));
    let purged2 = collect(&mut d2, &max_vector_of(&[A1, A2]));
    assert_eq!(purged1, purged2, "asymmetric purge for identical state");
    for (d, name) in [(&d1, "d1"), (&d2, "d2")] {
        assert_eq!(0, text_tombstones(d), "{name}: tombstones");
        assert_eq!(0, d.garbage_len(), "{name}: garbage");
        assert_gc_size_matches_rebuilt(d);
    }
    Ok(())
}

// The Text removal ledger in collection, at the CRDT level: a root built
// over a text, edited and restored the way `EditOperation` books it (JS
// `CRDTRoot.garbageCollect`, `collect`, `pruneLedgers`, `unregisterGCPair`).

mod ledger {
    use crate::crdt::element::{CrdtElement, DataSize};
    use crate::crdt::object::CrdtObject;
    use crate::crdt::rga_tree_split::{RestoreSpan, RgaTreeSplitPosRange};
    use crate::crdt::root::CrdtRoot;
    use crate::crdt::text::{CrdtText, TextValue};
    use crate::{Result, TimeTicket, VersionVector};

    use super::{A1, A2};

    fn a(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, A1)
    }

    fn b(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, A2)
    }

    fn vv<const N: usize>(entries: [(&str, i64); N]) -> VersionVector {
        let mut vector = VersionVector::new();
        for (actor, lamport) in entries {
            vector.set(actor, lamport);
        }
        vector
    }

    fn span(
        created_at: TimeTicket,
        start: usize,
        end: usize,
        content: &str,
    ) -> RestoreSpan<TextValue> {
        RestoreSpan {
            created_at,
            start,
            end,
            value: TextValue::create(content),
        }
    }

    /// A text created at a(1) holding `content`, inserted at a(2).
    fn seeded(content: &str) -> Result<CrdtText> {
        let mut text = CrdtText::create(a(1));
        text.edit_by_index(0, 0, content, None, a(2), None)?;
        Ok(text)
    }

    /// A root holding `text` at `t`, built the way a snapshot is loaded.
    fn root_with(text: CrdtText) -> CrdtRoot {
        CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("t", CrdtElement::text(text))],
        ))
    }

    fn text_of(root: &CrdtRoot) -> &CrdtText {
        root.text_by_created_at(&a(1)).expect("the text at t")
    }

    fn range(root: &CrdtRoot, from: usize, to: usize) -> Result<RgaTreeSplitPosRange> {
        text_of(root).index_range_to_pos_range(from, to)
    }

    /// Removes `range` from the text in `root` and books it as
    /// `EditOperation` does.
    fn remove(
        root: &mut CrdtRoot,
        range: RgaTreeSplitPosRange,
        at: TimeTicket,
        vector: Option<&VersionVector>,
    ) -> Result<()> {
        let result = root
            .text_by_created_at_mut(&a(1))
            .expect("the text at t")
            .edit_by_range(range, "", None, at, vector)?;
        root.acc(result.diff);
        for node in result.removed_nodes {
            if let Some(removed_at) = node.removed_at() {
                root.register_gc_pair_by_id(node.id_string(), node.data_size(), removed_at.clone());
            }
        }
        for (child_id, child_size, removed_at) in result.split_gc_pairs {
            root.register_gc_pair_by_id(child_id, child_size, removed_at);
        }
        Ok(())
    }

    /// Restores `spans` in the text in `root` and books it as the JS
    /// identity path does: the holder, then the pending pairs, then the
    /// revived pieces unless `unregister` is false, then the live diff.
    fn restore(
        root: &mut CrdtRoot,
        spans: &[RestoreSpan<TextValue>],
        at: TimeTicket,
        unregister: bool,
    ) -> Result<()> {
        let (result, holds_cancels) = {
            let text = root.text_by_created_at_mut(&a(1)).expect("the text at t");
            let result = text.restore(spans, &at, None)?;
            (result, text.has_cancelled_removals())
        };
        if holds_cancels {
            root.register_ledger_holder(&a(1));
        }
        for (child_id, child_size, removed_at) in result.pending_gc_pairs {
            root.register_gc_pair_by_id(child_id, child_size, removed_at);
        }
        if unregister {
            for node in &result.revived {
                root.unregister_gc_pair_by_id(&node.id_string(), node.data_size());
            }
        }
        root.acc(result.live_diff);
        Ok(())
    }

    fn tombstones(root: &CrdtRoot) -> Vec<String> {
        text_of(root)
            .nodes()
            .filter(|node| node.is_removed())
            .map(|node| node.value().content().to_owned())
            .collect()
    }

    fn tombstone_size(root: &CrdtRoot) -> DataSize {
        let mut size = DataSize::default();
        for node in text_of(root).nodes().filter(|node| node.is_removed()) {
            size.data += node.data_size().data;
            size.meta += node.data_size().meta;
        }
        size
    }

    /// The running gc size and garbage count match a root rebuilt from the
    /// same content.
    fn assert_matches_rebuilt(root: &CrdtRoot, name: &str) {
        let rebuilt = CrdtRoot::new(root.object().deepcopy());
        assert_eq!(
            rebuilt.doc_size().gc,
            root.doc_size().gc,
            "{name}: gc vs rebuilt"
        );
        assert_eq!(
            rebuilt.get_garbage_len(),
            root.get_garbage_len(),
            "{name}: garbage vs rebuilt"
        );
    }

    /// JS `text_undo_removal_ledger_test.ts` `keeps a piece until its
    /// cancelled removals are known everywhere`, at the CRDT level (Task 6
    /// ports it end to end): A and B remove `b` concurrently and A undoes.
    /// The tombstone stays while A's cancel is unknown somewhere, and goes
    /// once it is known and B's removal alone holds it. The holder comes
    /// from the snapshot load (JS `registerInternalGCPairs`).
    #[test]
    fn holds_a_text_tombstone_until_its_cancel_is_known_everywhere() -> Result<()> {
        let mut text = seeded("abc")?;
        let range = text.index_range_to_pos_range(1, 2)?;
        text.edit_by_range(range.clone(), "", None, a(3), None)?;
        text.edit_by_range(range, "", None, b(4), Some(&vv([(A1, 2), (A2, 4)])))?;
        text.restore(&[span(a(2), 1, 2, "b")], &a(5), None)?;
        let mut root = root_with(text);
        assert_eq!("ac", text_of(&root).to_string());
        assert_eq!(1, root.get_garbage_len());

        // B's removal is stable, A's cancel is not.
        assert_eq!(0, root.garbage_collect(&vv([(A1, 4), (A2, 4)])));
        assert_eq!(vec!["b"], tombstones(&root));
        assert_eq!(1, root.get_garbage_len());

        assert_eq!(1, root.garbage_collect(&vv([(A1, 5), (A2, 4)])));
        assert!(tombstones(&root).is_empty());
        assert_eq!(0, root.get_garbage_len());
        assert_eq!("ac", text_of(&root).to_string());
        assert_matches_rebuilt(&root, "collected");
        Ok(())
    }

    /// JS `CRDTRoot.garbageCollect` prunes the registered ledgers first: a
    /// revived piece drops its removal and cancel once every replica knows
    /// the cancel, and writes no removal events from then on.
    #[test]
    fn prunes_the_ledger_of_a_revived_piece_once_its_cancel_is_known() -> Result<()> {
        let mut text = seeded("abc")?;
        text.edit_by_index(1, 2, "", None, a(3), None)?;
        text.restore(&[span(a(2), 1, 2, "b")], &a(4), None)?;
        let mut root = root_with(text);
        let events = |root: &CrdtRoot| {
            text_of(root)
                .nodes()
                .find(|node| node.id().offset() == 1)
                .expect("the revived piece")
                .removal_events()
        };
        assert_eq!(vec![a(3), a(4)], events(&root));

        root.garbage_collect(&vv([(A1, 3)]));
        assert_eq!(vec![a(3), a(4)], events(&root));

        root.garbage_collect(&vv([(A1, 4)]));
        assert!(events(&root).is_empty());
        assert!(!text_of(&root).has_cancelled_removals());
        Ok(())
    }

    /// JS `CRDTRoot.registerElement` books a Text whose ledgers hold
    /// cancels as a ledger holder (`registerInternalGCPairs`), whichever
    /// route brings it in: here a set, as an undo re-set or a remote payload
    /// does. Without the holder the ledger is never pruned and the piece
    /// keeps writing removal events JS has dropped.
    #[test]
    fn prunes_the_ledger_of_a_text_set_into_the_document() -> Result<()> {
        let mut text = seeded("abc")?;
        text.edit_by_index(1, 2, "", None, a(3), None)?;
        text.restore(&[span(a(2), 1, 2, "b")], &a(4), None)?;
        let mut root = CrdtRoot::create();
        root.set_object_member(&TimeTicket::initial(), "t", CrdtElement::text(text), a(5))?;
        let events = |root: &CrdtRoot| {
            text_of(root)
                .nodes()
                .find(|node| node.id().offset() == 1)
                .expect("the revived piece")
                .removal_events()
        };
        assert_eq!(vec![a(3), a(4)], events(&root));

        root.garbage_collect(&vv([(A1, 5)]));
        assert!(events(&root).is_empty());
        assert!(!text_of(&root).has_cancelled_removals());
        Ok(())
    }

    /// The pair toggle takes out the registered size, not the node's
    /// current one as JS does, so a revived Text node that was not
    /// unregistered must not be registered again (Task 3 review).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "re-registered without being unregistered")]
    fn refuses_to_toggle_a_revived_text_node_left_registered() {
        let mut root = root_with(seeded("abc").unwrap());
        let b_only = range(&root, 1, 2).unwrap();
        remove(&mut root, b_only, a(3), None).unwrap();
        restore(&mut root, &[span(a(2), 1, 2, "b")], a(4), false).unwrap();
        let b_again = range(&root, 1, 2).unwrap();
        let _ = remove(&mut root, b_again, a(5), None);
    }

    /// A tombstone two uncancelled removals hold is never collected (JS
    /// `heldFromPurge`).
    #[test]
    fn never_collects_a_tombstone_two_removals_hold() -> Result<()> {
        let mut text = seeded("abc")?;
        let range = text.index_range_to_pos_range(1, 2)?;
        text.edit_by_range(range.clone(), "", None, a(3), None)?;
        text.edit_by_range(range, "", None, b(4), Some(&vv([(A1, 2), (A2, 4)])))?;
        let mut root = root_with(text);

        assert_eq!(
            0,
            root.garbage_collect(&vv([(A1, i64::MAX), (A2, i64::MAX)]))
        );
        assert_eq!(vec!["b"], tombstones(&root));
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(tombstone_size(&root), root.doc_size().gc);
        Ok(())
    }

    /// Restoring part of a deleted node splits its registered tombstone. JS
    /// reads each pair's size when it collects, so the original's pair gives
    /// back the size of what is left of it, not the size it was registered
    /// with. Before, between and after collections, the gc size and garbage
    /// count match a root rebuilt from the same content. `X`, inserted by B
    /// after `f`, holds `ef` back on a barrier B's vector does not cover.
    #[test]
    fn matches_a_rebuild_after_a_partial_revive() -> Result<()> {
        let mut text = seeded("abcdef")?;
        text.edit_by_index(6, 6, "X", None, b(3), None)?;
        let mut root = root_with(text);
        let all = range(&root, 0, 6)?;
        remove(&mut root, all, a(4), None)?;
        assert_matches_rebuilt(&root, "removed");

        restore(&mut root, &[span(a(2), 2, 4, "cd")], a(5), true)?;
        assert_eq!("cdX", text_of(&root).to_string());
        assert_eq!(vec!["ab", "ef"], tombstones(&root));
        assert_matches_rebuilt(&root, "revived");

        root.garbage_collect(&vv([(A1, 10)]));
        assert_eq!(vec!["ef"], tombstones(&root));
        assert_eq!(tombstone_size(&root), root.doc_size().gc);
        assert_matches_rebuilt(&root, "ab collected");

        root.garbage_collect(&vv([(A1, 10), (A2, 10)]));
        assert!(tombstones(&root).is_empty());
        assert_eq!(DataSize::default(), root.doc_size().gc);
        assert_matches_rebuilt(&root, "all collected");
        assert!(!text_of(&root).has_cancelled_removals());
        Ok(())
    }

    /// JS reads `removedAt` when it collects. A piece split off a tombstone
    /// is registered with the removal it was born with; B's later removal
    /// overwrites it, and A's undo then leaves B's removal alone holding the
    /// piece. It goes only once B's removal is stable, not A's.
    #[test]
    fn collects_a_split_off_piece_by_its_current_removal() -> Result<()> {
        let mut root = root_with(seeded("abcdef")?);
        let all = range(&root, 0, 6)?;
        let ef = range(&root, 4, 6)?;
        remove(&mut root, all, a(3), None)?;
        remove(&mut root, ef, b(10), Some(&vv([(A1, 2), (A2, 10)])))?;
        restore(&mut root, &[span(a(2), 0, 6, "abcdef")], a(11), true)?;
        assert_eq!("abcd", text_of(&root).to_string());
        assert_eq!(vec!["ef"], tombstones(&root));
        assert_matches_rebuilt(&root, "revived");

        assert_eq!(0, root.garbage_collect(&vv([(A1, 11), (A2, 9)])));
        assert_eq!(vec!["ef"], tombstones(&root));
        assert_matches_rebuilt(&root, "unstable");

        assert_eq!(1, root.garbage_collect(&vv([(A1, 11), (A2, 10)])));
        assert!(tombstones(&root).is_empty());
        assert_eq!(DataSize::default(), root.doc_size().gc);
        assert_matches_rebuilt(&root, "collected");
        Ok(())
    }

    /// JS `CRDTRoot.collect` drops the pair of a piece that is live again
    /// but still registered, moving its size out of gc as
    /// `unregisterGCPair` does, and counts nothing for it.
    #[test]
    fn drops_the_pair_of_a_revived_piece_left_registered() -> Result<()> {
        let mut root = root_with(seeded("abcdef")?);
        let all = range(&root, 0, 6)?;
        remove(&mut root, all, a(3), None)?;
        restore(&mut root, &[span(a(2), 2, 4, "cd")], a(4), false)?;
        assert_eq!("cd", text_of(&root).to_string());
        assert_eq!(3, root.get_garbage_len());

        assert_eq!(0, root.garbage_collect(&vv([(A1, 1)])));
        assert_eq!(vec!["ab", "ef"], tombstones(&root));
        assert_matches_rebuilt(&root, "dropped");
        Ok(())
    }

    /// A pair whose text was purged is dropped by the size its node has
    /// when the text goes, as JS purges it through the detached text: here
    /// the original's pair, registered with the whole node, holds only `ab`
    /// after a partial revive. `k`, removed by an unstable B change, stays
    /// in gc to show what is left; its content keeps an over-subtraction
    /// from saturating to the right answer.
    #[test]
    fn drops_the_pairs_of_a_purged_text_by_their_current_size() -> Result<()> {
        let mut kept = CrdtText::create(b(2));
        kept.edit_by_index(0, 0, "kept", None, b(3), None)?;
        let mut root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [
                ("t", CrdtElement::text(seeded("abcdef")?)),
                ("k", CrdtElement::text(kept)),
            ],
        ));
        let all = range(&root, 0, 6)?;
        remove(&mut root, all, a(3), None)?;
        restore(&mut root, &[span(a(2), 2, 4, "cd")], a(4), true)?;
        root.remove_object_member(&TimeTicket::initial(), &a(1), a(20))?;
        root.remove_object_member(&TimeTicket::initial(), &b(2), b(30))?;
        assert_matches_rebuilt(&root, "removed");

        root.garbage_collect(&vv([(A1, 20), (A2, 5)]));
        assert!(root.text_by_created_at(&a(1)).is_none());
        assert_eq!(1, root.get_garbage_len());
        assert_matches_rebuilt(&root, "text collected");
        Ok(())
    }
}
