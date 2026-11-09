use super::element::DataSize;
use super::splay::{NodeId as SplayNodeId, SplayTree, SplayValue};
use crate::time::{ticket_known, ActorId};
use crate::{Result, TimeTicket, VersionVector, YorkieError, TIME_TICKET_SIZE};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
struct SplitIndexValue {
    node_index: usize,
    len: usize,
}

impl SplitIndexValue {
    fn new(node_index: usize, len: usize) -> Self {
        Self { node_index, len }
    }
}

impl SplayValue for SplitIndexValue {
    fn len(&self) -> usize {
        self.len
    }

    fn to_test_string(&self) -> String {
        self.node_index.to_string()
    }
}

pub(crate) trait RgaTreeSplitValue: Clone + PartialEq {
    fn split(&mut self, offset: usize) -> Self;
    /// Returns a deep copy of `[from, to)` of this value (JS `substring`).
    fn substring(&self, from: usize, to: usize) -> Self;
    fn len(&self) -> usize;
    fn data_size(&self) -> DataSize;
    fn to_json(&self) -> String;
    fn to_test_string(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RgaTreeSplitNodeId {
    created_at: TimeTicket,
    offset: usize,
}

impl RgaTreeSplitNodeId {
    pub(crate) fn new(created_at: TimeTicket, offset: usize) -> Self {
        Self { created_at, offset }
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    pub(crate) fn split(&self, offset: usize) -> Self {
        Self::new(self.created_at.clone(), self.offset + offset)
    }

    pub(crate) fn has_same_created_at(&self, other: &Self) -> bool {
        self.created_at == other.created_at
    }

    pub(crate) fn to_id_string(&self) -> String {
        format!("{}:{}", self.created_at.to_id_string(), self.offset)
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!("{}:{}", self.created_at.to_test_string(), self.offset)
    }
}

impl Ord for RgaTreeSplitNodeId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.created_at
            .cmp(&other.created_at)
            .then_with(|| self.offset.cmp(&other.offset))
    }
}

impl PartialOrd for RgaTreeSplitNodeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RgaTreeSplitPos {
    id: RgaTreeSplitNodeId,
    relative_offset: usize,
}

impl RgaTreeSplitPos {
    pub(crate) fn new(id: RgaTreeSplitNodeId, relative_offset: usize) -> Self {
        Self {
            id,
            relative_offset,
        }
    }

    pub(crate) fn id(&self) -> &RgaTreeSplitNodeId {
        &self.id
    }

    pub(crate) fn relative_offset(&self) -> usize {
        self.relative_offset
    }

    pub(crate) fn absolute_id(&self) -> RgaTreeSplitNodeId {
        self.id.split(self.relative_offset)
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!("{}:{}", self.id.to_test_string(), self.relative_offset)
    }
}

pub(crate) type RgaTreeSplitPosRange = (RgaTreeSplitPos, RgaTreeSplitPos);

/// A change to the visible values made by `RgaTreeSplit::edit` (JS
/// `ValueChange`): the indexes `from..to` before the change were replaced by
/// `value`, or removed when `value` is `None`. JS also carries the editing
/// actor; no caller here reads it, so it is left out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ValueChange<V> {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) value: Option<V>,
}

/// A run of characters from a single original insertion: the absolute-offset
/// interval `[start, end)` of the insertion created at `created_at` (JS
/// `RestoreSpan`). `value` is a deep copy of the removed content, carried so
/// that purged nodes can be recreated.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RestoreSpan<V> {
    pub(crate) created_at: TimeTicket,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) value: V,
}

/// A GC pair as `CrdtRoot::register_gc_pair_by_id` takes it: the child's ID
/// string, the size to charge to gc, and the removal ticket.
pub(crate) type GcPairEntry = (String, DataSize, TimeTicket);

/// Result of `RgaTreeSplit::restore` (JS `restore`'s tuple).
#[derive(Debug, Clone)]
pub(crate) struct RestoreResult<V>
where
    V: RgaTreeSplitValue,
{
    /// Tombstoned pieces this restore took back to live.
    pub(crate) revived: Vec<RgaTreeSplitNode<V>>,
    /// Pieces this restore recreated because GC had purged them.
    pub(crate) recreated: Vec<RgaTreeSplitNode<V>>,
    /// The revived and recreated content as insertions, in ascending index
    /// order.
    pub(crate) changes: Vec<ValueChange<V>>,
    /// The size of the recreated pieces, which no split accounted.
    pub(crate) live_diff: DataSize,
    /// The pieces split off tombstones while isolating the restored ranges,
    /// born removed. The caller registers them before it unregisters the
    /// revived pieces: a revived piece may be one of them.
    pub(crate) pending_gc_pairs: Vec<GcPairEntry>,
}

/// Result of `RgaTreeSplit::retombstone` (JS `retombstone`'s tuple).
#[derive(Debug, Clone)]
pub(crate) struct RetombstoneResult<V>
where
    V: RgaTreeSplitValue,
{
    /// The pieces this redo took from live to removed, then the pieces split
    /// off tombstones, born removed.
    pub(crate) gc_pairs: Vec<GcPairEntry>,
    /// The removed regions as deletions, each index taken before its
    /// removal.
    pub(crate) changes: Vec<ValueChange<V>>,
    /// The size the splits of live pieces added; the caller accounts it
    /// before it registers `gc_pairs`.
    pub(crate) diff: DataSize,
    /// The pieces the removal was recorded on, which the reverse names.
    pub(crate) recorded: Vec<RestoreSpan<V>>,
}

/// The IDs of the nodes whose ledger holds a cancel, the only ones
/// `prune_ledgers` can drop anything from (JS `cancelled`). `None` until
/// first needed: a split decoded from a snapshot fills it with one scan.
///
/// It is a cache of what the nodes say, so it takes no part in equality:
/// two splits with the same nodes compare equal whether or not either has
/// scanned yet.
#[derive(Debug, Clone, Default)]
struct CancelledNodes(RefCell<Option<BTreeSet<RgaTreeSplitNodeId>>>);

impl PartialEq for CancelledNodes {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// An undo that cancelled one deletion of a node (JS `LedgerCancel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LedgerCancel {
    pub(crate) deletion: TimeTicket,
    pub(crate) cancelled_at: TimeTicket,
}

/// One deletion of a removal ledger beside the cancel that covers it, if an
/// undo cancelled it (JS `LedgerEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LedgerEntry {
    removal: TimeTicket,
    cancelled_at: Option<TimeTicket>,
}

/// The reading of a removal ledger every walk of it needs (JS `LedgerIndex`):
/// each deletion with its cancel, those entries by deletion, the deletions no
/// cancel covers, and the cancels by the deletion each one cancels -- which
/// covers the cancel of a deletion the ledger itself does not hold.
///
/// JS keeps this reading per node because rebuilding it on every walk makes
/// churning a ledger quadratic. Here it is kept eagerly and is always exactly
/// `LedgerIndex::build(dels, cancels)`, so two equal ledgers compare equal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LedgerIndex {
    entries: Vec<LedgerEntry>,
    by_key: BTreeMap<TimeTicket, usize>,
    live: Vec<TimeTicket>,
    cancelled: BTreeMap<TimeTicket, TimeTicket>,
}

impl LedgerIndex {
    fn build(dels: &[TimeTicket], cancels: &[LedgerCancel]) -> Self {
        let mut index = Self::default();
        for cancel in cancels {
            index
                .cancelled
                .insert(cancel.deletion.clone(), cancel.cancelled_at.clone());
        }
        for deletion in dels {
            index.push_removal(deletion.clone());
        }
        index
    }

    /// Indexes one more deletion, with the cancel already indexed for it.
    fn push_removal(&mut self, removal: TimeTicket) {
        if self.by_key.contains_key(&removal) {
            return;
        }
        let cancelled_at = self.cancelled.get(&removal).cloned();
        if cancelled_at.is_none() {
            self.live.push(removal.clone());
        }
        self.by_key.insert(removal.clone(), self.entries.len());
        self.entries.push(LedgerEntry {
            removal,
            cancelled_at,
        });
    }
}

/// The removal ledger of a node (JS `dels` and `cancels`): the deletions
/// that removed it while it was live in the deleter's view, and the
/// deletions undo cancelled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalLedger {
    dels: Vec<TimeTicket>,
    cancels: Vec<LedgerCancel>,
    index: LedgerIndex,
}

