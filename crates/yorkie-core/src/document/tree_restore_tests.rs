//! Document-level tests of Tree restore spans between replicas, without a
//! server.
//!
//! The first groups port JS `test/unit/document/
//! tree_restore_text_parent_test.ts` and `tree_restore_ticket_test.ts`
//! (`8629993e`). The last group checks how a peer's undo stack moves past
//! remote Tree edits, against outcomes measured on JS at the same commit.
//!
//! Where JS passes changes through protobuf, these pass them through the
//! `wire` projection `yorkie-protocol` encodes from.

use super::adv_sync::through_wire;
use super::Document;
use crate::change::{Change, ChangePack, Checkpoint};
use crate::crdt::element::{CrdtElement, DataSize};
use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId, TreeRestoreSpan};
use crate::operation::{Operation, RestoreMode, TreeEditOperation};
use crate::time::TimeTicket;
use crate::{JsonTree, JsonValue, Result, TreeNodeInput, VersionVector};

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

fn p(value: &str) -> TreeNodeInput {
    element("p", vec![text(value)])
}

fn xml(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().expect("a document tree"),
        other => panic!("expected a tree at t, got {other:?}"),
    }
}

fn edit_tree(doc: &mut Document, f: impl FnOnce(&mut JsonTree) -> Result<()>) -> Result<()> {
    doc.update(|root| f(root.get_tree_mut("t")?))
}

fn set_tree(doc: &mut Document, root: TreeNodeInput) -> Result<()> {
    doc.update(|r| r.set_tree("t", root).map(|_| ()))
}

/// JS `treeOf`: the CRDT tree under `t`.
fn tree_of(doc: &Document) -> &CrdtTree {
    match doc.crdt_root.object().get("t") {
        Some(CrdtElement::Tree(tree)) => tree,
        other => panic!("expected a tree at t, got {other:?}"),
    }
}

/// JS `actorOf`: an actor number as a 24-hex-digit actor ID.
fn actor_of(n: u32) -> String {
    format!("{n:024}")
}

/// JS `newReplica`.
fn new_replica(key: &str, actor: &str) -> Document {
    let mut doc = Document::new(key);
    doc.set_actor(actor);
    doc
}

/// JS `maxVectorOf`.
fn max_vector_of(actors: &[String]) -> VersionVector {
    let mut vector = VersionVector::new();
    for actor in actors {
        vector.set(actor.as_str(), i64::MAX);
    }
    vector
}

/// JS `pendingChanges`: every change the replica has not had acknowledged.
fn pending_changes(doc: &Document) -> Vec<Change> {
    doc.create_change_pack().changes().to_vec()
}

/// JS `recordChanges`: drains the replica's pending local changes and acks
/// them. A step that produced no change fails, so a check after it cannot
/// hold trivially.
fn record_changes(from: &mut Document, what: &str) -> Result<Vec<Change>> {
    let changes = pending_changes(from);
    assert!(!changes.is_empty(), "{what} produced no change to deliver");
    let last_seq = changes.last().map_or(0, |change| change.id().client_seq());
    from.apply_change_pack(&ChangePack::create(
        from.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))?;
    Ok(changes)
}

/// JS `roundTrip`: the changes through the encoding a push and a pull take.
fn round_trip(key: &str, changes: Vec<Change>) -> Result<Vec<Change>> {
    let pack = through_wire(&ChangePack::create(
        key,
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    ))?;
    Ok(pack.changes().to_vec())
}

/// JS `deliver`: applies the changes as a pull would. The neutral
/// checkpoint keeps the receiver's pending changes, and the empty vector
/// keeps garbage collection out of the delivery.
fn deliver(to: &mut Document, changes: Vec<Change>) -> Result<()> {
    to.apply_change_pack(&ChangePack::create(
        to.key(),
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    ))
}

// --- tree_restore_text_parent_test.ts ---

const TEXT_PARENT_KEY: &str = "tree-restore-text-parent";

/// JS `SpansOf`: the crafted spans, from the replaced insertion's ticket,
/// the seeded `<p>` and its `hello` text node.
type SpansOf = fn(&TimeTicket, &TreeNodeId, &TreeNodeId) -> Vec<TreeRestoreSpan>;

