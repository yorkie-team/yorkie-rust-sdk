use super::counter::CrdtCounter;
use super::element::{CrdtElement, DataSize};
use super::object::CrdtObject;
use super::primitive::CrdtPrimitive;
use super::rga_tree_list::RgaTreeListNode;
use super::rga_tree_split::RgaTreeSplitNode;
use super::text::{CrdtText, TextValue};
use super::tree::CrdtTree;
use crate::{Result, TimeTicket, YorkieError, TIME_TICKET_SIZE};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtElementPair {
    element: CrdtElement,
    parent: Option<CrdtElement>,
}

impl CrdtElementPair {
    fn new(element: CrdtElement, parent: Option<CrdtElement>) -> Self {
        Self { element, parent }
    }

    pub(crate) fn element(&self) -> &CrdtElement {
        &self.element
    }

    pub(crate) fn parent(&self) -> Option<&CrdtElement> {
        self.parent.as_ref()
    }
}

/// A registered GC pair. `child_size` and `removed_at` are what the pair was
/// registered with. JS reads a Text node's current size and `removedAt` when
/// it collects, and a Text node can change under its pair: a split leaves the
/// original smaller, a later removal overwrites `removed_at`, an undo cancels
/// it. So `collect` reads a Text node's current state from the text (see
/// `TextGcChild`), and these two serve only the children whose state cannot
/// change: removed attributes and the Array and Tree children.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtGcPair {
    child_id: String,
    child_size: DataSize,
    removed_at: TimeTicket,
    /// The Text node as it was when its text was purged with an element.
    /// JS keeps a reference to the detached node and keeps collecting it;
    /// this is that node, frozen as nothing can change it any more.
    detached: Option<Box<RgaTreeSplitNode<TextValue>>>,
}

impl CrdtGcPair {
    fn new(child_id: String, child_size: DataSize, removed_at: TimeTicket) -> Self {
        Self {
            child_id,
            child_size,
            removed_at,
            detached: None,
        }
    }
}

/// The state of a Text node under a GC pair as `collect` reads it, taken
/// from the node when it collects (JS `pair.child.getRemovedAt()`,
/// `getDataSize()` and `heldFromPurge`).
#[derive(Debug, Clone)]
struct TextGcChild {
    removed_at: Option<TimeTicket>,
    size: DataSize,
    held: bool,
}

impl TextGcChild {
    fn of(node: &RgaTreeSplitNode<TextValue>, vector: &crate::VersionVector) -> Self {
        Self {
            removed_at: node.removed_at().cloned(),
            size: node.data_size(),
            held: node.held_from_purge(vector),
        }
    }
}

/// The size of a document in bytes (JS `DocSize`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocSize {
    /// The size of the elements and nodes still in the document.
    pub live: DataSize,
    /// The size of the removed ones waiting for garbage collection.
    pub gc: DataSize,
}

impl DocSize {
    /// Returns the live and garbage sizes added up, the size a document's
    /// maximum size is checked against (JS `totalDocSize`).
    pub fn total(&self) -> usize {
        self.live.total() + self.gc.total()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RootStats {
    pub(crate) elements: usize,
    pub(crate) gc_elements: usize,
    pub(crate) gc_pairs: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtRoot {
    root_object: CrdtObject,
    element_pair_by_created_at: BTreeMap<String, CrdtElementPair>,
    gc_element_set_by_created_at: BTreeSet<String>,
    gc_pair_by_child_id: BTreeMap<String, CrdtGcPair>,
    /// The exact size charged to `doc_size.gc` for every registered element
    /// whose size counts toward gc rather than live (JS `sizeInGC`). An
    /// element gets there by its own removal or by a removed ancestor, and
    /// recording the amount, not a flag, keeps both sides balanced although
    /// `data_size` grows by a ticket once `removed_at` is set. JS keys this
    /// by element identity, since an undo copy can share a tombstone's
    /// createdAt; Rust keys it by createdAt (see the porting gaps).
    size_in_gc: BTreeMap<String, DataSize>,
    doc_size: DocSize,
    /// The Texts, by creation time, whose removal ledgers hold cancels that
    /// `garbage_collect` may prune once every replica knows them (JS
    /// `ledgerHolders`). JS holds the Text itself; Rust looks it up, so a
    /// Text purged since is dropped on the next prune.
    ledger_holders: BTreeSet<TimeTicket>,
}

impl CrdtRoot {
    pub(crate) fn new(root_object: CrdtObject) -> Self {
        let mut root = Self {
            root_object,
            element_pair_by_created_at: BTreeMap::new(),
            gc_element_set_by_created_at: BTreeSet::new(),
            gc_pair_by_child_id: BTreeMap::new(),
            size_in_gc: BTreeMap::new(),
            doc_size: DocSize::default(),
            ledger_holders: BTreeSet::new(),
        };
        let root_element = root.root_element();
        root.register_element_for_rebuild(&root_element, None, false);
        root.register_removed_descendants_for_rebuild(&root_element);
        root.register_gc_pairs(&root_element);
        root
    }

    pub(crate) fn create() -> Self {
        Self::new(CrdtObject::create(TimeTicket::initial()))
    }

    pub(crate) fn find_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtElement> {
        self.element_pair_by_created_at
            .get(&created_at.to_id_string())
            .map(CrdtElementPair::element)
    }

    pub(crate) fn find_element_pair_by_created_at(
        &self,
        created_at: &TimeTicket,
    ) -> Option<&CrdtElementPair> {
        self.element_pair_by_created_at
            .get(&created_at.to_id_string())
    }

    pub(crate) fn create_sub_paths(&self, created_at: &TimeTicket) -> Result<Vec<String>> {
        let Some(mut pair) = self.find_element_pair_by_created_at(created_at) else {
            return Ok(Vec::new());
        };

        let mut sub_paths = Vec::new();
        while let Some(parent) = pair.parent() {
            let child_created_at = pair.element().created_at();
            let sub_path = sub_path_of(parent, child_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(child_created_at.to_id_string()))?;
            sub_paths.insert(0, sub_path);

            pair = self
                .find_element_pair_by_created_at(parent.created_at())
                .ok_or_else(|| {
                    YorkieError::MissingCrdtElement(parent.created_at().to_id_string())
                })?;
        }

        sub_paths.insert(0, "$".to_owned());
        Ok(sub_paths)
    }

    pub(crate) fn create_path(&self, created_at: &TimeTicket) -> Result<String> {
        Ok(self.create_sub_paths(created_at)?.join("."))
    }

    pub(crate) fn register_element(&mut self, element: &CrdtElement, parent: Option<&CrdtElement>) {
        self.register_element_internal(element, parent);
        self.register_ledger_holders(element);
    }

    /// Registers every Text at or under `element` whose ledgers hold cancels
    /// as a ledger holder, the ledger part of JS `registerInternalGCPairs`.
    /// An undo re-sets a copy of a removed Text and a remote payload decodes
    /// one with its ledgers; unregistered, such a ledger would never be
    /// pruned. The GC pairs of the tombstones inside are still not booked
    /// here (see the porting gaps).
    fn register_ledger_holders(&mut self, element: &CrdtElement) {
        let mut holders = Vec::new();
        visit_texts(element, &mut |text: &CrdtText| {
            if text.has_cancelled_removals() {
                holders.push(text.created_at().clone());
            }
        });
        for created_at in holders {
            self.register_ledger_holder(&created_at);
        }
    }

    pub(crate) fn deregister_element(&mut self, element: &CrdtElement) -> usize {
        let mut count = 0;
        self.deregister_element_internal(element, &mut count);
        count
    }

    /// Registers a removed element for collection and moves its size, with
    /// its descendants', from live to gc (JS `registerRemovedElement`).
    pub(crate) fn register_removed_element(&mut self, element: &CrdtElement) {
        let moved = self.book_removed_element(element);

        // The removal set `removed_at` after the element was registered, so
        // the size just moved holds a ticket live was never charged. Live
        // gets it back, but only on the move that carried it: a size already
        // in gc did not take it from live.
        if moved && element.removed_at().is_some() {
            self.doc_size.live.meta += TIME_TICKET_SIZE;
        }
    }

    /// Registers an element that already carried its `removed_at` when it
    /// was registered, such as the losing value of a concurrent set (JS
    /// `adoptRemovedElement`). It is `register_removed_element` without the
    /// ticket refund: the size registered in live already held the ticket.
    fn adopt_removed_element(&mut self, element: &CrdtElement) {
        self.book_removed_element(element);
    }

    /// Puts `element` on the collection worklist and moves its size, with
    /// its descendants', to gc. Reports whether its own size left live.
    fn book_removed_element(&mut self, element: &CrdtElement) -> bool {
        let created_at = element.created_at().to_id_string();
        if let Some(pair) = self.element_pair_by_created_at.get_mut(&created_at) {
            pair.element = element.deepcopy();
        }

        let moved = self.move_size_to_gc(element);
        self.move_descendants_to_gc(element);
        self.gc_element_set_by_created_at.insert(created_at);
        moved
    }

    /// Moves the size of every descendant of `element` from live to gc (JS
    /// `moveDescendantsToGC`). A container's own size is its metadata only,
    /// so moving the container alone would strand its members in live.
    fn move_descendants_to_gc(&mut self, element: &CrdtElement) {
        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.move_size_to_gc(child);
                    self.move_descendants_to_gc(child);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.move_size_to_gc(child);
                    self.move_descendants_to_gc(child);
                }
            }
            CrdtElement::Primitive(_)
            | CrdtElement::Counter(_)
            | CrdtElement::Text(_)
            | CrdtElement::Tree(_) => {}
        }
    }