impl RemovalLedger {
    fn new(dels: Vec<TimeTicket>, cancels: Vec<LedgerCancel>) -> Self {
        let index = LedgerIndex::build(&dels, &cancels);
        Self {
            dels,
            cancels,
            index,
        }
    }
}

/// Result of `RgaTreeSplit::delete_nodes`: the value changes and the nodes
/// it removed, each with whether it was already removed before (a
/// concurrent removal it overwrote by LWW).
type DeleteResult<V> = (Vec<ValueChange<V>>, Vec<(RgaTreeSplitNode<V>, bool)>);

/// Result of `RgaTreeSplit::edit`.
#[derive(Debug, Clone)]
pub(crate) struct SplitEditResult<V>
where
    V: RgaTreeSplitValue,
{
    /// Where the caret sits after the edit.
    pub(crate) caret_pos: RgaTreeSplitPos,
    /// Nodes this edit took from live to removed, each needing a GC pair. A
    /// node whose `removed_at` it overwrote keeps the pair it already has.
    pub(crate) removed_nodes: Vec<RgaTreeSplitNode<V>>,
    pub(crate) diff: DataSize,
    /// One change per affected range, in the order JS reports them.
    pub(crate) changes: Vec<ValueChange<V>>,
    /// The values of every node this edit removed, in document order.
    pub(crate) removed_values: Vec<V>,
    /// One span per node this edit removed, in document order (JS
    /// `removedSpans`).
    pub(crate) removed_spans: Vec<RestoreSpan<V>>,
    /// The pieces the boundary splits cut off tombstones, born removed (JS
    /// `drainPendingGCPairs` in `edit`).
    pub(crate) split_gc_pairs: Vec<GcPairEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RgaTreeSplitNode<V>
where
    V: RgaTreeSplitValue,
{
    id: RgaTreeSplitNodeId,
    value: V,
    /// The latest uncancelled deletion.
    removed_at: Option<TimeTicket>,
    /// The removal ledger; `None` means the ledger is just `removed_at`.
    ledger: Option<Box<RemovalLedger>>,
    ins_prev_id: Option<RgaTreeSplitNodeId>,
    ins_next_id: Option<RgaTreeSplitNodeId>,
}

impl<V> RgaTreeSplitNode<V>
where
    V: RgaTreeSplitValue,
{
    pub(crate) fn new(id: RgaTreeSplitNodeId, value: V) -> Self {
        Self {
            id,
            value,
            removed_at: None,
            ledger: None,
            ins_prev_id: None,
            ins_next_id: None,
        }
    }

    pub(crate) fn id(&self) -> &RgaTreeSplitNodeId {
        &self.id
    }

    pub(crate) fn id_string(&self) -> String {
        self.id.to_id_string()
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        self.id.created_at()
    }

    pub(crate) fn value(&self) -> &V {
        &self.value
    }

    pub(crate) fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.removed_at.as_ref()
    }

    pub(crate) fn ins_prev_id(&self) -> Option<&RgaTreeSplitNodeId> {
        self.ins_prev_id.as_ref()
    }

    pub(crate) fn ins_next_id(&self) -> Option<&RgaTreeSplitNodeId> {
        self.ins_next_id.as_ref()
    }

    /// Sets the remove time of this node and drops the ledger (JS
    /// `setRemovedAt`).
    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.removed_at = removed_at;
        self.ledger = None;
    }

    pub(crate) fn set_ins_prev_id(&mut self, id: Option<RgaTreeSplitNodeId>) {
        self.ins_prev_id = id;
    }

    pub(crate) fn set_ins_next_id(&mut self, id: Option<RgaTreeSplitNodeId>) {
        self.ins_next_id = id;
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    pub(crate) fn len(&self) -> usize {
        if self.is_removed() {
            return 0;
        }

        self.content_len()
    }

    pub(crate) fn content_len(&self) -> usize {
        self.value.len()
    }

    pub(crate) fn create_pos_range(&self) -> RgaTreeSplitPosRange {
        (
            RgaTreeSplitPos::new(self.id.clone(), 0),
            RgaTreeSplitPos::new(self.id.clone(), self.len()),
        )
    }

    pub(crate) fn can_style(&self, edited_at: &TimeTicket, client_lamport_at_change: i64) -> bool {
        let node_existed = self.created_at().lamport() <= client_lamport_at_change;
        node_existed
            && self
                .removed_at
                .as_ref()
                .map(|removed_at| edited_at.after(removed_at))
                .unwrap_or(true)
    }

    pub(crate) fn data_size(&self) -> DataSize {
        let mut size = self.value.data_size();
        size.meta += TIME_TICKET_SIZE;

        if self.removed_at.is_some() {
            size.meta += TIME_TICKET_SIZE;
        }

        size
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{} {}",
            self.id.to_test_string(),
            self.value.to_test_string()
        )
    }

    /// Creates the split node of the given offset; it carries this node's
    /// `removed_at` and a copy of its ledger (JS `split`, `copyLedger`).
    fn split(&mut self, offset: usize) -> Self {
        let mut node = Self::new(self.id.split(offset), self.value.split(offset));
        node.removed_at = self.removed_at.clone();
        node.ledger = self.ledger.clone();
        node
    }

    /// Returns the removal ledger: whether there is one, the deletions and
    /// the cancels (JS `getLedger`).
    pub(crate) fn ledger(&self) -> (bool, &[TimeTicket], &[LedgerCancel]) {
        match &self.ledger {
            Some(ledger) => (true, &ledger.dels, &ledger.cancels),
            None => (false, &[], &[]),
        }
    }

    /// Tells whether the ledger holds a cancel, the only thing
    /// `prune_stable` can drop (JS `hasCancels`).
    pub(crate) fn has_cancels(&self) -> bool {
        self.ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.cancels.is_empty())
    }

    /// The deletions no cancel covers. A node without a ledger was removed by
    /// `removed_at` alone, if at all (JS `ledgerIndex().live`, `removals`).
    fn live_removals(&self) -> &[TimeTicket] {
        match &self.ledger {
            Some(ledger) => &ledger.index.live,
            None => self.removed_at.as_slice(),
        }
    }

    /// Builds the ledger from `removed_at` if there is none yet (JS
    /// `ledger`).
    fn ensure_ledger(&mut self) -> &mut RemovalLedger {
        let removed_at = &self.removed_at;
        self.ledger.get_or_insert_with(|| {
            Box::new(RemovalLedger::new(
                removed_at.iter().cloned().collect(),
                Vec::new(),
            ))
        })
    }

    /// Sets `removed_at` to the latest uncancelled deletion (JS `recompute`).
    fn recompute(&mut self) {
        let mut latest: Option<&TimeTicket> = None;
        for deletion in self.live_removals() {
            if latest.is_none_or(|latest| deletion.after(latest)) {
                latest = Some(deletion);
            }
        }
        self.removed_at = latest.cloned();
    }

    /// Adds a deletion to the ledger (JS `recordRemoval`). A node's first
    /// removal is just `removed_at`; the ledger is built only when a second
    /// removal or a cancel needs it.
    pub(crate) fn record_removal(&mut self, removed_at: TimeTicket) {
        if self.ledger.is_none() && self.removed_at.is_none() {
            self.removed_at = Some(removed_at);
            return;
        }

        let ledger = self.ensure_ledger();
        // An actor removes a piece only while it is live in its view, so it
        // never holds two uncancelled removals; a change whose vector leaves
        // out its own earlier removal is not honest, and recording it would
        // break the pairing `removal_events` relies on and grow the ledger.
        let mine = ledger
            .index
            .live
            .iter()
            .find(|deletion| deletion.actor_id() == removed_at.actor_id())
            .cloned();
        match mine {
            Some(mine) if mine != removed_at => return,
            Some(_) => {}
            None => {
                if !ledger.index.by_key.contains_key(&removed_at) {
                    ledger.dels.push(removed_at.clone());
                    ledger.index.push_removal(removed_at);
                }
            }
        }
        self.recompute();
    }

    /// Cancels one deletion (an undo of it). Returns whether the node is live
    /// afterwards (JS `cancelRemoval`).
    pub(crate) fn cancel_removal(&mut self, deletion: &TimeTicket, at: TimeTicket) -> bool {
        let ledger = self.ensure_ledger();
        if !ledger.index.cancelled.contains_key(deletion) {
            ledger.cancels.push(LedgerCancel {
                deletion: deletion.clone(),
                cancelled_at: at.clone(),
            });
            ledger.index.cancelled.insert(deletion.clone(), at.clone());
            if let Some(&entry) = ledger.index.by_key.get(deletion) {
                ledger.index.entries[entry].cancelled_at = Some(at);
                if let Some(live) = ledger.index.live.iter().position(|d| d == deletion) {
                    ledger.index.live.remove(live);
                }
            }
        }
        self.recompute();
        self.removed_at.is_none()
    }

    /// Drops each removal whose cancel is known to every replica, with that
    /// cancel (JS `pruneStable`). A change that arrives later knows both, so
    /// the pair never decides `is_removed_in_view` again, and dropping it
    /// keeps an undo-heavy text from carrying every removal it ever had. A
    /// removal is older than its cancel by the same actor, so the cancel
    /// being known is enough. What is left is the default ledger once
    /// `removed_at` alone says it.
    pub(crate) fn prune_stable(&mut self, vector: &VersionVector) {
        let Some(ledger) = self.ledger.as_mut() else {
            return;
        };

        let mut cancels = Vec::new();
        let mut dropped = BTreeSet::new();
        for cancel in std::mem::take(&mut ledger.cancels) {
            if !vector.after_or_equal(&cancel.cancelled_at) {
                cancels.push(cancel);
                continue;
            }
            dropped.insert(cancel.deletion);
        }
        let mut dels = std::mem::take(&mut ledger.dels);
        if !dropped.is_empty() {
            dels.retain(|deletion| !dropped.contains(deletion));
        }

        if cancels.is_empty()
            && (dels.is_empty() || (dels.len() == 1 && self.removed_at.as_ref() == Some(&dels[0])))
        {
            self.ledger = None;
            return;
        }
        **ledger = RemovalLedger::new(dels, cancels);
    }

    /// Returns the removal by `actor` no undo cancelled, if any (JS
    /// `uncancelledRemovalBy`).
    pub(crate) fn uncancelled_removal_by(&self, actor: &ActorId) -> Option<&TimeTicket> {
        self.live_removals()
            .iter()
            .find(|deletion| deletion.actor_id() == actor)
    }

    /// Tells whether GC must keep this tombstone under `vector`, the versions
    /// every replica has seen (JS `heldFromPurge`). A node removed by two or
    /// more deletions no undo cancelled stays: an undo of one of them must
    /// find it still held by the others, and a purged node could not say so.
    /// A node whose ledger is not yet stable stays too: every deletion and
    /// every cancel in it must be known to all replicas, so any change that
    /// arrives after the purge already knew them, and a node a restore
    /// recreates with only the removal it cancels answers
    /// `is_removed_in_view` as the full ledger would. Once that holds and a
    /// single removal is left, a restore that finds the piece gone can only
    /// be cancelling that removal, and recreates it.
    pub(crate) fn held_from_purge(&self, vector: &VersionVector) -> bool {
        let Some(ledger) = &self.ledger else {
            return false;
        };

        // A cancelled deletion is older than its cancel by the same actor, so
        // a stable cancel makes it stable too; an uncancelled one is
        // `removed_at`, which GC checks first, or one of two that keep the
        // node anyway.
        if ledger
            .cancels
            .iter()
            .any(|cancel| !vector.after_or_equal(&cancel.cancelled_at))
        {
            return true;
        }
        ledger.index.live.len() >= 2
    }

    /// Tells whether the node was removed in the view of `vector`: a deletion
    /// it knew that no undo it knew cancelled (JS `isRemovedInView`).
    pub(crate) fn is_removed_in_view(&self, vector: &VersionVector) -> bool {
        let known = |ticket: &TimeTicket| ticket_known(Some(vector), ticket);
        match &self.ledger {
            Some(ledger) => ledger.index.entries.iter().any(|entry| {
                known(&entry.removal) && !entry.cancelled_at.as_ref().is_some_and(known)
            }),
            None => self.removed_at.as_ref().is_some_and(known),
        }
    }

    /// Returns the ledger as one list: every removal and every cancel's
    /// ticket except `removed_at`, in ticket order (JS `getRemovalEvents`).
    /// Empty when `removed_at` says it all. An actor's events alternate in
    /// ticket order -- it removes a piece only while the piece is live in its
    /// view, and its undo cancels its one uncancelled removal -- so
    /// `set_removal_events` can tell them apart by place.
    pub(crate) fn removal_events(&self) -> Vec<TimeTicket> {
        let Some(ledger) = &self.ledger else {
            return Vec::new();
        };

        let mut events = ledger
            .dels
            .iter()
            .filter(|deletion| self.removed_at.as_ref() != Some(*deletion))
            .chain(ledger.cancels.iter().map(|cancel| &cancel.cancelled_at))
            .cloned()
            .collect::<Vec<_>>();
        events.sort();
        events
    }

    /// Rebuilds the ledger from `removed_at` and the events `removal_events`
    /// returned: per actor, in ticket order, a removal then the cancel of it
    /// (JS `setRemovalEvents`). It fails on a repeated ticket, which would
    /// pair a cancel with a removal that is not older, and when the result
    /// does not reproduce `removed_at`, and leaves the node as it was.
    pub(crate) fn set_removal_events(&mut self, events: Vec<TimeTicket>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let removed_at = self.removed_at.clone();
        let mut all = events;
        all.extend(removed_at.iter().cloned());
        all.sort();
        if all.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(YorkieError::InvalidArgument(
                "removal events repeat a ticket".to_owned(),
            ));
        }

        let mut dels = Vec::new();
        let mut cancels = Vec::new();
        let mut open: BTreeMap<ActorId, TimeTicket> = BTreeMap::new();
        for ticket in all {
            if let Some(deletion) = open.remove(ticket.actor_id()) {
                cancels.push(LedgerCancel {
                    deletion,
                    cancelled_at: ticket,
                });
                continue;
            }
            dels.push(ticket.clone());
            open.insert(ticket.actor_id().clone(), ticket);
        }

        self.ledger = Some(Box::new(RemovalLedger::new(dels, cancels)));
        self.recompute();
        if self.removed_at != removed_at {
            self.removed_at = removed_at;
            self.ledger = None;
            return Err(YorkieError::InvalidArgument(
                "removal events do not reproduce removed_at".to_owned(),
            ));
        }
        Ok(())
    }

    /// Checks if this node can be removed by a deletion at `edited_at` (JS
    /// `canRemove`).
    pub(crate) fn can_remove(
        &self,
        edited_at: &TimeTicket,
        creation_known: bool,
        tombstone_known: bool,
    ) -> bool {
        // Skip if the node's creation was not visible to this operation.
        if !creation_known {
            return false;
        }

        let Some(removed_at) = &self.removed_at else {
            return true;
        };

        // Overwrite only if the prior tombstone was not known (concurrent or
        // unseen) and newer. This enables LWW for concurrent deletions.
        !tombstone_known && edited_at.after(removed_at)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RgaTreeSplit<V>
where
    V: RgaTreeSplitValue,
{
    nodes: Vec<RgaTreeSplitNode<V>>,
    tree_by_index: RefCell<SplayTree<SplitIndexValue>>,
    tree_by_id: BTreeMap<RgaTreeSplitNodeId, usize>,
    splay_node_by_id: BTreeMap<RgaTreeSplitNodeId, SplayNodeId>,
    /// GC pairs for garbage no removal produced: pieces split off a removed
    /// node, born removed (JS `pendingGCPairs`). Every caller that splits
    /// drains it into the pairs it returns.
    pending_gc_pairs: Vec<GcPairEntry>,
    cancelled: CancelledNodes,
}

impl<V> RgaTreeSplit<V>
where
    V: RgaTreeSplitValue,
{
    pub(crate) fn new(initial_head: RgaTreeSplitNode<V>) -> Self {
        let mut split = Self {
            nodes: vec![initial_head],
            tree_by_index: RefCell::new(SplayTree::new()),
            tree_by_id: BTreeMap::new(),
            splay_node_by_id: BTreeMap::new(),
            pending_gc_pairs: Vec::new(),
            cancelled: CancelledNodes::default(),
        };
        split.rebuild_indexes();
        split
    }

    pub(crate) fn initial_head(&self) -> &RgaTreeSplitNode<V> {
        &self.nodes[0]
    }

    pub(crate) fn len(&self) -> usize {
        self.tree_by_index.borrow().len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn create_range(&self, from: usize, to: usize) -> Result<RgaTreeSplitPosRange> {
        if from > to {
            return Err(YorkieError::InvalidTextPosition(
                "from must be less than or equal to to".to_owned(),
            ));
        }

        let from_pos = self.find_node_pos(from)?;
        let to_pos = if from == to {
            from_pos.clone()
        } else {
            self.find_node_pos(to)?
        };

        Ok((from_pos, to_pos))
    }

    pub(crate) fn index_to_pos(&self, index: usize) -> Result<RgaTreeSplitPos> {
        self.find_node_pos(index)
    }

    pub(crate) fn find_indexes_from_range(
        &self,
        range: &RgaTreeSplitPosRange,
    ) -> Result<(usize, usize)> {
        Ok((
            self.pos_to_index(&range.0, false)?,
            self.pos_to_index(&range.1, true)?,
        ))
    }

    pub(crate) fn normalize_pos(&self, pos: &RgaTreeSplitPos) -> Result<RgaTreeSplitPos> {
        let mut index = self.find_floor_node(pos.id()).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                pos.id().to_test_string()
            ))
        })?;

        let mut total = pos.relative_offset();
        while index > 0 {
            index -= 1;
            total += self.nodes[index].len();
        }

        Ok(RgaTreeSplitPos::new(self.nodes[index].id().clone(), total))
    }

    pub(crate) fn refine_pos(&self, pos: &RgaTreeSplitPos) -> Result<RgaTreeSplitPos> {
        let mut index = self.find_floor_node(pos.id()).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                pos.id().to_test_string()
            ))
        })?;
        let mut offset_in_part = pos.relative_offset();
        let mut part_len = self.nodes[index].content_len();

        while offset_in_part > part_len {
            offset_in_part -= part_len;
            let Some(next_index) = self.next_index(index) else {
                return Ok(RgaTreeSplitPos::new(
                    self.nodes[index].id().clone(),
                    part_len,
                ));
            };

            index = next_index;
            part_len = self.nodes[index].len();
        }

        Ok(RgaTreeSplitPos::new(
            self.nodes[index].id().clone(),
            offset_in_part,
        ))
    }

    pub(crate) fn edit(
        &mut self,
        range: RgaTreeSplitPosRange,
        edited_at: TimeTicket,
        value: Option<V>,
        version_vector: Option<&VersionVector>,
    ) -> Result<SplitEditResult<V>> {
        let mut diff = DataSize::default();

        let (to_left, diff_to, to_right) = self.find_node_with_split(&range.1, &edited_at)?;
        let to_left_id = self.nodes[to_left].id().clone();
        let to_right_id = to_right.map(|index| self.nodes[index].id().clone());
        let (from_left, diff_from, from_right) = self.find_node_with_split(&range.0, &edited_at)?;
        let to_left = self.find_node_index(&to_left_id).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                to_left_id.to_test_string()
            ))
        })?;
        let to_right = to_right_id
            .as_ref()
            .and_then(|to_right_id| self.find_node_index(to_right_id));

        add_data_size(&mut diff, diff_to);
        add_data_size(&mut diff, diff_from);

        let nodes_to_delete = self.find_between(from_right, to_right);
        let (mut changes, removed_nodes) =
            self.delete_nodes(nodes_to_delete, edited_at.clone(), version_vector)?;

        let caret_id = to_right
            .and_then(|index| self.nodes.get(index))
            .map(|node| node.id().clone())
            .unwrap_or_else(|| self.nodes[to_left].id().clone());
        let mut caret_pos = RgaTreeSplitPos::new(caret_id, 0);

        if let Some(value) = value.filter(|value| value.len() > 0) {
            let index = self.pos_to_index(&self.nodes[from_left].create_pos_range().1, true)?;
            match changes.last_mut() {
                Some(last) if last.from == index => last.value = Some(value.clone()),
                _ => changes.push(ValueChange {
                    from: index,
                    to: index,
                    value: Some(value.clone()),
                }),
            }

            let inserted = self.insert_after_index(
                from_left,
                RgaTreeSplitNode::new(RgaTreeSplitNodeId::new(edited_at, 0), value),
            );
            add_data_size(&mut diff, self.nodes[inserted].data_size());
            caret_pos = RgaTreeSplitPos::new(
                self.nodes[inserted].id().clone(),
                self.nodes[inserted].content_len(),
            );
        }

        // 04. Report the removed nodes. A node that was already removed keeps
        // its GC pair, but its value and span are reported all the same.
        let mut newly_removed = Vec::new();
        let mut removed_values = Vec::new();
        let mut removed_spans = Vec::new();
        for (node, already_removed) in removed_nodes {
            removed_values.push(node.value().clone());
            // The span names the piece by its insertion, so later splits of
            // the tombstone do not change what it covers; the value is a deep
            // copy for the same reason.
            removed_spans.push(RestoreSpan {
                created_at: node.created_at().clone(),
                start: node.id().offset(),
                end: node.id().offset() + node.content_len(),
                value: node.value().clone(),
            });
            if !already_removed {
                newly_removed.push(node);
            }
        }

        Ok(SplitEditResult {
            caret_pos,
            removed_nodes: newly_removed,
            diff,
            changes,
            removed_values,
            removed_spans,
            split_gc_pairs: self.drain_pending_gc_pairs(),
        })
    }

    /// Re-establishes the characters `spans` describe under their original
    /// identities (JS `restore`). Per region of each span:
    ///   - a live piece is skipped (another undo restored it);
    ///   - a tombstoned piece is isolated and the undoing actor's own removal
    ///     cancelled; it revives unless another removal holds it;
    ///   - a gap (GC purged the piece) is recreated with the original ID.
    ///
    /// The caller must, in order, register `pending_gc_pairs`, unregister
    /// the revived pieces, and account `live_diff` (see `RestoreResult`).
    pub(crate) fn restore(
        &mut self,
        spans: &[RestoreSpan<V>],
        executed_at: &TimeTicket,
        fallback_anchor: Option<&RgaTreeSplitPos>,
    ) -> Result<RestoreResult<V>> {
        let mut revived = Vec::new();
        let mut recreated = Vec::new();
        let mut live_diff = DataSize::default();

        // The last piece placed at the cursor (revived, skipped or
        // recreated), in document order. A recreated fragment with no
        // surviving piece of its insertion to anchor it is placed after this,
        // so a multi-fragment run is rebuilt left to right rather than each
        // fragment landing at the same fallback, reversed. Spans arrive in
        // document order, so this is always the fragment's left neighbour.
        let mut chain_anchor: Option<RgaTreeSplitNodeId> = None;
        for span in spans {
            let pieces = self.find_pieces_overlapping(&span.created_at, span.start, span.end);

            let mut cursor = span.start;
            let mut piece_index = 0;
            while cursor < span.end {
                let piece = pieces.get(piece_index).map(|id| {
                    let index = self.index_of(id);
                    let start = self.nodes[index].id().offset();
                    (index, start, start + self.nodes[index].content_len())
                });

                match piece {
                    Some((index, piece_start, piece_end)) if piece_start <= cursor => {
                        // Covered by an existing piece.
                        let overlap_end = piece_end.min(span.end);
                        if self.nodes[index].is_removed() {
                            let (target, _) = self.isolate_range(index, cursor, overlap_end)?;
                            // An undo reverses its own actor's change, so the
                            // removal it cancels is the one uncancelled
                            // removal by that actor: one actor cannot remove
                            // a piece already removed in its view. No other
                            // removal holds it if there is none.
                            let node = &mut self.nodes[target];
                            let mut live = false;
                            if let Some(deletion) =
                                node.uncancelled_removal_by(executed_at.actor_id()).cloned()
                            {
                                live = node.cancel_removal(&deletion, executed_at.clone());
                                let id = node.id().clone();
                                self.track_cancelled(id);
                            }
                            let target_id = self.nodes[target].id().clone();
                            if live {
                                // The piece's length went from 0 to its
                                // content length.
                                self.rebuild_indexes();
                                revived.push(target_id.clone());
                            }
                            chain_anchor = Some(target_id);
                        } else {
                            chain_anchor = Some(self.nodes[index].id().clone());
                        }
                        cursor = overlap_end;
                        if overlap_end >= piece_end {
                            piece_index += 1;
                        }
                    }
                    _ => {
                        let gap_end = piece.map_or(span.end, |(_, start, _)| start.min(span.end));
                        let (id, size) = self.recreate(
                            span,
                            cursor,
                            gap_end,
                            executed_at,
                            fallback_anchor,
                            chain_anchor.as_ref(),
                        );
                        add_data_size(&mut live_diff, size);
                        recreated.push(id.clone());
                        chain_anchor = Some(id);
                        cursor = gap_end;
                    }
                }
            }
        }

        let pending_gc_pairs = self.drain_pending_gc_pairs();

        // Revived content is live now; report each piece as an insertion at
        // its final index. Ascending order keeps the indexes valid when the
        // changes are applied in sequence.
        let revived = revived
            .iter()
            .map(|id| self.nodes[self.index_of(id)].clone())
            .collect::<Vec<_>>();
        let recreated = recreated
            .iter()
            .map(|id| self.nodes[self.index_of(id)].clone())
            .collect::<Vec<_>>();
        let mut changes = Vec::new();
        for node in revived.iter().chain(&recreated) {
            let (from, _) = self.find_indexes_from_range(&node.create_pos_range())?;
            changes.push(ValueChange {
                from,
                to: from,
                value: Some(node.value().clone()),
            });
        }
        changes.sort_by_key(|change| change.from);

        Ok(RestoreResult {
            revived,
            recreated,
            changes,
            live_diff,
            pending_gc_pairs,
        })
    }

    /// Recreates `[start, end)` of `span`, a region GC purged, with its
    /// original ID, and returns that ID and the size of the recreated piece
    /// (the gap branch of JS `restore`).
    fn recreate(
        &mut self,
        span: &RestoreSpan<V>,
        start: usize,
        end: usize,
        executed_at: &TimeTicket,
        fallback_anchor: Option<&RgaTreeSplitPos>,
        chain_anchor: Option<&RgaTreeSplitNodeId>,
    ) -> (RgaTreeSplitNodeId, DataSize) {
        let value = span.value.substring(start - span.start, end - span.start);
        let id = RgaTreeSplitNodeId::new(span.created_at.clone(), start);
        let mut node = RgaTreeSplitNode::new(id.clone(), value);

        // A purged piece had a single uncancelled removal, known to every
        // replica before the purge (see `held_from_purge`), so this restore
        // can only be cancelling that one, and every change that can arrive
        // later knows it. A stand-in removal by the same actor at lamport 0,
        // which every such change's vector covers, keeps that answer for
        // `is_removed_in_view`, with this cancel. Being cancelled, it is never
        // `removed_at`. The pair is dropped once the cancel is known
        // everywhere (`prune_stable`); until then snapshots carry it.
        let stand_in = TimeTicket::new(0, 0, executed_at.actor_id().clone());
        node.record_removal(stand_in.clone());
        node.cancel_removal(&stand_in, executed_at.clone());
        // JS also books the attribute tombstones the copy duplicated
        // (`bookCopiedAttrTombstones`). Here the root keys GC pairs by ID
        // string alone and the copy carries the original's attribute IDs, so
        // booking them would toggle the original's pairs off; like a split's
        // copies, they stay uncollected (see the porting gaps).
        let size = node.data_size();

        let prev = self.find_restore_anchor(
            &span.created_at,
            start,
            end,
            executed_at,
            fallback_anchor,
            chain_anchor,
        );
        self.insert_after_index(prev, node);
        self.track_cancelled(id.clone());

        // The physical insert leaves the insertion chain (`ins_prev_id` /
        // `ins_next_id`) as it was. Relink it, as a split does, so a later
        // edit whose boundary lands on this piece does not resolve through a
        // stale link in `find_floor_node_prefer_to_left` (yorkie-js-sdk#1327).
        // The neighbours are the nearest surviving pieces, not just the ones
        // at the adjacent offsets: with the piece before the gap purged too,
        // the one before that would keep pointing past this piece, and
        // purging it later would hand the piece after the gap an
        // `ins_prev_id` that skips it.
        if start > 0 {
            let prev_id = RgaTreeSplitNodeId::new(span.created_at.clone(), start - 1);
            if let Some((ins_prev, _)) = self.floor_entry(&prev_id) {
                self.link_insertions(&ins_prev, &id);
            }
        }
        let next_id = RgaTreeSplitNodeId::new(span.created_at.clone(), end);
        if let Some((ins_next, _)) = self.ceiling_entry(&next_id) {
            self.link_insertions(&id, &ins_next);
        }
        (id, size)
    }

    /// Makes `next` follow `prev` in the insertion chain (JS `setInsNext`,
    /// `setInsPrev`).
    fn link_insertions(&mut self, prev: &RgaTreeSplitNodeId, next: &RgaTreeSplitNodeId) {
        let prev_index = self.index_of(prev);
        let next_index = self.index_of(next);
        self.nodes[prev_index].ins_next_id = Some(next.clone());
        self.nodes[next_index].ins_prev_id = Some(prev.clone());
    }

    /// Re-deletes the characters `spans` describe, a redo of an identity
    /// undo (JS `retombstone`). Only pieces live in the redo's view are
    /// removed: a piece removed in that view, or purged, is skipped, and a
    /// piece removed here by a deletion the redo never saw records the redo
    /// without being re-tombstoned. The caller accounts `diff` before it
    /// registers `gc_pairs`.
    pub(crate) fn retombstone(
        &mut self,
        spans: &[RestoreSpan<V>],
        executed_at: &TimeTicket,
        vector: Option<&VersionVector>,
    ) -> Result<RetombstoneResult<V>> {
        let mut gc_pairs = Vec::new();
        let mut changes = Vec::new();
        let mut diff = DataSize::default();
        // The pieces the removal was recorded on, which the reverse names: a
        // restore never reaches a piece that holds no removal by its actor.
        let mut recorded = Vec::new();
        let remote = vector.filter(|vector| !vector.is_empty());

        for span in spans {
            for piece in self.find_pieces_overlapping(&span.created_at, span.start, span.end) {
                let index = self.index_of(&piece);
                let piece_start = piece.offset();
                let piece_end = piece_start + self.nodes[index].content_len();
                let from = piece_start.max(span.start);
                let to = piece_end.min(span.end);

                if self.nodes[index].is_removed() {
                    // A piece live in the redo's view but removed here by a
                    // deletion the redo never saw still records the redo's
                    // deletion, so a later undo of that other deletion
                    // keeps it removed.
                    if remote.is_some_and(|vector| !self.nodes[index].is_removed_in_view(vector)) {
                        let (target, _) = self.isolate_range(index, from, to)?;
                        self.nodes[target].record_removal(executed_at.clone());
                        recorded.push(recorded_span(span, &self.nodes[target]));
                    }
                    continue;
                }
                // A piece live here but removed in the redo's view (by a
                // deletion whose undo the redo never saw) is left alone, as
                // the redoing replica left it: like a deletion, a redo
                // removes only what was live in its view.
                if remote.is_some_and(|vector| self.nodes[index].is_removed_in_view(vector)) {
                    continue;
                }

                let (target, split_diff) = self.isolate_range(index, from, to)?;
                // The piece was live, so the split overhead belongs to the
                // live diff, as an edit's boundary splits do.
                add_data_size(&mut diff, split_diff);
                // The visible range, taken while the target is still live.
                let (change_from, change_to) =
                    self.find_indexes_from_range(&self.nodes[target].create_pos_range())?;
                let node = &mut self.nodes[target];
                node.record_removal(executed_at.clone());
                recorded.push(recorded_span(span, node));
                if let Some(removed_at) = node.removed_at() {
                    gc_pairs.push((node.id_string(), node.data_size(), removed_at.clone()));
                }
                self.rebuild_indexes();
                if change_from < change_to {
                    changes.push(ValueChange {
                        from: change_from,
                        to: change_to,
                        value: None,
                    });
                }
            }
        }

        // Isolating a range inside a tombstone buffers born-removed
        // remainders. Hand them over with the rest: unregistered, they would
        // never be purged.
        gc_pairs.extend(self.drain_pending_gc_pairs());

        Ok(RetombstoneResult {
            gc_pairs,
            changes,
            diff,
            recorded,
        })
    }

    /// Re-snapshots `spans` against the pieces they address as they stand
    /// now (JS `refreshSpans`): same insertions, same intervals, with the
    /// current attributes. A span is captured when the edit it reverses
    /// runs, so a style applied to the run afterwards -- which `can_style`
    /// admits even on a tombstone -- is missing from it, and a replica that
    /// purged the run would recreate it with the stale snapshot while one
    /// that kept the tombstone revives the live attributes.
    ///
    /// A region a surviving piece (live or removed) covers is taken from that
    /// piece, so a span crossing several pieces becomes one span per piece; a
    /// region with no surviving piece keeps the snapshot. Read-only.
    pub(crate) fn refresh_spans(&self, spans: &[RestoreSpan<V>]) -> Vec<RestoreSpan<V>> {
        let mut refreshed = Vec::new();
        let from_snapshot = |span: &RestoreSpan<V>, start: usize, end: usize| RestoreSpan {
            created_at: span.created_at.clone(),
            start,
            end,
            value: span.value.substring(start - span.start, end - span.start),
        };

        for span in spans {
            let mut cursor = span.start;
            for piece in self.find_pieces_overlapping(&span.created_at, span.start, span.end) {
                let node = &self.nodes[self.index_of(&piece)];
                let piece_start = piece.offset();
                let start = piece_start.max(cursor);
                let end = (piece_start + node.content_len()).min(span.end);
                if end <= start {
                    continue;
                }
                if start > cursor {
                    refreshed.push(from_snapshot(span, cursor, start));
                }
                refreshed.push(RestoreSpan {
                    created_at: span.created_at.clone(),
                    start,
                    end,
                    value: node
                        .value()
                        .substring(start - piece_start, end - piece_start),
                });
                cursor = end;
            }
            if cursor < span.end {
                refreshed.push(from_snapshot(span, cursor, span.end));
            }
        }

        refreshed
    }

    /// Tells whether any node's ledger holds a cancel (JS
    /// `hasCancelledRemovals`).
    pub(crate) fn has_cancelled_removals(&self) -> bool {
        self.with_cancelled(|cancelled| !cancelled.is_empty())
    }

    /// Drops from every ledger the removals whose cancels `vector` covers
    /// (see `RgaTreeSplitNode::prune_stable`), and returns how many nodes
    /// still hold a cancel (JS `pruneLedgers`).
    pub(crate) fn prune_ledgers(&mut self, vector: &VersionVector) -> usize {
        let mut held = BTreeSet::new();
        let ids = self.with_cancelled(|cancelled| cancelled.iter().cloned().collect::<Vec<_>>());
        for id in ids {
            let Some(index) = self.find_node_index(&id) else {
                continue;
            };
            let node = &mut self.nodes[index];
            node.prune_stable(vector);
            if node.has_cancels() {
                held.insert(id);
            }
        }
        let count = held.len();
        *self.cancelled.0.borrow_mut() = Some(held);
        count
    }

    /// Returns the GC pairs buffered by splits of removed nodes and clears
    /// the buffer (JS `drainPendingGCPairs`).
    pub(crate) fn drain_pending_gc_pairs(&mut self) -> Vec<GcPairEntry> {
        std::mem::take(&mut self.pending_gc_pairs)
    }

    /// Runs `f` on the IDs of the nodes holding a cancel, scanning the nodes
    /// the first time (JS `scanCancelled`).
    fn with_cancelled<R>(&self, f: impl FnOnce(&mut BTreeSet<RgaTreeSplitNodeId>) -> R) -> R {
        let mut cancelled = self.cancelled.0.borrow_mut();
        f(cancelled.get_or_insert_with(|| {
            self.nodes
                .iter()
                .filter(|node| node.has_cancels())
                .map(|node| node.id().clone())
                .collect()
        }))
    }

    /// Records that the ledger of the node `id` holds a cancel (JS
    /// `trackCancelled`).
    fn track_cancelled(&mut self, id: RgaTreeSplitNodeId) {
        self.with_cancelled(|cancelled| cancelled.insert(id));
    }

    /// Returns the nodes of the insertion `created_at`, live or removed, that
    /// overlap the absolute offsets `[start, end)`, in ascending offset order
    /// (JS `findPiecesOverlapping`).
    fn find_pieces_overlapping(
        &self,
        created_at: &TimeTicket,
        start: usize,
        end: usize,
    ) -> Vec<RgaTreeSplitNodeId> {
        let mut pieces = Vec::new();
        let mut probe = end.checked_sub(1);
        while let Some(offset) = probe {
            let key = RgaTreeSplitNodeId::new(created_at.clone(), offset);
            let Some((id, index)) = self.floor_entry(&key) else {
                break;
            };
            let node_start = id.offset();
            let node_end = node_start + self.nodes[index].content_len();
            if node_end <= start {
                break;
            }
            if node_start < end && node_end > start {
                pieces.push(id);
            }
            if node_start <= start {
                break;
            }
            probe = node_start.checked_sub(1);
        }
        pieces.reverse();
        pieces
    }

    /// Returns the node of the insertion `created_at` whose offsets cover
    /// `offset`, if any (JS `findPieceCovering`).
    fn find_piece_covering(&self, created_at: &TimeTicket, offset: usize) -> Option<usize> {
        let key = RgaTreeSplitNodeId::new(created_at.clone(), offset);
        let (id, index) = self.floor_entry(&key)?;
        let node_start = id.offset();
        (node_start <= offset && offset < node_start + self.nodes[index].content_len())
            .then_some(index)
    }

    /// Returns the index of the node to insert a recreated fragment
    /// `[gap_start, gap_end)` of the insertion `created_at` after (JS
    /// `findRestoreAnchor`). Every rule keys on what the operation carries
    /// and on ID lookups:
    ///  (a) a piece covering `gap_end`: directly before it, the original
    ///      slot;
    ///  (b) the nearest surviving piece of the insertion before `gap_start`:
    ///      directly after it;
    ///  (c) the nearest surviving piece of the insertion at or after
    ///      `gap_end`: directly before it;
    ///  (d) the chain anchor, the fragment this restore placed just before:
    ///      after it, so a purged run is rebuilt left to right;
    ///  (e) the operation's fallback anchor, refined;
    ///  (f) the head.
    fn find_restore_anchor(
        &mut self,
        created_at: &TimeTicket,
        gap_start: usize,
        gap_end: usize,
        executed_at: &TimeTicket,
        fallback_anchor: Option<&RgaTreeSplitPos>,
        chain_anchor: Option<&RgaTreeSplitNodeId>,
    ) -> usize {
        if let Some(succ) = self.find_piece_covering(created_at, gap_end) {
            return succ - 1;
        }

        if gap_start > 0 {
            let key = RgaTreeSplitNodeId::new(created_at.clone(), gap_start - 1);
            if let Some((_, index)) = self.floor_entry(&key) {
                return index;
            }
        }

        // The nearest surviving piece of the insertion after the gap, not the
        // rightmost one: anchoring before a later piece would place the gap
        // after the pieces in between, and a deletion that later spans from
        // the gap to one of them would run from its start to the end of the
        // text.
        let after = RgaTreeSplitNodeId::new(created_at.clone(), gap_end);
        if let Some((_, index)) = self.ceiling_entry(&after) {
            return index - 1;
        }

        if let Some(chain_anchor) = chain_anchor {
            return self.index_of(chain_anchor);
        }

        // As in JS and Go, the size a split of a live node adds here is not
        // reported; a split of a removed node is buffered as usual.
        if let Some(fallback_anchor) = fallback_anchor {
            let left = self.refine_pos(fallback_anchor).and_then(|refined| {
                self.find_node_with_split(&refined, executed_at)
                    .map(|(left, _, _)| left)
            });
            if let Ok(left) = left {
                return left;
            }
        }

        0
    }

    /// Splits the node at `index` so that a node covering exactly the
    /// absolute offsets `[from, to)` exists, and returns its index and the
    /// size the splits added (JS `isolateRange`). A split of a removed node
    /// buffers its born-removed piece and adds nothing to the diff.
    ///
    /// Requires `node start <= from < to <= node end`.
    fn isolate_range(&mut self, index: usize, from: usize, to: usize) -> Result<(usize, DataSize)> {
        let mut diff = DataSize::default();
        let mut node = index;
        let node_start = self.nodes[node].id().offset();
        if from > node_start {
            add_data_size(&mut diff, self.split_node(node, from - node_start)?);
            node += 1;
        }
        let new_start = self.nodes[node].id().offset();
        if to < new_start + self.nodes[node].content_len() {
            add_data_size(&mut diff, self.split_node(node, to - new_start)?);
        }
        Ok((node, diff))
    }

    /// The greatest node ID at or below `key` with the same insertion (JS
    /// `treeByID.floorEntry` checked with `hasSameCreatedAt`).
    fn floor_entry(&self, key: &RgaTreeSplitNodeId) -> Option<(RgaTreeSplitNodeId, usize)> {
        let (id, index) = self.tree_by_id.range(..=key.clone()).next_back()?;
        id.has_same_created_at(key).then(|| (id.clone(), *index))
    }

    /// The least node ID at or above `key` with the same insertion (JS
    /// `treeByID.ceilingEntry` checked with `hasSameCreatedAt`).
    fn ceiling_entry(&self, key: &RgaTreeSplitNodeId) -> Option<(RgaTreeSplitNodeId, usize)> {
        let (id, index) = self.tree_by_id.range(key.clone()..).next()?;
        id.has_same_created_at(key).then(|| (id.clone(), *index))
    }

    /// The index of a node known to exist.
    fn index_of(&self, id: &RgaTreeSplitNodeId) -> usize {
        self.find_node_index(id)
            .expect("the node was found in this split")
    }

    pub(crate) fn find_node_with_split(
        &mut self,
        pos: &RgaTreeSplitPos,
        edited_at: &TimeTicket,
    ) -> Result<(usize, DataSize, Option<usize>)> {
        let absolute_id = pos.absolute_id();
        let mut node_index = self.find_floor_node_prefer_to_left(&absolute_id)?;
        let node_offset = self.nodes[node_index].id().offset();
        let relative_offset = absolute_id
            .offset()
            .checked_sub(node_offset)
            .ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "position {} precedes node {}",
                    absolute_id.to_test_string(),
                    self.nodes[node_index].id().to_test_string()
                ))
            })?;

        let diff = self.split_node(node_index, relative_offset)?;

        while self
            .next_index(node_index)
            .is_some_and(|next| self.nodes[next].created_at().after(edited_at))
        {
            node_index += 1;
        }

        Ok((node_index, diff, self.next_index(node_index)))
    }

    pub(crate) fn find_between(&self, from: Option<usize>, to: Option<usize>) -> Vec<usize> {
        let Some(mut index) = from else {
            return Vec::new();
        };

        let mut nodes = Vec::new();
        while index < self.nodes.len() && Some(index) != to {
            nodes.push(index);
            index += 1;
        }

        nodes
    }

    pub(crate) fn node(&self, index: usize) -> Option<&RgaTreeSplitNode<V>> {
        self.nodes.get(index)
    }

    pub(crate) fn node_mut(&mut self, index: usize) -> Option<&mut RgaTreeSplitNode<V>> {
        self.nodes.get_mut(index)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &RgaTreeSplitNode<V>> {
        self.nodes.iter().skip(1)
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut RgaTreeSplitNode<V>> {
        self.nodes.iter_mut().skip(1)
    }

    pub(crate) fn insert_after_id(
        &mut self,
        prev_id: &RgaTreeSplitNodeId,
        node: RgaTreeSplitNode<V>,
    ) -> Result<RgaTreeSplitNodeId> {
        let prev_index = self.find_node_index(prev_id).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                prev_id.to_test_string()
            ))
        })?;
        let inserted_id = node.id().clone();
        self.insert_after_index(prev_index, node);
        Ok(inserted_id)
    }

    /// Returns the node holding `id` (JS `findNode`).
    pub(crate) fn find_node(&self, id: &RgaTreeSplitNodeId) -> Option<&RgaTreeSplitNode<V>> {
        self.find_floor_node(id).map(|index| &self.nodes[index])
    }

    pub(crate) fn find_node_mut_by_id(
        &mut self,
        id: &RgaTreeSplitNodeId,
    ) -> Option<&mut RgaTreeSplitNode<V>> {
        let index = self.find_node_index(id)?;
        self.nodes.get_mut(index)
    }

    pub(crate) fn to_json(&self) -> String {
        let values = self
            .iter()
            .filter(|node| !node.is_removed())
            .map(|node| node.value().to_json())
            .collect::<Vec<_>>();

        format!("[{}]", values.join(","))
    }

    pub(crate) fn to_test_string(&self) -> String {
        self.nodes
            .iter()
            .map(|node| {
                if node.is_removed() {
                    format!("{{{}}}", node.to_test_string())
                } else {
                    format!("[{}]", node.to_test_string())
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub(crate) fn purge(&mut self, child: &RgaTreeSplitNode<V>) {
        self.purge_by_id(&child.id_string());
    }

    /// Looks up the node `child_id` that `purge_by_id` would purge. Returns
    /// `None` when there is no such node, and otherwise `Some` of the
    /// creation ticket of the node behind it: `find_node_with_split` skips
    /// forward while the next node was created after the edit, so unlinking
    /// a node hands that decision to its successor, which must be causally
    /// stable first (JS `RGATreeSplit.purgeBarrierAt`).
    pub(crate) fn purge_barrier_by_id(&self, child_id: &str) -> Option<Option<TimeTicket>> {
        let index = self
            .nodes
            .iter()
            .position(|node| node.id_string() == child_id)
            .filter(|index| *index != 0)?;
        Some(
            self.nodes
                .get(index + 1)
                .map(|next| next.created_at().clone()),
        )
    }

    pub(crate) fn purge_by_id(&mut self, child_id: &str) -> bool {
        let Some(index) = self
            .nodes
            .iter()
            .position(|node| node.id_string() == child_id)
        else {
            return false;
        };

        if index == 0 {
            return false;
        }

        let child_id = self.nodes[index].id().clone();
        if let Some(cancelled) = self.cancelled.0.borrow_mut().as_mut() {
            cancelled.remove(&child_id);
        }
        let ins_prev_id = self.nodes[index].ins_prev_id.clone();
        let ins_next_id = self.nodes[index].ins_next_id.clone();
        self.nodes.remove(index);

        for node in &mut self.nodes {
            if node.ins_prev_id.as_ref() == Some(&child_id) {
                node.ins_prev_id = ins_prev_id.clone();
            }

            if node.ins_next_id.as_ref() == Some(&child_id) {
                node.ins_next_id = ins_next_id.clone();
            }
        }

        self.rebuild_indexes();
        true
    }

    fn find_node_pos(&self, index: usize) -> Result<RgaTreeSplitPos> {
        let length = self.len();
        if index > length {
            return Err(YorkieError::InvalidTextPosition(format!(
                "index {index} exceeds text length {length}"
            )));
        }

        if index == 0 {
            return Ok(RgaTreeSplitPos::new(self.initial_head().id().clone(), 0));
        }

        let (node_index, offset) = {
            let mut tree = self.tree_by_index.borrow_mut();
            let (splay_node, offset) = tree.find_for_text(index)?.ok_or_else(|| {
                YorkieError::InvalidTextPosition("text index tree is empty".to_owned())
            })?;
            (tree.value(splay_node).node_index, offset)
        };
        let node = self.nodes.get(node_index).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!("node index {node_index} not found"))
        })?;

        Ok(RgaTreeSplitPos::new(node.id().clone(), offset))
    }

    fn pos_to_index(&self, pos: &RgaTreeSplitPos, prefer_to_left: bool) -> Result<usize> {
        let absolute_id = pos.absolute_id();
        let index = if prefer_to_left {
            self.find_floor_node_prefer_to_left(&absolute_id)?
        } else {
            self.find_floor_node(&absolute_id).ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "node not found for {}",
                    absolute_id.to_test_string()
                ))
            })?
        };

        let splay_node = *self
            .splay_node_by_id
            .get(self.nodes[index].id())
            .ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "index node not found for {}",
                    self.nodes[index].id().to_test_string()
                ))
            })?;
        let cursor = self
            .tree_by_index
            .borrow_mut()
            .index_of(splay_node)
            .ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "index node is detached for {}",
                    self.nodes[index].id().to_test_string()
                ))
            })?;

        let offset = if self.nodes[index].is_removed() {
            0
        } else {
            absolute_id.offset() - self.nodes[index].id().offset()
        };

        Ok(cursor + offset)
    }

    fn find_floor_node_prefer_to_left(&self, id: &RgaTreeSplitNodeId) -> Result<usize> {
        let mut index = self.find_floor_node(id).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!("node not found for {}", id.to_test_string()))
        })?;

        if id.offset() > 0 && self.nodes[index].id().offset() == id.offset() {
            if let Some(ins_prev_id) = self.nodes[index].ins_prev_id.clone() {
                if let Some(ins_prev_index) = self.find_node_index(&ins_prev_id) {
                    index = ins_prev_index;
                }
            }
        }

        Ok(index)
    }

    fn find_floor_node(&self, id: &RgaTreeSplitNodeId) -> Option<usize> {
        let (floor_id, index) = self.tree_by_id.range(..=id.clone()).next_back()?;

        if floor_id != id && !floor_id.has_same_created_at(id) {
            return None;
        }

        Some(*index)
    }

    fn find_node_index(&self, id: &RgaTreeSplitNodeId) -> Option<usize> {
        self.tree_by_id.get(id).copied()
    }

    fn split_node(&mut self, index: usize, offset: usize) -> Result<DataSize> {
        let mut diff = DataSize::default();
        let content_len = self.nodes[index].content_len();
        if offset > content_len {
            return Err(YorkieError::InvalidTextPosition(format!(
                "offset {offset} exceeds node length {content_len}"
            )));
        }

        if offset == 0 || offset == content_len {
            return Ok(diff);
        }

        let prev_size = self.nodes[index].data_size();
        let left_id = self.nodes[index].id().clone();
        let old_ins_next_id = self.nodes[index].ins_next_id.clone();
        let mut split_node = self.nodes[index].split(offset);
        let split_id = split_node.id().clone();

        split_node.ins_prev_id = Some(left_id);
        split_node.ins_next_id = old_ins_next_id.clone();
        self.nodes[index].ins_next_id = Some(split_id.clone());

        if let Some(old_ins_next_id) = old_ins_next_id {
            if let Some(old_ins_next_index) = self.find_node_index(&old_ins_next_id) {
                self.nodes[old_ins_next_index].ins_prev_id = Some(split_id.clone());
            }
        }

        if split_node.has_cancels() {
            if let Some(cancelled) = self.cancelled.0.borrow_mut().as_mut() {
                cancelled.insert(split_id.clone());
            }
        }
        self.insert_after_index(index, split_node);

        add_data_size(&mut diff, self.nodes[index].data_size());
        add_data_size(&mut diff, self.nodes[index + 1].data_size());
        sub_data_size(&mut diff, prev_size);

        // A piece split off a removed node inherits `removed_at` without a
        // removal, so no removal registers a GC pair for it. Buffer one here,
        // or it stays in the text forever. It was never live, so the size
        // the split added goes to gc with it, and the caller's live diff gets
        // nothing.
        if let Some(removed_at) = self.nodes[index + 1].removed_at() {
            self.pending_gc_pairs
                .push((split_id.to_id_string(), diff, removed_at.clone()));
            return Ok(DataSize::default());
        }

        Ok(diff)
    }

    fn insert_after_index(&mut self, prev_index: usize, node: RgaTreeSplitNode<V>) -> usize {
        let insert_index = prev_index + 1;
        self.nodes.insert(insert_index, node);
        self.rebuild_indexes();
        insert_index
    }

    /// Removes the removable `candidates` and returns the value changes and
    /// the removed nodes (JS `deleteNodes`). The changes are computed from
    /// the indexes before the deletion, one per run of removed nodes between
    /// the kept ones.
    fn delete_nodes(
        &mut self,
        candidates: Vec<usize>,
        edited_at: TimeTicket,
        vector: Option<&VersionVector>,
    ) -> Result<DeleteResult<V>> {
        let (Some(&first), Some(&last)) = (candidates.first(), candidates.last()) else {
            return Ok((Vec::new(), Vec::new()));
        };

        let is_local = vector.is_none_or(VersionVector::is_empty);

        // 01. Split the candidates into the nodes to remove and the nodes to
        // keep. The edges outside the candidates are kept; `None` on the
        // right means the candidates reach the end of the text.
        let mut nodes_to_remove = Vec::new();
        let mut nodes_to_keep = vec![Some(first - 1)];
        for index in candidates {
            let node = &mut self.nodes[index];
            let creation_known = ticket_known(vector, node.created_at());
            let tombstone_known = node
                .removed_at()
                .is_some_and(|removed_at| ticket_known(vector, removed_at));

            // Once a tombstone can be cancelled by an undo, "the deleter knew
            // the node" is not enough -- the node has to have been LIVE in the
            // deleter's view. A node the deleter saw removed and an undo
            // revived here has to survive the deletion on every replica.
            let live_in_view = match vector {
                Some(vector) if !is_local => creation_known && !node.is_removed_in_view(vector),
                _ => !node.is_removed(),
            };
            if live_in_view && node.can_remove(&edited_at, creation_known, tombstone_known) {
                nodes_to_remove.push(index);
            } else {
                // A node this deletion saw live but someone else removed
                // concurrently records the deletion all the same. Otherwise an
                // undo of the other deletion revives it on replicas that
                // applied the two in one order and not in the other.
                if !is_local && live_in_view && node.is_removed() {
                    node.record_removal(edited_at.clone());
                }
                nodes_to_keep.push(Some(index));
            }
        }
        nodes_to_keep.push(self.next_index(last));

        // 02. Make the value changes with the indexes before the deletion.
        let changes = self.make_changes(&nodes_to_keep)?;

        // 03. Record the removals. Nodes that were already removed (a
        // concurrent LWW overwrite of an existing tombstone) are marked: they
        // already have a registered GC pair, and registering a second one
        // would toggle the first off.
        let mut removed_nodes = Vec::new();
        for index in nodes_to_remove {
            let node = &mut self.nodes[index];
            let already_removed = node.is_removed();
            node.record_removal(edited_at.clone());
            removed_nodes.push((node.clone(), already_removed));
        }

        self.rebuild_indexes();
        Ok((changes, removed_nodes))
    }

    /// Returns one change for each run of nodes between two kept boundaries,
    /// last run first (JS `makeChanges`).
    fn make_changes(&self, boundaries: &[Option<usize>]) -> Result<Vec<ValueChange<V>>> {
        let mut changes = Vec::new();
        for pair in boundaries.windows(2) {
            let left = pair[0].expect("only the right edge can be missing");
            let right = pair[1];
            if self.next_index(left) == right {
                continue;
            }

            let from = self.pos_to_index(&self.nodes[left + 1].create_pos_range().0, false)?;
            let to = match right {
                Some(right) => {
                    self.pos_to_index(&self.nodes[right - 1].create_pos_range().1, true)?
                }
                None => self.len(),
            };

            if from < to {
                changes.push(ValueChange {
                    from,
                    to,
                    value: None,
                });
            }
        }

        changes.reverse();
        Ok(changes)
    }

    fn next_index(&self, index: usize) -> Option<usize> {
        (index + 1 < self.nodes.len()).then_some(index + 1)
    }

    fn rebuild_indexes(&mut self) {
        let mut tree_by_index = SplayTree::new();
        let mut tree_by_id = BTreeMap::new();
        let mut splay_node_by_id = BTreeMap::new();

        for (index, node) in self.nodes.iter().enumerate() {
            let splay_node = tree_by_index.insert(SplitIndexValue::new(index, node.len()));
            tree_by_id.insert(node.id().clone(), index);
            splay_node_by_id.insert(node.id().clone(), splay_node);
        }

        self.tree_by_index = RefCell::new(tree_by_index);
        self.tree_by_id = tree_by_id;
        self.splay_node_by_id = splay_node_by_id;
    }
}

/// The part of `span` that `node` covers, with the span's own content (the
/// `record` closure of JS `retombstone`).
fn recorded_span<V: RgaTreeSplitValue>(
    span: &RestoreSpan<V>,
    node: &RgaTreeSplitNode<V>,
) -> RestoreSpan<V> {
    let start = node.id().offset();
    let end = start + node.content_len();
    RestoreSpan {
        created_at: span.created_at.clone(),
        start,
        end,
        value: span.value.substring(start - span.start, end - span.start),
    }
}

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

fn sub_data_size(target: &mut DataSize, size: DataSize) {
    target.data = target.data.saturating_sub(size.data);
    target.meta = target.meta.saturating_sub(size.meta);
}