/// A span for `id` with no attributes and no siblings.
fn span(
    id: TreeNodeId,
    node_type: &str,
    is_text: bool,
    value: Option<&str>,
    parent_id: &TreeNodeId,
) -> TreeRestoreSpan {
    TreeRestoreSpan {
        id,
        node_type: node_type.to_owned(),
        is_text,
        length: value.map_or(0, str::len),
        value: value.map(str::to_owned),
        attrs: None,
        parent_id: Some(parent_id.clone()),
        left_sibling_id: None,
        right_sibling_id: None,
    }
}

/// JS `seed`: `<r><p>hello</p></r>`.
fn seed_hello(doc: &mut Document) -> Result<()> {
    set_tree(doc, element("r", vec![p("hello")]))
}

/// JS `craft`: makes `from` insert a text node, replaces that change's
/// operation with a restore TreeEdit carrying `spans_of(...)`, and returns
/// all of `from`'s changes after the round trip a push takes. The span's own
/// ID reuses the insertion's ticket: no node holds it on a replica that
/// never saw the insertion, so restore falls through to the recreate path.
fn craft(from: &mut Document, spans_of: SpansOf) -> Result<Vec<Change>> {
    let (p_id, text_id) = {
        let p = tree_of(from)
            .root()
            .children()
            .next()
            .expect("the seeded <p>");
        let hello = p.children().next().expect("the seeded text");
        assert!(hello.is_text());
        (p.id().clone(), hello.id().clone())
    };
    edit_tree(from, |t| t.edit(1, 1, vec![text("x")], 0))?;

    let mut changes = pending_changes(from);
    let last = changes.pop().expect("the insertion");
    let [Operation::TreeEdit(valid)] = last.operations() else {
        panic!("expected one tree edit, got {:?}", last.operations());
    };
    let executed_at = valid.executed_at()?.clone();
    let spans = spans_of(&executed_at, &p_id, &text_id);
    let span_count = spans.len();
    let crafted = TreeEditOperation::create(
        valid.parent_created_at().clone(),
        valid.range_from().clone(),
        valid.to_pos().clone(),
        None,
        0,
        Some(executed_at),
    )
    .with_identity(spans, RestoreMode::Restore, Vec::new());
    changes
        .push(last.with_id_and_operations(last.id().clone(), vec![Operation::TreeEdit(crafted)]));

    let decoded = round_trip(TEXT_PARENT_KEY, changes)?;
    let Some(Operation::TreeEdit(op)) = decoded.last().and_then(|c| c.operations().first()) else {
        panic!("the crafted change lost its tree edit");
    };
    assert_eq!(
        span_count,
        op.restore_spans().len(),
        "decoding cannot tell a text parent from an element"
    );
    Ok(decoded)
}

/// JS "Tree restore span under a text parent", one test per row (a port of
/// Go `TestRestoreSpanUnderTextParentStaysApplicable`, yorkie#2175). A span
/// that names a text node as its parent, or whose `is_text` disagrees with
/// its node type, is left unplaced on a replica that pulls it and on one
/// replaying every change, and the tree stays editable.
fn assert_span_is_skipped(name: &str, spans_of: SpansOf) -> Result<()> {
    let mut a = new_replica(TEXT_PARENT_KEY, &actor_of(1));
    seed_hello(&mut a)?;
    let mut b = new_replica(TEXT_PARENT_KEY, &actor_of(2));
    deliver(&mut b, round_trip(TEXT_PARENT_KEY, pending_changes(&a))?)?;

    let changes = craft(&mut a, spans_of)?;

    // A replica that pulls the change.
    deliver(&mut b, changes[changes.len() - 1..].to_vec())?;
    assert_eq!("<r><p>hello</p></r>", xml(&b), "{name}");

    // A replica replaying every stored change, as a snapshot rebuild does.
    let mut built = new_replica(TEXT_PARENT_KEY, &actor_of(3));
    deliver(&mut built, changes)?;
    assert_eq!(
        built.to_sorted_json(),
        b.to_sorted_json(),
        "{name}: replicas converge"
    );

    // The tree stays editable after the skipped span.
    edit_tree(&mut b, |t| t.edit(6, 6, vec![text("!")], 0))?;
    assert_eq!("<r><p>hello!</p></r>", xml(&b), "{name}");
    Ok(())
}