    /// Moves the size of `element` from live to gc and reports whether it
    /// moved a size live was holding (JS `moveSizeToGC`). A size already in
    /// gc, because the element or a container above it was removed before,
    /// only has its charge topped up to the current size.
    fn move_size_to_gc(&mut self, element: &CrdtElement) -> bool {
        let size = element.data_size();
        let created_at = element.created_at().to_id_string();

        if let Some(charged) = self.size_in_gc.insert(created_at, size) {
            adjust_data_size(&mut self.doc_size.gc, charged, size);
            return false;
        }

        add_data_size(&mut self.doc_size.gc, size);
        sub_data_size(&mut self.doc_size.live, size);
        true
    }

    pub(crate) fn register_gc_pair(&mut self, child: &RgaTreeListNode) {
        if let Some(removed_at) = child.removed_at() {
            self.register_gc_pair_by_id(child.id_string(), child.data_size(), removed_at.clone());
        }
    }

    pub(crate) fn register_gc_pair_by_id(
        &mut self,
        child_id: String,
        child_size: DataSize,
        removed_at: TimeTicket,
    ) {
        if let Some(pair) = self.gc_pair_by_child_id.remove(&child_id) {
            // A second registration un-registers a revived child (JS
            // `registerGCPair`), but only removed attributes and Array and
            // Tree children toggle. A revived Text node never comes back
            // this way: every revive unregisters its pair first
            // (`unregister_gc_pair_by_id`), and `collect` drops a revived
            // pair left behind. So a Text node registered again is another
            // node under the same ID string -- a piece of an undo's copy of
            // a Text whose original left its pair behind, purged (`detached`)
            // or replaced by the copy -- which JS, keying pairs by node and
            // parent, books as a pair of its own. Here the new pair replaces
            // the old one: the old charge leaves gc as `collect` would take
            // it, the new one is booked below (see the porting gaps).
            let charged = match pair.detached.as_deref() {
                Some(node) => Some(node.data_size()),
                None if self.has_text_node(&child_id) => Some(pair.child_size),
                None => None,
            };
            match charged {
                Some(charged) => sub_data_size(&mut self.doc_size.gc, charged),
                None => {
                    sub_data_size(&mut self.doc_size.gc, pair.child_size);
                    return;
                }
            }
        }

        let pair = CrdtGcPair::new(child_id.clone(), child_size, removed_at);
        add_data_size(&mut self.doc_size.gc, pair.child_size);
        self.gc_pair_by_child_id.insert(child_id, pair);
    }

    /// Unregisters the GC pair of the Text node `child_id`, revived by an
    /// identity undo, so a later re-registration (a redo) is not read as the
    /// toggle in `register_gc_pair_by_id` (JS `unregisterGCPair`).
    /// `child_size` is the node's size now that its `removed_at` is cleared;
    /// with the ticket the node was registered with, that is the size its
    /// removal put in gc, and gc gives it back.
    ///
    /// JS also adds `child_size` back to live, mirroring `registerGCPair`,
    /// which took it out of live. Rust registration leaves live alone -- a
    /// text removal does not move its size out of live (see the porting
    /// gaps) -- so live keeps the revived size already, and adding it again
    /// would count it twice.
    pub(crate) fn unregister_gc_pair_by_id(&mut self, child_id: &str, child_size: DataSize) {
        if self.gc_pair_by_child_id.remove(child_id).is_none() {
            return;
        }

        sub_data_size(&mut self.doc_size.gc, child_size);
        self.doc_size.gc.meta = self.doc_size.gc.meta.saturating_sub(TIME_TICKET_SIZE);
    }

    /// Registers the Text `text_created_at`, whose removal ledgers may be
    /// pruned (JS `registerLedgerHolder`).
    pub(crate) fn register_ledger_holder(&mut self, text_created_at: &TimeTicket) {
        self.ledger_holders.insert(text_created_at.clone());
    }

    /// Prunes every registered holder and drops the ones with nothing left
    /// to prune (JS `pruneLedgers`). A removed holder is dropped too: an
    /// undo that brings it back registers its copy again. So is one no
    /// longer in the document.
    fn prune_ledgers(&mut self, vector: &crate::VersionVector) {
        for created_at in std::mem::take(&mut self.ledger_holders) {
            let Some(text) = self.text_by_created_at_mut(&created_at) else {
                continue;
            };
            if text.removed_at().is_none() && text.prune_ledgers(vector) > 0 {
                self.ledger_holders.insert(created_at);
            }
        }
    }

