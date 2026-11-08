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

fn collect(d: &mut Document, vector: &VersionVector) -> Result<usize> {
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
    collect(d, &max_vector_of(&[A1, A2]))?;
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
    assert_eq!(4, doc.garbage_collect(&max_vector_of(&[A1]))?);
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
    collect(&mut mover, &vv)?;

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

    collect(&mut mover, &vv)?;
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

    collect(&mut mover, &vv)?;
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