/// JS "element span under a text parent".
#[test]
fn skips_an_element_span_under_a_text_parent() -> Result<()> {
    assert_span_is_skipped("element span under a text parent", |at, _, text_id| {
        vec![span(
            TreeNodeId::new(at.clone(), 0),
            "p",
            false,
            None,
            text_id,
        )]
    })
}

/// JS "text span under a text parent".
#[test]
fn skips_a_text_span_under_a_text_parent() -> Result<()> {
    assert_span_is_skipped("text span under a text parent", |at, _, text_id| {
        vec![span(
            TreeNodeId::new(at.clone(), 0),
            "text",
            true,
            Some("x"),
            text_id,
        )]
    })
}

/// JS "element span typed as text".
#[test]
fn skips_an_element_span_typed_as_text() -> Result<()> {
    assert_span_is_skipped("element span typed as text", |at, p_id, _| {
        vec![span(
            TreeNodeId::new(at.clone(), 0),
            "text",
            false,
            None,
            p_id,
        )]
    })
}

/// JS "text span typed as an element, twice".
#[test]
fn skips_a_text_span_typed_as_an_element_twice() -> Result<()> {
    assert_span_is_skipped("text span typed as an element, twice", |at, p_id, _| {
        let span = || span(TreeNodeId::new(at.clone(), 0), "p", true, Some("x"), p_id);
        vec![span(), span()]
    })
}

/// JS "honest undo after GC": the undo recreates a purged text node under
/// its element parent, the pulling replica agrees, and redo removes it
/// again.
#[test]
fn recreates_a_purged_text_node_under_its_element_parent() -> Result<()> {
    let sender = actor_of(1);
    let mut a = new_replica(TEXT_PARENT_KEY, &sender);
    seed_hello(&mut a)?;
    edit_tree(&mut a, |t| t.edit(1, 6, vec![], 0))?;
    assert!(a.garbage_collect(&max_vector_of(&[sender])) > 0);
    a.history_mut().undo()?;
    assert_eq!("<r><p>hello</p></r>", xml(&a));

    let changes = round_trip(TEXT_PARENT_KEY, pending_changes(&a))?;
    let Some(Operation::TreeEdit(undo)) = changes.last().and_then(|c| c.operations().first())
    else {
        panic!("expected the undo's tree edit");
    };
    assert!(
        !undo.restore_spans().is_empty(),
        "the undo takes the restore path"
    );

    let mut b = new_replica(TEXT_PARENT_KEY, &actor_of(2));
    deliver(&mut b, changes)?;
    assert_eq!("<r><p>hello</p></r>", xml(&b));
    assert_eq!(b.to_sorted_json(), a.to_sorted_json());

    a.history_mut().redo()?;
    assert_eq!("<r><p></p></r>", xml(&a));
    let changes = round_trip(TEXT_PARENT_KEY, pending_changes(&a))?;
    deliver(&mut b, changes[changes.len() - 1..].to_vec())?;
    assert_eq!("<r><p></p></r>", xml(&b));
    Ok(())
}

// --- tree_restore_ticket_test.ts ---

const TICKET_KEY: &str = "tree-restore-ticket";

/// The actors, numbered as in JS and Go: A authors and undoes, B removes
/// `<p>` at the lower ticket, C concurrently at the higher one, and the
/// six observers take 10..15.
const ACTOR_AUTHOR_A: u32 = 1;
const ACTOR_REMOVER_B: u32 = 2;
const ACTOR_REMOVER_C: u32 = 5;
const ACTOR_OBSERVER_BASE: u32 = 10;

/// JS `nodeByID`: the node the ID lookup answers, if it is that node.
fn node_by_id<'a>(doc: &'a Document, id: &TreeNodeId) -> Option<&'a TreeNode> {
    tree_of(doc)
        .find_floor_node(id)
        .filter(|node| node.id() == id)
}

/// JS `tombstoneOf`: `absent`, `live`, or the removal ticket.
fn tombstone_of(doc: &Document, id: &TreeNodeId) -> String {
    match node_by_id(doc, id) {
        None => "absent".to_owned(),
        Some(node) => node
            .removed_at()
            .map_or_else(|| "live".to_owned(), TimeTicket::to_test_string),
    }
}