    pub(crate) fn acc(&mut self, diff: DataSize) {
        add_data_size(&mut self.doc_size.live, diff);
    }

    pub(crate) fn get_object_member(
        &self,
        parent_created_at: &TimeTicket,
        key: &str,
    ) -> Result<Option<&CrdtElement>> {
        let object = self
            .object_by_created_at(parent_created_at)
            .ok_or_else(|| self.object_parent_error(parent_created_at))?;

        Ok(object.get(key))
    }

    pub(crate) fn object_member_sub_path(
        &self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> Result<Option<&str>> {
        let object = self
            .object_by_created_at(parent_created_at)
            .ok_or_else(|| self.object_parent_error(parent_created_at))?;

        Ok(object.sub_path_of(created_at))
    }

    pub(crate) fn container_sub_path(
        &self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> Result<Option<String>> {
        if let Some(object) = self.object_by_created_at(parent_created_at) {
            return Ok(object.sub_path_of(created_at).map(ToOwned::to_owned));
        }

        if let Some(array) = self.array_by_created_at(parent_created_at) {
            return Ok(array.sub_path_of(created_at));
        }

        Err(self.container_parent_error(parent_created_at))
    }

    pub(crate) fn set_object_member(
        &mut self,
        parent_created_at: &TimeTicket,
        key: impl Into<String>,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<(CrdtElement, Option<CrdtElement>)> {
        let key = key.into();
        let value_created_at = value.created_at().clone();

        if self.object_by_created_at(parent_created_at).is_none() {
            return Err(self.object_parent_error(parent_created_at));
        }

        let (inserted, removed) = {
            let object = self
                .object_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            let removed = object.set(key, value, executed_at);
            let inserted = object
                .get_by_id(&value_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
                .deepcopy();

            (inserted, removed)
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);

        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

        self.register_element(&inserted, Some(&parent));
        if let Some(removed) = &removed {
            self.register_removed_element(removed);
        }
        if inserted.removed_at().is_some() {
            self.adopt_removed_element(&inserted);
        }

        Ok((inserted, removed))
    }

    pub(crate) fn remove_object_member(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.object_by_created_at(parent_created_at).is_none() {
            return Err(self.object_parent_error(parent_created_at));
        }

        let removed = {
            let object = self
                .object_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            object.delete(created_at, executed_at)?
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);
        if removed.removed_at().is_some() {
            self.register_removed_element(&removed);
        }

        Ok(removed)
    }

    pub(crate) fn get_container_child(
        &self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> Result<Option<CrdtElement>> {
        if let Some(object) = self.object_by_created_at(parent_created_at) {
            return Ok(object.get_by_id(created_at).map(CrdtElement::deepcopy));
        }

        if let Some(array) = self.array_by_created_at(parent_created_at) {
            return Ok(array.get_by_id(created_at).map(CrdtElement::deepcopy));
        }

        Err(self.container_parent_error(parent_created_at))
    }

    pub(crate) fn insert_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        let value_created_at = value.created_at().clone();

        if self.array_by_created_at(parent_created_at).is_none() {
            return Err(self.array_parent_error(parent_created_at));
        }

        let inserted = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            array.insert_after(prev_created_at, value, Some(executed_at))?;
            array
                .get_by_id(&value_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
                .deepcopy()
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);

        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

        self.register_element(&inserted, Some(&parent));
        if inserted.removed_at().is_some() {
            self.adopt_removed_element(&inserted);
        }

        Ok(inserted)
    }

    pub(crate) fn move_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.array_by_created_at(parent_created_at).is_none() {
            return Err(self.array_parent_error(parent_created_at));
        }

        let (moved, dead_node, previous_size) = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            let previous_size = array
                .get_by_id(created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?
                .data_size();
            let dead_node = array.move_after(prev_created_at, created_at, executed_at)?;
            let dead_node = dead_node.filter(|node| node.removed_at().is_some());
            let moved = array
                .get_by_id(created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?
                .deepcopy();

            (moved, dead_node, previous_size)
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);
        adjust_data_size(&mut self.doc_size.live, previous_size, moved.data_size());
        if let Some(dead_node) = &dead_node {
            self.register_gc_pair(dead_node);
        }
        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;
        if let Some(pair) = self
            .element_pair_by_created_at
            .get_mut(&created_at.to_id_string())
        {
            pair.element = moved.deepcopy();
            pair.parent = Some(parent);
        }

        Ok(moved)
    }

    pub(crate) fn set_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<(CrdtElement, CrdtElement)> {
        let value_created_at = value.created_at().clone();

        if self.array_by_created_at(parent_created_at).is_none() {
            return Err(self.array_parent_error(parent_created_at));
        }

        let (inserted, removed) = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            let removed = array.set(created_at, value, executed_at)?;
            let inserted = array
                .get_by_id(&value_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
                .deepcopy();

            (inserted, removed)
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);

        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

        self.register_element(&inserted, Some(&parent));
        self.register_removed_element(&removed);
        if inserted.removed_at().is_some() {
            self.adopt_removed_element(&inserted);
        }

        Ok((inserted, removed))
    }

    pub(crate) fn remove_container_element(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.object_by_created_at(parent_created_at).is_some() {
            return self.remove_object_member(parent_created_at, created_at, executed_at);
        }

        if self.array_by_created_at(parent_created_at).is_some() {
            return self.remove_array_element(parent_created_at, created_at, executed_at);
        }

        Err(self.container_parent_error(parent_created_at))
    }

    pub(crate) fn get_element_map_size(&self) -> usize {
        self.element_pair_by_created_at.len()
    }

    pub(crate) fn get_garbage_element_set_size(&self) -> usize {
        let mut seen = BTreeSet::new();

        for created_at in &self.gc_element_set_by_created_at {
            seen.insert(created_at.clone());
            if let Some(pair) = self.element_pair_by_created_at.get(created_at) {
                collect_descendant_ids(pair.element(), &mut seen);
            }
        }

        seen.len()
    }

    pub(crate) fn get_garbage_len(&self) -> usize {
        self.get_garbage_element_set_size() + self.gc_pair_by_child_id.len()
    }

    /// Purges the tombstones `vector` covers and returns how many it purged
    /// (JS `CRDTRoot.garbageCollect`).
    ///
    /// A pass can hold a purge back (see `collect`), and holding one back
    /// can be the only reason another is held back: purging a node hands its
    /// successor to the node in front of it, and that successor is one this
    /// pass already found stable. So a pass that both purged and deferred may
    /// have more to do, and the loop repeats until a pass purges nothing new
    /// or defers nothing. Everything held back stays on the worklist for the
    /// next vector that covers it.
    pub(crate) fn garbage_collect(&mut self, vector: &crate::VersionVector) -> usize {
        let mut count = 0;

        // Pruning first can release a tombstone whose last pending cancel the
        // vector now covers.
        self.prune_ledgers(vector);

        loop {
            let (purged, deferred) = self.collect(vector);
            count += purged;
            if purged == 0 || deferred == 0 {
                return count;
            }
        }
    }

    /// Runs one collection pass and returns how much it purged and how much
    /// it held back on a barrier (JS `CRDTRoot.collect`).
    ///
    /// A tombstone is not only a value that is gone, it is also a place in
    /// its parent that other replicas may still be deciding against:
    /// `removed_at` covers the value, and the barrier covers the place. An
    /// RGA resolves a concurrent insert by walking forward past newer nodes,
    /// tombstones included, so purging a node is safe only once the node
    /// that would end that walk in its place is causally stable as well.
    fn collect(&mut self, vector: &crate::VersionVector) -> (usize, usize) {
        let mut count = 0;
        let mut deferred = 0;

        let removed_element_ids = self
            .gc_element_set_by_created_at
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for created_id in removed_element_ids {
            // The indexed copy may be stale: purging another element of this
            // pass can have removed this one, or a descendant of it, from the
            // live tree. Skip what is gone, and purge and deregister what the
            // live tree holds, as JS does with its live references.
            let Some(pair) = self.element_pair_by_created_at.get(&created_id) else {
                continue;
            };
            let Some(parent_created_at) = pair.parent().map(|p| p.created_at().clone()) else {
                continue;
            };
            let created_at = pair.element().created_at().clone();
            let Some(element) = self.actual_element_by_created_at(&created_at) else {
                continue;
            };
            let Some(removed_at) = element.removed_at() else {
                continue;
            };
            if !vector.after_or_equal(removed_at) {
                continue;
            }

            if let Some(array) = self.array_by_created_at(&parent_created_at) {
                let barrier = array.purge_barrier_at(&created_at);
                if barrier.is_some_and(|at| !vector.after_or_equal(&at)) {
                    deferred += 1;
                    continue;
                }
            }

            // As in JS, a member that cannot be purged is skipped, not
            // reported: collection runs inside every change pack apply, and
            // failing there would stop the document from syncing for good.
            // It stays on the worklist, charged to gc.
            let purged = if let Some(object) = self.object_by_created_at_mut(&parent_created_at) {
                object.purge(&element)
            } else if let Some(array) = self.array_by_created_at_mut(&parent_created_at) {
                array.purge(&element)
            } else {
                // Unreachable: `element` was just found in the live tree, and
                // only objects and arrays hold elements. Were the indexed
                // parent stale, deregistering without a purge would still
                // match JS, which purges from the detached parent it holds.
                debug_assert!(
                    false,
                    "removed element {created_id} has no container parent"
                );
                Ok(())
            };
            if purged.is_err() {
                continue;
            }

            self.detach_text_gc_pairs(&element);
            count += self.deregister_element(&element);
            self.refresh_element_pair_and_ancestors(&parent_created_at);
        }

        let text_children = self.text_gc_children(vector);
        let gc_pairs = self
            .gc_pair_by_child_id
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for pair in gc_pairs {
            // A Text node is read as it is now, or as it was when its text
            // went (see `CrdtGcPair`); any other child as it was registered.
            let text_child = text_children.get(&pair.child_id).cloned().or_else(|| {
                pair.detached
                    .as_deref()
                    .map(|node| TextGcChild::of(node, vector))
            });
            let (removed_at, size) = match &text_child {
                Some(TextGcChild {
                    removed_at: Some(removed_at),
                    size,
                    ..
                }) => (removed_at.clone(), *size),
                Some(TextGcChild {
                    removed_at: None,
                    size,
                    ..
                }) => {
                    // The node was revived but its pair was not unregistered.
                    // Reverse the gc accounting and drop the stale entry, so
                    // the toggle in `register_gc_pair_by_id` cannot be
                    // tripped later (JS `collect`).
                    self.unregister_gc_pair_by_id(&pair.child_id, *size);
                    continue;
                }
                None => (pair.removed_at.clone(), pair.child_size),
            };
            if !vector.after_or_equal(&removed_at) {
                continue;
            }

            // A Text node whose removal ledger still matters stays. Unlike a
            // barrier, holding it back frees nothing else, so it is not
            // counted as deferred (JS `heldFromPurge`).
            if text_child.as_ref().is_some_and(|child| child.held) {
                continue;
            }

            // A child no longer in the tree went with its container: a
            // purged element holding a Text or Tree, or a purged Tree node
            // holding its descendants. JS keeps a reference to the parent and
            // purges the child from the detached structure; here there is
            // nothing left to purge, so the pair is only dropped, with the
            // same accounting.
            let Some(site) = self.root_object.locate_gc_pair(&pair.child_id) else {
                self.drop_gc_pair(&pair.child_id, size);
                count += 1;
                continue;
            };
            if site.barrier.is_some_and(|at| !vector.after_or_equal(&at)) {
                deferred += 1;
                continue;
            }

            if self.purge_gc_pair_by_child_id(&pair.child_id) {
                self.drop_gc_pair(&pair.child_id, size);
                count += 1;
                self.refresh_element_pair_and_ancestors(&site.container);
            }
        }

        (count, deferred)
    }

    pub(crate) fn object(&self) -> &CrdtObject {
        &self.root_object
    }

    pub(crate) fn object_mut(&mut self) -> &mut CrdtObject {
        &mut self.root_object
    }

    pub(crate) fn object_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtObject> {
        if self.root_object.created_at() == created_at {
            return Some(&self.root_object);
        }

        self.root_object.find_object_by_created_at(created_at)
    }

    pub(crate) fn object_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtObject> {
        if self.root_object.created_at() == created_at {
            return Some(&mut self.root_object);
        }

        self.root_object.find_object_by_created_at_mut(created_at)
    }

    pub(crate) fn array_by_created_at(
        &self,
        created_at: &TimeTicket,
    ) -> Option<&super::array::CrdtArray> {
        self.root_object.find_array_by_created_at(created_at)
    }

    pub(crate) fn array_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut super::array::CrdtArray> {
        self.root_object.find_array_by_created_at_mut(created_at)
    }

    pub(crate) fn text_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtText> {
        self.root_object.find_text_by_created_at(created_at)
    }

    pub(crate) fn text_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtText> {
        self.root_object.find_text_by_created_at_mut(created_at)
    }

    pub(crate) fn counter_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtCounter> {
        self.root_object.find_counter_by_created_at(created_at)
    }

    pub(crate) fn counter_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtCounter> {
        self.root_object.find_counter_by_created_at_mut(created_at)
    }

    pub(crate) fn tree_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtTree> {
        self.root_object.find_tree_by_created_at(created_at)
    }

    pub(crate) fn tree_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtTree> {
        self.root_object.find_tree_by_created_at_mut(created_at)
    }

    pub(crate) fn increase_counter(
        &mut self,
        created_at: &TimeTicket,
        value: &CrdtPrimitive,
        actor: Option<&str>,
    ) -> Result<()> {
        if self.counter_by_created_at(created_at).is_none() {
            return Err(self.counter_parent_error(created_at));
        }

        let (previous_size, current_size) = {
            let counter = self
                .counter_by_created_at_mut(created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?;
            let previous_size = counter.data_size();
            if counter.is_dedup() {
                counter.increase_dedup(value, actor.unwrap_or_default())?;
            } else {
                counter.increase(value)?;
            }
            (previous_size, counter.data_size())
        };

        self.refresh_element_pair_and_ancestors(created_at);
        adjust_data_size(&mut self.doc_size.live, previous_size, current_size);
        Ok(())
    }

    pub(crate) fn doc_size(&self) -> DocSize {
        self.doc_size
    }

    pub(crate) fn deepcopy(&self) -> Self {
        Self::new(self.root_object.deepcopy())
    }

    pub(crate) fn to_json(&self) -> String {
        self.root_object.to_json()
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.root_object.to_sorted_json()
    }

    pub(crate) fn stats(&self) -> RootStats {
        RootStats {
            elements: self.get_element_map_size(),
            gc_elements: self.get_garbage_element_set_size(),
            gc_pairs: self.gc_pair_by_child_id.len(),
        }
    }

    pub(crate) fn root_element(&self) -> CrdtElement {
        CrdtElement::object(self.root_object.deepcopy())
    }

    fn actual_element_by_created_at(&self, created_at: &TimeTicket) -> Option<CrdtElement> {
        if self.root_object.created_at() == created_at {
            return Some(self.root_element());
        }

        self.root_object
            .find_by_created_at(created_at)
            .map(CrdtElement::deepcopy)
    }

    fn refresh_element_pair_subtree(&mut self, created_at: &TimeTicket) {
        let Some(element) = self.actual_element_by_created_at(created_at) else {
            return;
        };
        let parent = self
            .find_element_pair_by_created_at(created_at)
            .and_then(|pair| pair.parent())
            .map(CrdtElement::deepcopy);

        self.refresh_element_pair_tree(&element, parent.as_ref());
    }

    fn refresh_element_pair_tree(&mut self, element: &CrdtElement, parent: Option<&CrdtElement>) {
        if let Some(pair) = self
            .element_pair_by_created_at
            .get_mut(&element.created_at().to_id_string())
        {
            pair.element = element.deepcopy();
            pair.parent = parent.map(CrdtElement::deepcopy);
        }

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.refresh_element_pair_tree(child, Some(element));
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.refresh_element_pair_tree(child, Some(element));
                }
            }
            CrdtElement::Primitive(_)
            | CrdtElement::Counter(_)
            | CrdtElement::Text(_)
            | CrdtElement::Tree(_) => {}
        }
    }

    fn refresh_element_pair_and_ancestors(&mut self, created_at: &TimeTicket) {
        let mut current = Some(created_at.clone());

        while let Some(created_at) = current {
            current = self
                .find_element_pair_by_created_at(&created_at)
                .and_then(|pair| pair.parent())
                .map(|parent| parent.created_at().clone());

            self.refresh_element_pair_subtree(&created_at);
        }
    }

    fn object_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "object",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn array_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "array",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn container_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "object or array",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn counter_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "counter",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn remove_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        let removed = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            array.delete(created_at, executed_at)?
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);
        if removed.removed_at().is_some() {
            self.register_removed_element(&removed);
        }

        Ok(removed)
    }

    fn register_element_internal(&mut self, element: &CrdtElement, parent: Option<&CrdtElement>) {
        self.element_pair_by_created_at.insert(
            element.created_at().to_id_string(),
            CrdtElementPair::new(element.deepcopy(), parent.map(|parent| parent.deepcopy())),
        );
        add_data_size(&mut self.doc_size.live, element.data_size());

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_element_internal(child, Some(element));
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.register_element_internal(child, Some(element));
                }
            }
            CrdtElement::Primitive(_)
            | CrdtElement::Counter(_)
            | CrdtElement::Text(_)
            | CrdtElement::Tree(_) => {}
        }
    }

    fn deregister_element_internal(&mut self, element: &CrdtElement, count: &mut usize) {
        let created_at = element.created_at().to_id_string();
        // Take the size out of the side that holds it, by the amount charged
        // there (JS `deregisterElement`). A member a peer set into a container
        // already removed here never went through a removal, so it is still
        // in live.
        match self.size_in_gc.remove(&created_at) {
            Some(charged) => sub_data_size(&mut self.doc_size.gc, charged),
            None => sub_data_size(&mut self.doc_size.live, element.data_size()),
        }
        self.element_pair_by_created_at.remove(&created_at);
        self.gc_element_set_by_created_at.remove(&created_at);
        *count += 1;

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.deregister_element_internal(child, count);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.deregister_element_internal(child, count);
                }
            }
            CrdtElement::Primitive(_)
            | CrdtElement::Counter(_)
            | CrdtElement::Text(_)
            | CrdtElement::Tree(_) => {}
        }
    }

    fn register_element_for_rebuild(
        &mut self,
        element: &CrdtElement,
        parent: Option<&CrdtElement>,
        ancestor_removed: bool,
    ) {
        self.element_pair_by_created_at.insert(
            element.created_at().to_id_string(),
            CrdtElementPair::new(element.deepcopy(), parent.map(|parent| parent.deepcopy())),
        );

        // A removed element and everything under it count toward gc, as
        // JS charges them when it adopts the tombstones it registers.
        let in_removed_subtree = ancestor_removed || element.is_removed();
        if in_removed_subtree {
            let size = element.data_size();
            add_data_size(&mut self.doc_size.gc, size);
            self.size_in_gc
                .insert(element.created_at().to_id_string(), size);
        } else {
            add_data_size(&mut self.doc_size.live, element.data_size());
        }

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_element_for_rebuild(child, Some(element), in_removed_subtree);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.register_element_for_rebuild(child, Some(element), in_removed_subtree);
                }
            }
            CrdtElement::Primitive(_)
            | CrdtElement::Counter(_)
            | CrdtElement::Text(_)
            | CrdtElement::Tree(_) => {}
        }
    }

    fn register_removed_descendants_for_rebuild(&mut self, element: &CrdtElement) {
        if element.removed_at().is_some() {
            let created_at = element.created_at().to_id_string();
            if let Some(pair) = self.element_pair_by_created_at.get_mut(&created_at) {
                pair.element = element.deepcopy();
            }

            self.gc_element_set_by_created_at.insert(created_at);
        }

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_removed_descendants_for_rebuild(child);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.register_removed_descendants_for_rebuild(child);
                }
            }
            CrdtElement::Primitive(_)
            | CrdtElement::Counter(_)
            | CrdtElement::Text(_)
            | CrdtElement::Tree(_) => {}
        }
    }

    fn register_gc_pairs(&mut self, element: &CrdtElement) {
        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_gc_pairs(child);
                }
            }
            CrdtElement::Array(array) => {
                for node in array.iter_all_nodes() {
                    if node.element().is_none() && node.removed_at().is_some() {
                        self.register_gc_pair(node);
                    }
                    if let Some(child) = node.element() {
                        self.register_gc_pairs(child);
                    }
                }
            }
            CrdtElement::Text(text) => {
                for (child_id, child_size, removed_at) in text.gc_pair_entries() {
                    self.register_gc_pair_by_id(child_id, child_size, removed_at);
                }
                // JS `registerInternalGCPairs`: a Text loaded with cancels in
                // its ledgers is a holder from the start.
                if text.has_cancelled_removals() {
                    self.register_ledger_holder(text.created_at());
                }
            }
            CrdtElement::Tree(tree) => {
                for (child_id, child_size, removed_at) in tree.gc_pair_entries() {
                    self.register_gc_pair_by_id(child_id, child_size, removed_at);
                }
            }
            CrdtElement::Primitive(_) | CrdtElement::Counter(_) => {}
        }
    }

    fn purge_gc_pair_by_child_id(&mut self, child_id: &str) -> bool {
        self.root_object.purge_gc_pair_by_id(child_id)
    }

    /// Drops the pair of a collected child and takes `size`, the child's
    /// size when it went, out of gc (JS `collect`).
    fn drop_gc_pair(&mut self, child_id: &str, size: DataSize) {
        if self.gc_pair_by_child_id.remove(child_id).is_some() {
            sub_data_size(&mut self.doc_size.gc, size);
        }
    }

    /// Reads the current state of every registered Text node in the
    /// document, keyed by node ID, in one walk. Nothing a collection pass
    /// does changes another node's state, so one read serves the pass.
    fn text_gc_children(&self, vector: &crate::VersionVector) -> BTreeMap<String, TextGcChild> {
        let mut children = BTreeMap::new();
        if self.gc_pair_by_child_id.is_empty() {
            return children;
        }

        let mut read = |text: &CrdtText| {
            for node in text.nodes() {
                let id = node.id_string();
                if self.gc_pair_by_child_id.contains_key(&id) {
                    children.insert(id, TextGcChild::of(node, vector));
                }
            }
        };
        for (_, element) in self.root_object.iter_all() {
            visit_texts(element, &mut read);
        }
        children
    }

    /// Tells whether a Text in the document holds a node whose ID string is
    /// `child_id`. Only a registration that finds a pair under that ID
    /// asks, to tell a Text node from a child that toggles.
    fn has_text_node(&self, child_id: &str) -> bool {
        let mut found = false;
        for (_, element) in self.root_object.iter_all() {
            visit_texts(element, &mut |text: &CrdtText| {
                found = found || text.nodes().any(|node| node.id_string() == child_id);
            });
        }
        found
    }

    /// Freezes the registered Text nodes under `element`, which is being
    /// purged, into their pairs, so later passes still read them as JS reads
    /// the detached nodes it keeps.
    fn detach_text_gc_pairs(&mut self, element: &CrdtElement) {
        let pairs = &mut self.gc_pair_by_child_id;
        visit_texts(element, &mut |text: &CrdtText| {
            for node in text.nodes() {
                if let Some(pair) = pairs.get_mut(&node.id_string()) {
                    pair.detached = Some(Box::new(node.clone()));
                }
            }
        });
    }
}