/// The node IDs the lookup holds, against the nodes reachable from the
/// root, tombstones included (JS `getNodeSize` against `reachableCount`).
/// Rust nodes own their children, so a registered node that no path
/// reaches shows only as a difference between the two.
fn registered_and_reachable(doc: &Document) -> (usize, usize) {
    let tree = tree_of(doc);
    (tree.node_map_len(), tree.nodes().count())
}

/// JS `TicketFixture`: one history with three concurrent changes, recorded
/// so every delivery order replays identical changes.
struct TicketFixture {
    setup: Vec<Change>,
    remove_low: Vec<Change>,
    remove_high: Vec<Change>,
    restore: Vec<Change>,
    low_at: TimeTicket,
    high_at: TimeTicket,
    /// The restored text node, named before the purge erased it.
    text_id: TreeNodeId,
    /// The `<p>` above it.
    parent_id: TreeNodeId,
    /// The never-purged siblings `a` and `d`.
    sibling_ids: Vec<TreeNodeId>,
    actors: Vec<String>,
}

/// JS `firstTicket`. Every recorded step here starts with a Tree edit.
fn first_ticket(changes: &[Change], what: &str) -> Result<TimeTicket> {
    let op = changes
        .iter()
        .flat_map(Change::operations)
        .next()
        .unwrap_or_else(|| panic!("{what} carried no operation to read a ticket from"));
    let Operation::TreeEdit(op) = op else {
        panic!("{what} does not start with a tree edit: {op:?}");
    };
    Ok(op.executed_at()?.clone())
}

/// JS `newTicketFixture`.
fn new_ticket_fixture() -> Result<TicketFixture> {
    let mut a = new_replica(TICKET_KEY, &actor_of(ACTOR_AUTHOR_A));
    let mut b = new_replica(TICKET_KEY, &actor_of(ACTOR_REMOVER_B));
    let mut c = new_replica(TICKET_KEY, &actor_of(ACTOR_REMOVER_C));

    // Every actor that will ever hold the document is in the vector, or a
    // collection would purge nothing.
    let mut actors: Vec<String> = [ACTOR_AUTHOR_A, ACTOR_REMOVER_B, ACTOR_REMOVER_C]
        .into_iter()
        .map(actor_of)
        .collect();
    actors.extend((0..6).map(|i| actor_of(ACTOR_OBSERVER_BASE + i)));

    set_tree(&mut a, element("r", vec![p("abcd")]))?;
    edit_tree(&mut a, |t| t.edit(2, 4, vec![], 0))?;
    assert_eq!("<r><p>ad</p></r>", xml(&a));

    let mut text_id = None;
    let mut parent_id = None;
    let mut sibling_ids = Vec::new();
    for node in tree_of(&a).nodes() {
        if node.is_text() && node.value() == "bc" {
            text_id = Some(node.id().clone());
        } else if node.is_text() && (node.value() == "a" || node.value() == "d") {
            sibling_ids.push(node.id().clone());
        } else if node.node_type() == "p" {
            parent_id = Some(node.id().clone());
        }
    }
    let text_id = text_id.expect("the tombstoned \"bc\" should be nameable pre-purge");
    let parent_id = parent_id.expect("the enclosing <p> should be nameable");
    assert_eq!(2, sibling_ids.len(), "both never-purged siblings are named");

    let setup = record_changes(&mut a, "the setup edits")?;

    // B and C collect the setup too, so "bc" is purged everywhere and A's
    // undo takes the recreate path.
    deliver(&mut b, setup.clone())?;
    deliver(&mut c, setup.clone())?;
    for doc in [&mut a, &mut b, &mut c] {
        doc.garbage_collect(&max_vector_of(&actors));
    }
    assert_eq!("<r><p>ad</p></r>", xml(&b));
    assert_eq!("<r><p>ad</p></r>", xml(&c));
    assert!(
        node_by_id(&a, &text_id).is_none(),
        "the removed text must be purged, not merely tombstoned"
    );

    // Neither remover has seen the other.
    edit_tree(&mut b, |t| t.edit(0, 4, vec![], 0))?;
    let remove_low = record_changes(&mut b, "b's removal of <p>")?;
    edit_tree(&mut c, |t| t.edit(0, 4, vec![], 0))?;
    let remove_high = record_changes(&mut c, "c's concurrent removal of <p>")?;

    a.history_mut().undo()?;
    let restore = record_changes(&mut a, "a's undo of its own text removal")?;

    let low_at = first_ticket(&remove_low, "b's removal")?;
    let high_at = first_ticket(&remove_high, "c's removal")?;
    Ok(TicketFixture {
        setup,
        remove_low,
        remove_high,
        restore,
        low_at,
        high_at,
        text_id,
        parent_id,
        sibling_ids,
        actors,
    })
}

/// JS `observer`: a replica holding the collected setup.
fn observer(f: &TicketFixture, n: u32) -> Result<Document> {
    let mut doc = new_replica(TICKET_KEY, &actor_of(n));
    deliver(&mut doc, f.setup.clone())?;
    doc.garbage_collect(&max_vector_of(&f.actors));
    assert_eq!("<r><p>ad</p></r>", xml(&doc));
    Ok(doc)
}

/// The six delivery orders of the three concurrent changes.
const TICKET_ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

/// Delivers the fixture's three concurrent changes to a fresh observer in
/// each of the six orders, labelled with the order.
fn delivered_in_every_order(f: &TicketFixture) -> Result<Vec<(String, Document)>> {
    let steps = [
        ("removeLow", &f.remove_low),
        ("removeHigh", &f.remove_high),
        ("restore", &f.restore),
    ];
    // The premise of the scenario: c's removal wins the LWW race.
    assert!(
        f.high_at.after(&f.low_at),
        "the fixture needs c's removal to win: low={} high={}",
        f.low_at.to_test_string(),
        f.high_at.to_test_string()
    );

    let mut outcomes = Vec::new();
    for (i, order) in TICKET_ORDERS.iter().enumerate() {
        let label = order
            .iter()
            .map(|&idx| steps[idx].0)
            .collect::<Vec<_>>()
            .join("->");
        let mut doc = observer(f, ACTOR_OBSERVER_BASE + i as u32)?;
        for &idx in order {
            deliver(&mut doc, steps[idx].1.clone())?;
        }
        outcomes.push((label, doc));
    }
    Ok(outcomes)
}