/// Calls `f` on every Text at or under `element`, removed ones included.
fn visit_texts(element: &CrdtElement, f: &mut impl FnMut(&CrdtText)) {
    match element {
        CrdtElement::Object(object) => {
            for (_, child) in object.iter_all() {
                visit_texts(child, f);
            }
        }
        CrdtElement::Array(array) => {
            for child in array.iter_all() {
                visit_texts(child, f);
            }
        }
        CrdtElement::Text(text) => f(text),
        CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Tree(_) => {}
    }
}

fn sub_path_of(parent: &CrdtElement, created_at: &TimeTicket) -> Option<String> {
    match parent {
        CrdtElement::Object(object) => object.sub_path_of(created_at).map(ToOwned::to_owned),
        CrdtElement::Array(array) => array.sub_path_of(created_at),
        CrdtElement::Primitive(_)
        | CrdtElement::Counter(_)
        | CrdtElement::Text(_)
        | CrdtElement::Tree(_) => None,
    }
}

fn collect_descendant_ids(element: &CrdtElement, seen: &mut BTreeSet<String>) {
    match element {
        CrdtElement::Object(object) => {
            for (_, child) in object.iter_all() {
                seen.insert(child.created_at().to_id_string());
                collect_descendant_ids(child, seen);
            }
        }
        CrdtElement::Array(array) => {
            for child in array.iter_all() {
                seen.insert(child.created_at().to_id_string());
                collect_descendant_ids(child, seen);
            }
        }
        CrdtElement::Primitive(_)
        | CrdtElement::Counter(_)
        | CrdtElement::Text(_)
        | CrdtElement::Tree(_) => {}
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

fn adjust_data_size(target: &mut DataSize, old: DataSize, new: DataSize) {
    if new.data >= old.data {
        target.data += new.data - old.data;
    } else {
        target.data = target.data.saturating_sub(old.data - new.data);
    }

    if new.meta >= old.meta {
        target.meta += new.meta - old.meta;
    } else {
        target.meta = target.meta.saturating_sub(old.meta - new.meta);
    }
}

#[cfg(test)]
mod tests {
    use super::CrdtRoot;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::rht::Rht;
    use crate::crdt::text::CrdtText;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::{TimeTicket, VersionVector, TIME_TICKET_SIZE};
    use std::collections::BTreeMap;

    #[test]
    fn creates_root_with_initial_object() -> crate::Result<()> {
        let root = CrdtRoot::create();

        assert_eq!(1, root.get_element_map_size());
        assert!(root.find_by_created_at(&TimeTicket::initial()).is_some());
        assert_eq!("$", root.create_path(&TimeTicket::initial())?);
        assert_eq!("", root.create_path(&TimeTicket::max())?);
        assert_eq!("{}", root.to_json());
        assert_eq!("{}", root.to_sorted_json());
        Ok(())
    }

    #[test]
    fn registers_and_finds_object_members() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        assert_eq!(2, root.get_element_map_size());
        assert_eq!(Some(&member), root.find_by_created_at(&created_at));
        assert_eq!("$.k1", root.create_path(&created_at)?);
        assert_eq!(r#"{"k1":"k1"}"#, root.to_json());
        Ok(())
    }

    #[test]
    fn deregisters_object_members() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let removed_at = ticket(2, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        let removed = root.object_mut().delete_by_key("k1", removed_at).unwrap();
        assert_eq!(1, root.deregister_element(&removed));

        assert_eq!(1, root.get_element_map_size());
        assert!(root.find_by_created_at(&created_at).is_none());
    }

    #[test]
    fn registers_nested_object_descendants() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let profile_created_at = ticket(1, "a");
        let name_created_at = ticket(2, "a");
        let mut profile = CrdtObject::create(profile_created_at.clone());
        let name = primitive_str("yorkie", name_created_at.clone());

        profile.set("name", name.deepcopy(), name_created_at.clone());
        let profile = CrdtElement::object(profile);
        root.object_mut()
            .set("profile", profile.deepcopy(), profile_created_at.clone());
        let parent = root.root_element();
        root.register_element(&profile, Some(&parent));

        assert_eq!(3, root.get_element_map_size());
        assert_eq!("$.profile", root.create_path(&profile_created_at)?);
        assert_eq!("$.profile.name", root.create_path(&name_created_at)?);
        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, root.to_json());
        Ok(())
    }

    #[test]
    fn tracks_removed_elements_for_garbage_collection() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let removed_at = ticket(2, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        let removed = root.object_mut().delete_by_key("k1", removed_at).unwrap();
        root.register_removed_element(&removed);

        assert_eq!(1, root.get_garbage_element_set_size());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            removed.removed_at(),
            root.find_by_created_at(&created_at).unwrap().removed_at()
        );
        assert_eq!(2, root.stats().elements);
        assert_eq!(1, root.stats().gc_elements);
    }

    #[test]
    fn deepcopies_root_object_and_rebuilds_indexes() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        let copy = root.deepcopy();
        root.object_mut().delete_by_key("k1", ticket(2, "a"));

        assert_eq!(r#"{"k1":"k1"}"#, copy.to_json());
        assert_eq!("$.k1", copy.create_path(&created_at)?);
        assert_eq!(2, copy.get_element_map_size());
        Ok(())
    }

    #[test]
    fn tracks_array_dead_positions_for_garbage_collection() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let one_at = ticket(2, "a");
        let two_at = ticket(3, "a");
        let moved_at = ticket(4, "a");

        root.set_object_member(
            &TimeTicket::initial(),
            "items",
            CrdtElement::array(CrdtArray::create(array_at.clone())),
            array_at.clone(),
        )?;
        root.insert_array_element(
            &array_at,
            &TimeTicket::initial(),
            primitive_str("one", one_at.clone()),
            one_at.clone(),
        )?;
        root.insert_array_element(
            &array_at,
            &one_at,
            primitive_str("two", two_at.clone()),
            two_at.clone(),
        )?;

        root.move_array_element(&array_at, &two_at, &one_at, moved_at)?;

        assert_eq!(r#"{"items":["two","one"]}"#, root.to_json());
        assert_eq!(1, root.stats().gc_pairs);
        assert_eq!(1, root.get_garbage_len());

        let copy = root.deepcopy();
        assert_eq!(1, copy.stats().gc_pairs);
        assert_eq!(1, copy.get_garbage_len());
        assert_eq!("$.items.1", copy.create_path(&one_at)?);

        let mut vector = VersionVector::new();
        vector.set("a", 3);
        assert_eq!(0, root.garbage_collect(&vector));
        assert_eq!(1, root.get_garbage_len());

        vector.set("a", 4);
        assert_eq!(1, root.garbage_collect(&vector));
        assert_eq!(0, root.get_garbage_len());
        assert_eq!(0, root.stats().gc_pairs);
        assert_eq!(r#"{"items":["two","one"]}"#, root.to_json());
        assert_eq!("$.items.1", root.create_path(&one_at)?);
        Ok(())
    }

    #[test]
    fn deepcopies_array_position_nodes_for_following_insert() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let zero_at = ticket(2, "a");
        let one_at = ticket(3, "a");
        let two_at = ticket(4, "a");
        let moved_at = ticket(5, "a");
        let three_at = ticket(6, "a");

        create_integer_array(
            &mut root,
            &array_at,
            [(&zero_at, 0), (&one_at, 1), (&two_at, 2)],
        )?;
        root.move_array_element(&array_at, &TimeTicket::initial(), &two_at, moved_at.clone())?;

        assert_eq!(r#"{"items":[2,0,1]}"#, root.to_json());

        let mut copy = root.deepcopy();
        assert_eq!(root.to_json(), copy.to_json());
        assert_eq!(root.stats(), copy.stats());
        assert_eq!(root.doc_size(), copy.doc_size());
        assert_eq!(
            moved_at,
            copy.array_by_created_at(&array_at)
                .unwrap()
                .pos_created_at(&two_at)?
        );

        let copied_last_at = copy
            .array_by_created_at(&array_at)
            .unwrap()
            .get_last_created_at();
        copy.insert_array_element(
            &array_at,
            &copied_last_at,
            primitive_int(3, three_at.clone()),
            three_at.clone(),
        )?;

        assert_eq!(r#"{"items":[2,0,1]}"#, root.to_json());
        assert_eq!(r#"{"items":[2,0,1,3]}"#, copy.to_json());
        assert_eq!("$.items.0", copy.create_path(&two_at)?);
        assert_eq!("$.items.3", copy.create_path(&three_at)?);
        Ok(())
    }

    #[test]
    fn rebuilds_array_gc_indexes_from_copied_root_object() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let zero_at = ticket(2, "a");
        let one_at = ticket(3, "a");
        let two_at = ticket(4, "a");
        let moved_at = ticket(5, "a");
        let removed_at = ticket(6, "a");

        create_integer_array(
            &mut root,
            &array_at,
            [(&zero_at, 0), (&one_at, 1), (&two_at, 2)],
        )?;
        root.move_array_element(&array_at, &TimeTicket::initial(), &two_at, moved_at)?;
        root.remove_container_element(&array_at, &one_at, removed_at.clone())?;

        assert_eq!(r#"{"items":[2,0]}"#, root.to_json());
        assert_eq!(1, root.stats().gc_elements);
        assert_eq!(1, root.stats().gc_pairs);
        assert_eq!(2, root.get_garbage_len());

        let mut rebuilt = CrdtRoot::new(root.root_object.deepcopy());
        assert_eq!(root.to_json(), rebuilt.to_json());
        assert_eq!(root.stats(), rebuilt.stats());
        assert_eq!(root.doc_size(), rebuilt.doc_size());
        assert_eq!("$.items.0", rebuilt.create_path(&two_at)?);
        assert_eq!(
            Some(&removed_at),
            rebuilt.find_by_created_at(&one_at).unwrap().removed_at()
        );

        let mut vector = VersionVector::new();
        vector.set("a", 5);
        assert_eq!(1, rebuilt.garbage_collect(&vector));
        assert_eq!(1, rebuilt.stats().gc_elements);
        assert_eq!(0, rebuilt.stats().gc_pairs);
        assert_eq!(1, rebuilt.get_garbage_len());
        assert_eq!(r#"{"items":[2,0]}"#, rebuilt.to_json());

        vector.set("a", 6);
        assert_eq!(1, rebuilt.garbage_collect(&vector));
        assert_eq!(0, rebuilt.get_garbage_len());
        assert!(rebuilt.find_by_created_at(&one_at).is_none());
        assert_eq!(r#"{"items":[2,0]}"#, rebuilt.to_json());
        Ok(())
    }

    #[test]
    fn registers_text_members_and_text_gc_pairs() -> crate::Result<()> {
        let text_at = ticket(1, "a");
        let mut text = CrdtText::create(text_at.clone());
        text.edit_by_index(0, 0, "Hello World", None, ticket(2, "a"), None)?;

        let mut attrs = BTreeMap::new();
        attrs.insert("b".to_owned(), "true".to_owned());
        text.set_style_by_index(0, 5, attrs, ticket(3, "a"), None)?;
        text.remove_style_by_index(0, 5, &["b".to_owned()], ticket(4, "a"), None)?;
        text.edit_by_index(5, 11, "", None, ticket(5, "a"), None)?;

        assert_eq!(2, text.gc_pair_entries().len());

        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("message", CrdtElement::text(text))],
        ));

        assert_eq!(2, root.get_element_map_size());
        assert_eq!("$.message", root.create_path(&text_at)?);
        assert_eq!(r#"{"message":[{"val":"Hello"}]}"#, root.to_json());
        assert_eq!(
            "Hello",
            root.text_by_created_at(&text_at).unwrap().to_string()
        );
        assert_eq!(2, root.stats().gc_pairs);
        assert_eq!(2, root.get_garbage_len());

        let copy = root.deepcopy();
        assert_eq!(2, copy.stats().gc_pairs);
        assert_eq!(r#"{"message":[{"val":"Hello"}]}"#, copy.to_json());
        Ok(())
    }

    #[test]
    fn garbage_collects_text_internal_nodes() -> crate::Result<()> {
        let text_at = ticket(1, "a");
        let mut text = CrdtText::create(text_at.clone());
        text.edit_by_index(0, 0, "Hello World", None, ticket(2, "a"), None)?;
        text.edit_by_index(5, 10, "Yorkie", None, ticket(3, "a"), None)?;
        text.edit_by_index(0, 5, "", None, ticket(4, "a"), None)?;
        text.edit_by_index(6, 7, "", None, ticket(5, "a"), None)?;

        let mut root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("message", CrdtElement::text(text))],
        ));

        assert_eq!(
            "Yorkie",
            root.text_by_created_at(&text_at).unwrap().to_string()
        );
        assert_eq!(3, root.get_garbage_len());

        let mut vector = VersionVector::new();
        vector.set("a", 5);
        assert_eq!(3, root.garbage_collect(&vector));

        assert_eq!(0, root.get_garbage_len());
        assert_eq!(r#"{"message":[{"val":"Yorkie"}]}"#, root.to_json());
        assert_eq!(
            r#"[0:00:0:0 {} ""][3:a:0:0 {} "Yorkie"]"#,
            root.text_by_created_at(&text_at).unwrap().to_test_string()
        );
        Ok(())
    }

    #[test]
    fn finds_mutable_text_members_in_the_live_tree() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let text_at = ticket(1, "a");

        root.set_object_member(
            &TimeTicket::initial(),
            "message",
            CrdtElement::text(CrdtText::create(text_at.clone())),
            text_at.clone(),
        )?;

        root.text_by_created_at_mut(&text_at)
            .unwrap()
            .edit_by_index(0, 0, "Hi", None, ticket(2, "a"), None)?;

        assert_eq!(r#"{"message":[{"val":"Hi"}]}"#, root.to_json());
        assert_eq!("$.message", root.create_path(&text_at)?);
        Ok(())
    }

    #[test]
    fn registers_tree_members_and_tree_gc_pairs() -> crate::Result<()> {
        let tree_at = ticket(1, "a");
        let mut attrs = Rht::new();
        attrs.set("bold", "\"true\"", ticket(5, "a"));
        attrs.remove("bold", ticket(6, "a"));

        let mut removed_text = TreeNode::create_text(tree_node_id(4, 0), " removed");
        removed_text.remove(ticket(7, "a"));
        let paragraph = TreeNode::create_element(
            tree_node_id(2, 0),
            "p",
            Some(attrs),
            vec![
                TreeNode::create_text(tree_node_id(3, 0), "Hi"),
                removed_text,
            ],
        );
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(tree_at.clone(), 0),
                "root",
                None,
                vec![paragraph],
            ),
            tree_at.clone(),
        );

        let mut root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("body", CrdtElement::tree(tree))],
        ));

        assert_eq!(2, root.get_element_map_size());
        assert_eq!("$.body", root.create_path(&tree_at)?);
        assert_eq!(
            r#"{"body":{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"Hi"}]}]}}"#,
            root.to_json()
        );
        assert_eq!(2, root.stats().gc_pairs);
        assert_eq!(2, root.get_garbage_len());
        assert_eq!(
            r#"<root><p>Hi</p></root>"#,
            root.tree_by_created_at(&tree_at).unwrap().to_xml()
        );

        let copy = root.deepcopy();
        assert_eq!(2, copy.stats().gc_pairs);
        assert_eq!(root.to_json(), copy.to_json());

        let mut vector = VersionVector::new();
        vector.set("a", 6);
        assert_eq!(1, root.garbage_collect(&vector));
        assert_eq!(1, root.stats().gc_pairs);

        vector.set("a", 7);
        assert_eq!(1, root.garbage_collect(&vector));
        assert_eq!(0, root.get_garbage_len());
        assert_eq!(
            r#"{"body":{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"Hi"}]}]}}"#,
            root.to_json()
        );
        assert_eq!(3, root.tree_by_created_at(&tree_at).unwrap().node_map_len());
        Ok(())
    }

    #[test]
    fn reports_document_size_for_registered_elements() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        assert_eq!(TIME_TICKET_SIZE * 2, root.doc_size().live.meta);
        assert!(root.doc_size().live.data > 0);
    }

    fn create_integer_array<const N: usize>(
        root: &mut CrdtRoot,
        array_at: &TimeTicket,
        values: [(&TimeTicket, i32); N],
    ) -> crate::Result<()> {
        root.set_object_member(
            &TimeTicket::initial(),
            "items",
            CrdtElement::array(CrdtArray::create(array_at.clone())),
            array_at.clone(),
        )?;

        let mut prev_at = TimeTicket::initial();
        for (created_at, value) in values {
            root.insert_array_element(
                array_at,
                &prev_at,
                primitive_int(value, created_at.clone()),
                created_at.clone(),
            )?;
            prev_at = created_at.clone();
        }

        Ok(())
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn primitive_int(value: i32, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::Integer(value),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }

    fn tree_node_id(lamport: i64, offset: usize) -> TreeNodeId {
        TreeNodeId::new(ticket(lamport, "a"), offset)
    }
}