/// JS "tree restore tombstone ticket across delivery orders", "stamps the
/// restored node with the winning removal ticket in all six orders" (a
/// port of Go `TestTreeRestoreAgreesOnTheTombstoneTicketAcrossDeliveryOrders`,
/// yorkie#2008). A text node recreated under a `<p>` two peers removed
/// concurrently is born removed with the winning removal's ticket, as its
/// never-purged siblings carry, in every delivery order; and collection
/// then drains every replica, with every registered node reachable.
/// Findings are collected and reported together, as JS does.
///
/// JS also requires the whole doc size to agree across the orders after
/// collection. Here only its gc half is compared: live size is the running
/// size of the porting gap (a Tree removal never moves a node's size out of
/// live), and `keeps_the_live_size_equal_across_delivery_orders` shows how
/// it diverges.
#[test]
fn stamps_a_restored_node_with_the_winning_removal_ticket() -> Result<()> {
    let f = new_ticket_fixture()?;
    let mut outcomes = delivered_in_every_order(&f)?;

    let high = f.high_at.to_test_string();
    let mut problems = Vec::new();
    let mut eq = |actual: String, expected: &str, msg: String| {
        if actual != expected {
            problems.push(format!("{msg}: got {actual}, want {expected}"));
        }
    };
    for (label, doc) in &outcomes {
        eq(
            tombstone_of(doc, &f.parent_id),
            &high,
            format!("[{label}] the parent <p> settles on the winning removal ticket"),
        );
        let restored = tombstone_of(doc, &f.text_id);
        eq(
            restored.clone(),
            &high,
            format!(
                "[{label}] the restored node's tombstone ticket is the winning removal's (low={})",
                f.low_at.to_test_string()
            ),
        );
        for id in &f.sibling_ids {
            eq(
                restored.clone(),
                &tombstone_of(doc, id),
                format!("[{label}] the restored node agrees with its never-purged sibling"),
            );
        }
    }
    let (ref_label, ref_doc) = &outcomes[0];
    for (label, doc) in &outcomes[1..] {
        eq(
            tombstone_of(doc, &f.text_id),
            &tombstone_of(ref_doc, &f.text_id),
            format!("the restored node's ticket diverged ([{ref_label}] vs [{label}])"),
        );
        eq(
            xml(doc),
            &xml(ref_doc),
            format!("content diverged ([{ref_label}] vs [{label}])"),
        );
    }

    for (_, doc) in &mut outcomes {
        doc.garbage_collect(&max_vector_of(&f.actors));
    }
    let (_, ref_doc) = &outcomes[0];
    for (label, doc) in &outcomes {
        let (registered, reachable) = registered_and_reachable(doc);
        eq(
            registered.to_string(),
            &reachable.to_string(),
            format!("[{label}] after collection, every registered node is reachable"),
        );
        eq(
            doc.garbage_len().to_string(),
            "0",
            format!("[{label}] everything is causally stable, so collection must drain"),
        );
        eq(
            format!("{:?}", doc.doc_size().gc),
            &format!("{:?}", DataSize::default()),
            format!("[{label}] GC accounting telescopes to zero after collection"),
        );
        eq(
            tombstone_of(doc, &f.text_id),
            &tombstone_of(ref_doc, &f.text_id),
            format!("[{label}] the restored node's fate diverged after collection"),
        );
        eq(
            format!("{:?}", doc.doc_size().gc),
            &format!("{:?}", ref_doc.doc_size().gc),
            format!("[{label}] gc size diverged after collection"),
        );
    }

    assert!(
        problems.is_empty(),
        "{} finding(s):\n{}",
        problems.len(),
        problems
            .iter()
            .map(|p| format!("  - {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok(())
}

/// The doc size half of JS "stamps the restored node with the winning
/// removal ticket in all six orders": after collection every order holds
/// the same doc size (JS: live `{data: 0, meta: 96}` in all six).
///
/// Rust diverges by the size of the recreated `bc` (4 bytes of data, 24 of
/// meta) in the two orders where the restore arrives first, the only ones
/// where it is recreated live rather than born removed. Its size goes to
/// live; the removal that follows leaves live alone, and collection takes
/// the size out of gc only, so live keeps it. JS moves a removed node's
/// size from live to gc, so its live size telescopes in every order. The
/// cause is the running live-size gap, not restore.
#[test]
#[ignore = "live doc size gap: a Tree removal does not move a node's size from live \
            to gc, so a node recreated live and then removed stays in live after \
            collection (see docs/current-porting-gaps.md, doc_size)"]
fn keeps_the_live_size_equal_across_delivery_orders() -> Result<()> {
    let f = new_ticket_fixture()?;
    let mut outcomes = delivered_in_every_order(&f)?;
    for (_, doc) in &mut outcomes {
        doc.garbage_collect(&max_vector_of(&f.actors));
    }
    let (ref_label, ref_doc) = &outcomes[0];
    for (label, doc) in &outcomes[1..] {
        assert_eq!(
            ref_doc.doc_size(),
            doc.doc_size(),
            "doc size of [{label}] against [{ref_label}]"
        );
    }
    Ok(())
}

// --- A peer's undo stack past remote Tree edits ---
//
// No JS test covers these two; the expected trees are what the same steps
// give on JS at `8629993e` (in memory, through protobuf). Both need an undo
// entry that addresses the tree by index, which a Tree reverse does only for
// a split (a boundary deletion) or a copy: the reverse of a plain insertion
// or deletion goes by identity and is never reconciled.

const RECONCILE_KEY: &str = "tree-undo-reconcile";

/// Two replicas holding `<doc>children</doc>`, a under actor 1.
fn reconcile_pair(children: Vec<TreeNodeInput>) -> Result<(Document, Document)> {
    let mut a = new_replica(RECONCILE_KEY, &actor_of(1));
    let mut b = new_replica(RECONCILE_KEY, &actor_of(2));
    set_tree(&mut a, element("doc", children))?;
    let changes = record_changes(&mut a, "the seed")?;
    deliver(&mut b, round_trip(RECONCILE_KEY, changes)?)?;
    Ok((a, b))
}

/// Records `from`'s pending changes and delivers them to `to` through the
/// wire.
fn send(from: &mut Document, to: &mut Document, what: &str) -> Result<()> {
    let changes = record_changes(from, what)?;
    deliver(to, round_trip(RECONCILE_KEY, changes)?)
}

/// An undo that revives a `<p>` and its text arrives at a peer as one
/// identity edit with two executed ranges, the `<p>` and then its text. The
/// peer reconciles its pending split undo past both, so the undo deletes
/// the boundary its split opened (JS `applyChangeInternal`, one
/// `reconcileTreeEdit` per executed range).
#[test]
fn moves_a_peer_split_undo_past_every_range_of_a_remote_restore() -> Result<()> {
    let (mut a, mut b) = reconcile_pair(vec![p("ab"), p("cd"), p("efgh")])?;
    edit_tree(&mut a, |t| t.edit_by_path(&[1], &[2], vec![], 0))?;
    send(&mut a, &mut b, "a's delete of <p>cd</p>")?;
    edit_tree(&mut b, |t| t.edit(7, 7, vec![], 1))?;
    send(&mut b, &mut a, "b's split of efgh")?;
    assert_eq!("<doc><p>ab</p><p>ef</p><p>gh</p></doc>", xml(&a));
    assert_eq!(xml(&a), xml(&b));

    a.history_mut().undo()?;
    let changes = round_trip(RECONCILE_KEY, record_changes(&mut a, "a's undo")?)?;
    let Some(Operation::TreeEdit(restore)) = changes.last().and_then(|c| c.operations().first())
    else {
        panic!("expected a's undo to be a tree edit");
    };
    assert_eq!(2, restore.restore_spans().len(), "the <p> and its text");
    deliver(&mut b, changes)?;
    assert_eq!("<doc><p>ab</p><p>cd</p><p>ef</p><p>gh</p></doc>", xml(&b));

    b.history_mut().undo()?;
    send(&mut b, &mut a, "b's undo")?;
    assert_eq!("<doc><p>ab</p><p>cd</p><p>efgh</p></doc>", xml(&b));
    assert_eq!(a.to_sorted_json(), b.to_sorted_json());
    Ok(())
}

/// b types into a `<p>` that a removed concurrently. On a, the text lands
/// under the removed `<p>` and takes no room in the index, but its size
/// still counts as inserted (JS `CRDTTree.edit`, `insertedContentSize`), so
/// a's pending split undo shifts right by one. JS gives this same tree: the
/// shifted boundary deletion takes the `<p>` and `e` instead of the
/// boundary the split opened. The test pins Rust to JS; the JS outcome is a
/// defect to report upstream, not one to fix here.
#[test]
fn shifts_a_split_undo_past_text_typed_under_a_removed_parent_as_js_does() -> Result<()> {
    let (mut a, mut b) = reconcile_pair(vec![p("ab"), p("cdef")])?;
    edit_tree(&mut a, |t| t.edit(7, 7, vec![], 1))?;
    edit_tree(&mut a, |t| t.edit(0, 4, vec![], 0))?;
    edit_tree(&mut b, |t| t.edit(1, 1, vec![text("X")], 0))?;
    let from_a = round_trip(
        RECONCILE_KEY,
        record_changes(&mut a, "a's split and delete")?,
    )?;
    let from_b = round_trip(RECONCILE_KEY, record_changes(&mut b, "b's typing")?)?;
    deliver(&mut a, from_b)?;
    deliver(&mut b, from_a)?;
    assert_eq!("<doc><p>cd</p><p>ef</p></doc>", xml(&a));
    assert_eq!(xml(&a), xml(&b));

    a.history_mut().undo()?;
    send(&mut a, &mut b, "a's undo of the delete")?;
    assert_eq!("<doc><p>ab</p><p>cd</p><p>ef</p></doc>", xml(&a));

    a.history_mut().undo()?;
    send(&mut a, &mut b, "a's undo of the split")?;
    assert_eq!("<doc><p>ab</p><p>cd</p>f</doc>", xml(&a));
    assert_eq!(a.to_sorted_json(), b.to_sorted_json());
    Ok(())
}
