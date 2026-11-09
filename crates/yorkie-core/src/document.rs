use crate::change::{
    Change, ChangeContext, ChangeExecutionResult, ChangeId, ChangePack, Checkpoint,
};
use crate::crdt::array::CrdtArray;
use crate::crdt::counter::CounterValue;
use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::crdt::rga_tree_split::RgaTreeSplitPosRange;
use crate::crdt::root::{CrdtRoot, DocSize};
use crate::crdt::split_links::drop_split_links_in_element;
use crate::crdt::text::CrdtText;
use crate::crdt::tree::{CrdtTree, TreePos};
use crate::event::{ChangeInfo, DocEvent, OperationInfo, Subscribers, SubscriptionId};
use crate::history::{
    History, HistoryMut, HistoryOperation, HistoryRef, MAX_UNDO_REDO_STACK_DEPTH,
};
use crate::json::{counter_operand, JsonEditRecorder, JsonEditRecorderRef, RecordedJsonValue};
use crate::operation::{
    AddOperation, ArraySetOperation, EditOperation, IncreaseOperation, MoveOperation, OpSource,
    Operation, RemoveOperation, SetOperation, StyleOperation, TreeEditOperation,
    TreeStyleOperation,
};
use crate::presence::{Presence, PresenceChange, PresenceProxy};
use crate::reissue::{reissue_operations, reissue_version_vector};
use crate::schema::validate_yorkie_ruleset;
use crate::{
    ActorId, JsonArray, JsonObject, JsonValue, Result, SchemaRule, TimeTicket, VersionVector,
    YorkieError, INITIAL_ACTOR_ID,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

mod options;

pub use options::DocumentOptions;

/// A local Yorkie document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    key: String,
    status: DocStatus,
    root: JsonObject,
    crdt_root: CrdtRoot,
    checkpoint: Checkpoint,
    change_id: ChangeId,
    local_changes: Vec<Change>,
    max_size_per_document: usize,
    schema_rules: Vec<SchemaRule>,
    presences: BTreeMap<ActorId, Presence>,
    online_clients: BTreeSet<ActorId>,
    subscribers: Subscribers,
    history: History,
    /// Whether this document took in state it did not mint itself: a
    /// snapshot or a remote change (JS `absorbedRemote`). Such a document is
    /// not "never synced" even when its checkpoint, status and version
    /// vector still look untouched (a snapshot pack can carry the initial
    /// checkpoint), so `set_actor_with_reissue` leaves it alone.
    absorbed_remote: bool,
    /// Whether this document has produced a local change that ran
    /// operations, so tickets naming its actor may already be in the
    /// document its key names (JS `mintedOperations`). It is never cleared:
    /// a push cannot be taken back.
    minted_operations: bool,
    /// `DocumentOptions::disable_gc` (JS `opts.disableGC`): garbage
    /// collection purges nothing.
    disable_gc: bool,
    /// Whether the attachment opted out of garbage collection (JS
    /// `disableGC`, set by the client on attach): a remote change then
    /// advances only the lamport clock and leaves the version vector at this
    /// document's own entry.
    attach_disable_gc: bool,
}

/// Document attachment state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocStatus {
    Detached,
    Attached,
    Removed,
}

impl Document {
    /// Creates a document with the given Yorkie resource key.
    pub fn new(key: impl Into<String>) -> Self {
        Self::create(key.into(), MAX_UNDO_REDO_STACK_DEPTH, false)
    }

    /// Creates a document with the given Yorkie resource key and options
    /// (JS `new Document(key, opts)`).
    ///
    /// Fails with `InvalidArgument` when `max_undo_depth` is `Some(0)`.
    pub fn with_options(key: impl Into<String>, options: DocumentOptions) -> Result<Self> {
        let max_undo_depth = options.max_undo_depth.unwrap_or(MAX_UNDO_REDO_STACK_DEPTH);
        if max_undo_depth < 1 {
            return Err(YorkieError::InvalidArgument(format!(
                "maxUndoDepth must be a positive integer: {max_undo_depth}"
            )));
        }
        Ok(Self::create(key.into(), max_undo_depth, options.disable_gc))
    }

    fn create(key: String, max_undo_depth: usize, disable_gc: bool) -> Self {
        Self {
            key,
            status: DocStatus::Detached,
            root: JsonObject::new(),
            crdt_root: CrdtRoot::create(),
            checkpoint: Checkpoint::initial(),
            change_id: ChangeId::initial(),
            local_changes: Vec::new(),
            max_size_per_document: 0,
            schema_rules: Vec::new(),
            presences: BTreeMap::new(),
            online_clients: BTreeSet::new(),
            subscribers: Subscribers::default(),
            history: History::new(max_undo_depth),
            absorbed_remote: false,
            minted_operations: false,
            disable_gc,
            attach_disable_gc: false,
        }
    }

    /// Returns this document's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns this document's current status.
    pub fn status(&self) -> DocStatus {
        self.status
    }

    /// Returns whether this document is attached to a client.
    pub fn is_attached(&self) -> bool {
        self.status == DocStatus::Attached
    }

    /// Returns the actor currently assigned to local changes.
    pub fn actor_id(&self) -> &ActorId {
        self.change_id.actor_id()
    }

    /// Assigns the actor for new local changes and pending local changes.
    ///
    /// Like JS `setActor` without `reissue`, it rewrites only each pending
    /// change's ID and each operation's `executed_at`. Elements and nodes an
    /// earlier edit created keep the actor they were created under; the
    /// client's attach re-issues them through `set_actor_with_reissue`.
    pub fn set_actor(&mut self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();

        // The self presence and the online entry follow the actor. This
        // matches JS `reissueActor` (a presence set before attach, rebuilt
        // under the new actor); plain JS `setActor` does not move presences.
        let prev = self.change_id.actor_id().clone();
        if prev != actor_id {
            if let Some(presence) = self.presences.remove(&prev) {
                self.presences.insert(actor_id.clone(), presence);
            }
            if self.online_clients.remove(&prev) {
                self.online_clients.insert(actor_id.clone());
            }
        }

        for change in &mut self.local_changes {
            change.set_actor(actor_id.clone());
        }

        self.change_id = self.change_id.set_actor(actor_id);
    }

    /// Assigns the actor like `set_actor`, but first re-issues every ticket a
    /// never-synced document minted under its previous actor to the given
    /// one, in the local changes and the root alike (JS `setActor` with
    /// `reissue`). It returns whether it re-issued. A document that has
    /// synced, has no local changes or already has the actor falls back to
    /// `set_actor`.
    ///
    /// Only the client's attach calls it: whether the re-issue is sound
    /// depends on what else the actor has minted under the document's key,
    /// which only the client knows. A position or element ID taken from the
    /// document before the call no longer resolves after a re-issue, and a
    /// re-issue clears the undo and redo stacks, as JS does.
    ///
    /// An error leaves the document as it was.
    #[doc(hidden)]
    pub fn set_actor_with_reissue(&mut self, actor_id: impl Into<ActorId>) -> Result<bool> {
        let actor_id = actor_id.into();
        if self.reissue_actor(&actor_id)? {
            return Ok(true);
        }

        self.set_actor(actor_id);
        Ok(false)
    }

    /// Re-issues the tickets of a never-synced document to the given actor
    /// and returns `true`, or returns `false` and changes nothing when the
    /// document has synced, has no local changes or already has the actor
    /// (JS `reissueActor`).
    ///
    /// The local changes go through `reissue_operations`, and the root and
    /// presences are rebuilt by replaying them on a fresh root: a
    /// never-synced root is exactly the initial root plus its local changes,
    /// and the replay is the computation the server performs on the pushed
    /// changes. Everything is built aside and assigned at the end, so an
    /// error leaves the document untouched. As in JS, the replay publishes
    /// no event.
    fn reissue_actor(&mut self, actor_id: &ActorId) -> Result<bool> {
        let prev = self.change_id.actor_id().clone();
        if prev == *actor_id || self.local_changes.is_empty() || !self.never_synced() {
            return Ok(false);
        }

        let changes = self
            .local_changes
            .iter()
            .map(|change| {
                let id = change.id();
                let id = id
                    .set_actor(actor_id.clone())
                    .set_version_vector(reissue_version_vector(
                        id.version_vector(),
                        &prev,
                        actor_id,
                    ));
                let operations = reissue_operations(change.operations(), &prev, actor_id)?;
                Ok(change.with_id_and_operations(id, operations))
            })
            .collect::<Result<Vec<_>>>()?;

        // Each rebuilt change runs with the source it first ran with: an
        // undo or a redo ran as `UndoRedo`, which the Set and Remove skip
        // guards consult, so replaying it as `Local` could run an operation
        // that never ran.
        let mut crdt_root = CrdtRoot::create();
        let mut presences = BTreeMap::new();
        for change in &changes {
            change.execute(&mut crdt_root, change.replay_source())?;
            change.apply_presence_change(&mut presences);
        }
        let root = crdt_object_to_json_object(crdt_root.object())?;

        let mut online_clients = self.online_clients.clone();
        if online_clients.remove(&prev) {
            online_clients.insert(actor_id.clone());
        }

        self.local_changes = changes;
        self.crdt_root = crdt_root;
        self.root = root;
        self.presences = presences;
        self.online_clients = online_clients;
        self.change_id = self
            .change_id
            .set_actor(actor_id.clone())
            .set_version_vector(reissue_version_vector(
                self.change_id.version_vector(),
                &prev,
                actor_id,
            ));

        // The undo and redo stacks hold the old tickets, and cannot be
        // re-issued the way the local changes are: a reverse operation has
        // no `executed_at` until the undo runs it, which the wire projection
        // rejects. They are dropped, the state `apply_snapshot` leaves them
        // in too.
        self.clear_history();
        Ok(true)
    }

    /// Returns whether this document has produced a local change that ran
    /// operations, so tickets naming its actor may already be in the
    /// document its key names (JS `hasMintedOperations`). The client reads
    /// it to decide whether a re-issue, which starts the actor's lamports
    /// back at 1, can still be sound for that key.
    #[doc(hidden)]
    pub fn has_minted_operations(&self) -> bool {
        self.minted_operations
    }

    /// Returns whether every ticket naming this document's actor was minted
    /// by a local change still in `local_changes`: nothing has been pushed
    /// and nothing pulled (JS `neverSynced`).
    fn never_synced(&self) -> bool {
        if self.absorbed_remote
            || self.status != DocStatus::Detached
            || self.checkpoint != Checkpoint::initial()
        {
            return false;
        }

        let actor_id = self.change_id.actor_id();
        self.change_id
            .version_vector()
            .iter()
            .all(|(id, _)| id == actor_id)
    }

    /// Applies a document status transition.
    pub fn apply_status(&mut self, status: DocStatus) {
        let actor_id = self.change_id.actor_id().clone();
        let prev = self.presence_state(&actor_id, self.status == DocStatus::Attached);
        self.status = status;

        if status == DocStatus::Detached {
            // A detached document no longer owns a presence; drop it so a
            // later attach starts from an empty one.
            self.presences.remove(self.change_id.actor_id());
            self.set_actor(INITIAL_ACTOR_ID);
        }

        if let Some(event) = self.reconcile_presence(&actor_id, prev) {
            self.publish(event);
        }
        self.publish(DocEvent::StatusChanged(status));
    }

    /// Updates the document root.
    pub fn update<F>(&mut self, update_fn: F) -> Result<()>
    where
        F: FnOnce(&mut JsonObject) -> Result<()>,
    {
        self.update_with_presence(|root, _| update_fn(root))
    }

    /// Updates the document root and the local presence in one change
    /// (JS `doc.update((root, presence) => ...)`).
    pub fn update_with_presence<F>(&mut self, update_fn: F) -> Result<()>
    where
        F: FnOnce(&mut JsonObject, &mut PresenceProxy) -> Result<()>,
    {
        if self.status == DocStatus::Removed {
            return Err(YorkieError::DocumentRemoved(self.key.clone()));
        }

        let mut next_root = self.root.clone();
        let recorder = Arc::new(Mutex::new(DocumentEditRecorder::new(
            self.change_id.clone(),
        )));
        let recorder_ref: JsonEditRecorderRef = recorder.clone();
        next_root.attach_recorder(recorder_ref);

        let mut proxy = PresenceProxy::new(
            self.presences
                .get(self.change_id.actor_id())
                .cloned()
                .unwrap_or_default(),
        );
        update_fn(&mut next_root, &mut proxy)?;

        let mut context = recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .context
            .clone();
        let (presence_change, reverse_presence) = proxy.into_parts();
        context.set_presence_change(presence_change);
        if context.operations().is_empty() {
            collect_object_changes(
                &mut context,
                &self.crdt_root,
                &TimeTicket::initial(),
                &self.root,
                &next_root,
            )?;
        }

        if context.has_change() {
            let mut change = context.to_change();
            let actor_id = self.change_id.actor_id().clone();
            let prev = self.presence_state(&actor_id, self.status == DocStatus::Attached);
            let (staged, executed) = self.stage(&mut change, |change, root| {
                let executed = change.execute(root, OpSource::Local)?;
                self.validate_local_change(root)?;
                Ok(executed)
            })?;

            // Nothing below fails: the change is committed from here on.
            self.commit_staged(staged);
            change.apply_presence_change(&mut self.presences);

            // An array set replaces the element under a new creation time, and
            // the history may still name the old one.
            for operation in change.operations() {
                if let Operation::ArraySet(operation) = operation {
                    self.history.reconcile_created_at(
                        operation.created_at(),
                        operation.value().created_at(),
                    );
                }
            }
            let entry = HistoryOperation::entry(executed.reverse_ops, reverse_presence);
            if !entry.is_empty() {
                self.history.push_undo(entry);
            }
            // As in JS, a new local edit clears the redo stack; a change of
            // presence alone does not.
            if !executed.op_infos.is_empty() {
                self.history.clear_redo();
            }

            // As in JS, the change event is gated on the operations that ran,
            // not on the op infos they produced.
            let change_event = (!executed.operations.is_empty())
                .then(|| DocEvent::LocalChange(change_info(&change, executed.op_infos)));
            let presence_event = change
                .presence_change()
                .and_then(|_| self.reconcile_presence(&actor_id, prev));

            self.local_changes.push(change);
            self.minted_operations |= !executed.operations.is_empty();
            self.change_id = context.next_id();
            for event in change_event.into_iter().chain(presence_event) {
                self.publish(event);
            }
            return Ok(());
        }

        Ok(())
    }

    /// Runs `execute` for a local change against a copy of the CRDT root and
    /// returns the copy, with its public JSON view, for `commit_staged`.
    ///
    /// JS runs a local change on its clone root first and drops the clone if
    /// that fails or validation rejects it, so the root never holds part of
    /// a change. Here the copy plays the clone's part: a change that fails
    /// partway, or that `execute` rejects after running it, is dropped with
    /// the copy, and this document is left as it was. Nothing reached the
    /// root, so unlike a failure of the JS root pass there are no issued
    /// tickets to burn: the change ID is not advanced, and the next change
    /// reuses it.
    ///
    /// A change without operations touches no CRDT element and is run
    /// without a copy.
    fn stage(
        &self,
        change: &mut Change,
        execute: impl FnOnce(&mut Change, &mut CrdtRoot) -> Result<ChangeExecutionResult>,
    ) -> Result<(Option<StagedRoot>, ChangeExecutionResult)> {
        if !change.has_operations() {
            return Ok((None, ChangeExecutionResult::default()));
        }

        let mut crdt_root = self.crdt_root.clone();
        let executed = execute(change, &mut crdt_root)?;
        let root = crdt_object_to_json_object(crdt_root.object())?;
        Ok((Some(StagedRoot { crdt_root, root }), executed))
    }

    /// Checks a root a local change ran on against this document's schema
    /// rules and then its maximum size, as JS `update` checks its clone
    /// root before committing the change.
    ///
    /// Only local updates are checked, as in JS: a remote change and an undo
    /// or redo are applied even when they break a rule or the limit.
    fn validate_local_change(&self, root: &CrdtRoot) -> Result<()> {
        if !self.schema_rules.is_empty() {
            let errors = validate_yorkie_ruleset(root.object(), &self.schema_rules)?;
            if !errors.is_empty() {
                return Err(YorkieError::SchemaValidationFailed(
                    errors.into_iter().map(|error| error.message).collect(),
                ));
            }
        }

        let size = root.doc_size().total();
        if self.max_size_per_document > 0 && self.max_size_per_document < size {
            return Err(YorkieError::DocumentSizeExceedsLimit {
                size,
                limit: self.max_size_per_document,
            });
        }
        Ok(())
    }

    /// Replaces the root with one `stage` produced.
    fn commit_staged(&mut self, staged: Option<StagedRoot>) {
        if let Some(StagedRoot { crdt_root, root }) = staged {
            self.crdt_root = crdt_root;
            self.root = root;
        }
    }

    /// Returns a read-only view of this document's undo and redo history
    /// (JS `doc.history.canUndo`/`canRedo`).
    pub fn history(&self) -> HistoryRef<'_> {
        HistoryRef::new(self)
    }

    /// Returns the handle that undoes and redoes local changes (JS
    /// `doc.history`).
    ///
    /// JS refuses an undo or a redo called from inside an `update` callback.
    /// Here the callback cannot reach the document at all: it holds
    /// `&mut Document` only through `update`, so the borrow checker rejects
    /// the call.
    ///
    /// ```compile_fail,E0499
    /// let mut doc = yorkie_core::Document::new("doc");
    /// let handle = &mut doc;
    /// handle.update(|_root| doc.history_mut().undo()).unwrap();
    /// ```
    pub fn history_mut(&mut self) -> HistoryMut<'_> {
        HistoryMut::new(self)
    }

    /// Drops both the undo and the redo stack (JS `clearHistory`), so that
    /// setup changes, such as an attach's initial root, cannot be undone. The
    /// document, its local changes and its presence stay as they are.
    pub fn clear_history(&mut self) {
        self.history.clear_redo();
        self.history.clear_undo();
    }

    pub(crate) fn history_stacks(&self) -> &History {
        &self.history
    }

    /// Pops the latest undo (or redo) entry and applies it as a new local
    /// change (JS `executeUndoRedo`).
    pub(crate) fn execute_undo_redo(&mut self, is_undo: bool) -> Result<()> {
        let entry = if is_undo {
            self.history.pop_undo()
        } else {
            self.history.pop_redo()
        };
        let Some(entry) = entry else {
            return Ok(());
        };
        // As in JS, a failed undo or redo drops the popped entry.
        //
        // Applied to the stacks only once the change has run, so a failed
        // undo or redo leaves the other entries as they were. They name only
        // the other entries, so deferring them changes nothing for this one.
        let mut reconciles = Vec::new();

        let actor_id = self.change_id.actor_id().clone();
        let mut context = ChangeContext::create(self.change_id.clone(), None);
        let mut proxy =
            PresenceProxy::new(self.presences.get(&actor_id).cloned().unwrap_or_default());
        for history_op in entry {
            let mut operation = match history_op {
                HistoryOperation::Presence(values) => {
                    proxy.restore(values);
                    continue;
                }
                HistoryOperation::Operation(operation) => *operation,
            };

            let ticket = context.issue_time_ticket();
            operation.set_executed_at(ticket.clone());
            // A Set, Add or ArraySet reverse carries a deep copy of the value
            // it restores, with the split-sibling links of the tree it was
            // taken from. Every other replica decodes this operation through
            // `drop_split_links_in_element`, so drop them here too, or this
            // replica alone keeps them and the two disagree from the next
            // same-boundary split on (JS `executeUndoRedo`).
            match &mut operation {
                Operation::Set(operation) => drop_split_links_in_element(operation.value_mut()),
                Operation::Add(operation) => drop_split_links_in_element(operation.value_mut()),
                Operation::ArraySet(operation) => {
                    drop_split_links_in_element(operation.value_mut())
                }
                _ => {}
            }
            // An array set and an add that brings a removed element back both
            // put the element under a new creation time, and the rest of the
            // history may still name the old one.
            match &mut operation {
                Operation::ArraySet(operation) => {
                    let prev = operation.created_at().clone();
                    operation.value_mut().set_created_at(ticket.clone());
                    reconciles.push((prev, ticket.clone()));
                }
                Operation::Add(operation) => {
                    let prev = operation.value().created_at().clone();
                    operation.value_mut().set_created_at(ticket.clone());
                    reconciles.push((prev, ticket.clone()));
                }
                _ => {}
            }
            context.push(operation);
        }

        let (presence_change, reverse_presence) = proxy.into_parts();
        context.set_presence_change(presence_change);
        let mut change = context.to_change();
        change.mark_as_undo_redo();
        let prev = self.presence_state(&actor_id, self.status == DocStatus::Attached);
        let (staged, executed) =
            self.stage(&mut change, |change, root| change.execute_undo_redo(root))?;

        // Nothing below fails: the change is committed from here on.
        self.commit_staged(staged);
        for (prev, curr) in &reconciles {
            self.history.reconcile_created_at(prev, curr);
        }
        change.apply_presence_change(&mut self.presences);

        let entry = HistoryOperation::entry(executed.reverse_ops, reverse_presence);
        if !entry.is_empty() {
            if is_undo {
                self.history.push_redo(entry);
            } else {
                self.history.push_undo(entry);
            }
        }

        // As in JS, an undo whose operations all skipped (their target was
        // removed meanwhile) and that restores no presence is not sent.
        if change.presence_change().is_none() && executed.operations.is_empty() {
            return Ok(());
        }

        let change_event = (!executed.operations.is_empty())
            .then(|| DocEvent::LocalChange(change_info(&change, executed.op_infos)));
        let presence_event = change
            .presence_change()
            .and_then(|_| self.reconcile_presence(&actor_id, prev));

        self.local_changes.push(change);
        self.minted_operations |= !executed.operations.is_empty();
        self.change_id = context.next_id();
        for event in change_event.into_iter().chain(presence_event) {
            self.publish(event);
        }
        Ok(())
    }

    /// Returns the undo stack (JS `getUndoStackForTest`).
    #[cfg(test)]
    pub(crate) fn undo_stack_for_test(&self) -> &[Vec<HistoryOperation>] {
        self.history.undo_stack()
    }

    /// Returns the redo stack (JS `getRedoStackForTest`).
    #[cfg(test)]
    pub(crate) fn redo_stack_for_test(&self) -> &[Vec<HistoryOperation>] {
        self.history.redo_stack()
    }

    /// Returns the CRDT root, for tests that read node identities.
    #[cfg(test)]
    pub(crate) fn crdt_root_for_test(&self) -> &CrdtRoot {
        &self.crdt_root
    }

    /// Returns an immutable view of the document root.
    pub fn get_root(&self) -> &JsonObject {
        &self.root
    }

    /// Returns this document's checkpoint.
    pub fn checkpoint(&self) -> Checkpoint {
        self.checkpoint
    }

    /// Returns whether this document has local changes waiting to be synced.
    pub fn has_local_changes(&self) -> bool {
        !self.local_changes.is_empty()
    }

    /// Returns the version vector of this document: the latest lamport seen
    /// from each actor (JS `getVersionVector`).
    #[doc(hidden)]
    pub fn version_vector(&self) -> &VersionVector {
        self.change_id.version_vector()
    }

    /// Returns how many removed elements and nodes this document still
    /// holds, an element counted with its descendants (JS `getGarbageLen`).
    #[doc(hidden)]
    pub fn garbage_len(&self) -> usize {
        self.crdt_root.get_garbage_len()
    }

    /// Purges the tombstones `vector` covers and returns how many it purged
    /// (JS `garbageCollect`). `apply_change_pack` runs it with the server's
    /// minimum version vector; calling it with a vector the server did not
    /// send can purge a tombstone a peer's change still anchors on. A
    /// document created with `DocumentOptions::disable_gc` purges nothing
    /// and returns 0.
    #[doc(hidden)]
    pub fn garbage_collect(&mut self, vector: &VersionVector) -> usize {
        if self.disable_gc {
            return 0;
        }
        self.crdt_root.garbage_collect(vector)
    }

    /// Records whether the attachment opted out of garbage collection (JS
    /// `setDisableGC`). The client sets it on every attach, before applying
    /// the attach response, so the remote changes that response carries
    /// already advance only the lamport clock; a detach leaves it as it is.
    #[doc(hidden)]
    pub fn set_disable_gc(&mut self, disable_gc: bool) {
        self.attach_disable_gc = disable_gc;
    }

    /// Returns the size of this document (JS `getDocSize`).
    pub fn doc_size(&self) -> DocSize {
        self.crdt_root.doc_size()
    }

    /// Returns the maximum size limit of this document in bytes.
    pub fn max_size_per_document(&self) -> usize {
        self.max_size_per_document
    }

    /// Sets the maximum size limit of this document in bytes. A local update
    /// that leaves the document larger fails with
    /// `YorkieError::DocumentSizeExceedsLimit`; zero means no limit.
    pub fn set_max_size_per_document(&mut self, size: usize) {
        self.max_size_per_document = size;
    }

    /// Returns the schema rules associated with this document.
    pub fn schema_rules(&self) -> &[SchemaRule] {
        &self.schema_rules
    }

    /// Sets the schema rules associated with this document. A local update
    /// that leaves the document breaking them fails with
    /// `YorkieError::SchemaValidationFailed`.
    pub fn set_schema_rules(&mut self, rules: Vec<SchemaRule>) {
        self.schema_rules = rules;
    }

    /// Creates a pack of local changes to send to the remote.
    pub fn create_change_pack(&self) -> ChangePack {
        let changes = self.local_changes.clone();
        let checkpoint = self.checkpoint.increase_client_seq(changes.len() as u32);
        ChangePack::create(
            self.key.clone(),
            checkpoint,
            false,
            changes,
            Some(self.change_id.version_vector().clone()),
            None,
        )
    }

    /// Applies the given change pack to this document.
    pub fn apply_change_pack(&mut self, pack: &ChangePack) -> Result<()> {
        if pack.has_snapshot() {
            self.apply_snapshot(pack)?;
        } else {
            self.apply_changes(pack.changes(), OpSource::Remote)?;
            self.remove_pushed_local_changes(pack.checkpoint().client_seq());
        }

        self.checkpoint = self.checkpoint.forward(pack.checkpoint());

        // As in JS, collect with the server's minimum version vector once the
        // pack has applied. A snapshot pack does not collect: its vector
        // describes the snapshot, and the replayed local changes are newer.
        if !pack.has_snapshot() {
            if let Some(vector) = pack.version_vector() {
                self.garbage_collect(vector);
            }
        }

        if pack.is_removed() {
            self.apply_status(DocStatus::Removed);
        }
        Ok(())
    }

    /// Takes the given pack as a push acknowledgement only (JS
    /// `acknowledgePushedChanges`): it drops the local changes the server
    /// confirmed and forwards the client seq of the checkpoint, leaving the
    /// server seq where it was so a later pull fetches what this pack did not
    /// carry. It is for a reply whose remote state the client must not take,
    /// such as the reply to a push-only request.
    ///
    /// That reply still carries the server's minimum version vector, and it
    /// must not reach garbage collection: the client has not pulled the
    /// remote changes the vector already accounts for, so collecting with it
    /// purges tombstones those changes anchor on.
    ///
    /// The removal flag is taken: it describes the document, not the skipped
    /// content.
    #[doc(hidden)]
    pub fn acknowledge_pushed_changes(&mut self, pack: &ChangePack) {
        let client_seq = pack.checkpoint().client_seq();
        self.remove_pushed_local_changes(client_seq);
        self.checkpoint = self
            .checkpoint
            .forward(Checkpoint::new(self.checkpoint.server_seq(), client_seq));

        if pack.is_removed() {
            self.apply_status(DocStatus::Removed);
        }
    }

    /// Returns the presence of the current client. Empty unless attached,
    /// as in JS `getMyPresence`.
    pub fn my_presence(&self) -> Presence {
        if self.status != DocStatus::Attached {
            return Presence::new();
        }
        self.presences
            .get(self.change_id.actor_id())
            .cloned()
            .unwrap_or_default()
    }

    /// Returns the presence of the given client: self, or an online client
    /// that has a presence (JS `getPresence`).
    pub fn presence(&self, actor_id: &ActorId) -> Option<Presence> {
        if actor_id == self.change_id.actor_id() {
            return Some(self.my_presence());
        }
        if !self.online_clients.contains(actor_id) {
            return None;
        }
        self.presences.get(actor_id).cloned()
    }

    /// Returns the presences of online clients other than this one, ordered
    /// by actor ID (JS `getOthersPresences`).
    pub fn others_presences(&self) -> Vec<(ActorId, Presence)> {
        let me = self.change_id.actor_id();
        self.online_clients
            .iter()
            .filter(|actor_id| *actor_id != me)
            .filter_map(|actor_id| {
                self.presences
                    .get(actor_id)
                    .map(|presence| (actor_id.clone(), presence.clone()))
            })
            .collect()
    }

    /// Returns this client and the online clients that have a presence,
    /// ordered by actor ID (JS `getPresences`).
    ///
    /// Deliberate divergence: JS lists self first, then online clients in
    /// insertion order. Rust sorts everything by actor ID and lists self once.
    pub fn presences(&self) -> Vec<(ActorId, Presence)> {
        let mut all = self.others_presences();
        all.push((self.change_id.actor_id().clone(), self.my_presence()));
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    }

    /// Replaces the set of online clients.
    #[doc(hidden)]
    pub fn set_online_clients(&mut self, actor_ids: impl IntoIterator<Item = ActorId>) {
        self.online_clients = actor_ids.into_iter().collect();
    }

    /// Marks a client as online.
    #[doc(hidden)]
    pub fn add_online_client(&mut self, actor_id: ActorId) {
        self.online_clients.insert(actor_id);
    }

    /// Marks a client as offline.
    #[doc(hidden)]
    pub fn remove_online_client(&mut self, actor_id: &ActorId) {
        self.online_clients.remove(actor_id);
    }

    /// Returns whether a presence is stored for the given client.
    #[doc(hidden)]
    pub fn has_presence(&self, actor_id: &ActorId) -> bool {
        self.presences.contains_key(actor_id)
    }

    /// Clears the set of online clients (JS `resetOnlineClients`).
    #[doc(hidden)]
    pub fn reset_online_clients(&mut self) {
        self.online_clients.clear();
    }

    /// Replaces the online clients with the ones a watch stream reported when
    /// it opened, skipping this client, and publishes `Initialized` (JS
    /// `applyWatchInit`).
    ///
    /// Presences of clients that are no longer online are kept, as in JS: a
    /// peer whose watch stream reconnects is then reported as watched again.
    #[doc(hidden)]
    pub fn apply_watch_init(&mut self, client_ids: impl IntoIterator<Item = ActorId>) {
        let me = self.change_id.actor_id().clone();
        self.online_clients = client_ids.into_iter().filter(|id| *id != me).collect();
        let presences = self.presences();
        self.publish(DocEvent::Initialized(presences));
    }

    /// Marks a client that started watching as online and publishes `Watched`
    /// once its presence is known (JS `applyDocEvent(DOCUMENT_WATCHED)`).
    #[doc(hidden)]
    pub fn apply_watched(&mut self, publisher: ActorId) {
        let was_online = self.online_clients.contains(&publisher);
        if was_online && self.has_presence(&publisher) {
            return;
        }
        let prev = self.presence_state(&publisher, was_online);
        self.online_clients.insert(publisher.clone());
        if let Some(event) = self.reconcile_presence(&publisher, prev) {
            self.publish(event);
        }
    }

    /// Marks a client that stopped watching as offline and publishes
    /// `Unwatched` if it was visible (JS `applyDocEvent(DOCUMENT_UNWATCHED)`).
    /// Its presence is kept, for the same reason as in `apply_watch_init`.
    #[doc(hidden)]
    pub fn apply_unwatched(&mut self, publisher: &ActorId) {
        let prev = self.presence_state(publisher, self.online_clients.contains(publisher));
        self.online_clients.remove(publisher);
        if let Some(event) = self.reconcile_presence(publisher, prev) {
            self.publish(event);
        }
    }

    /// Serializes the document root with object keys sorted lexicographically.
    pub fn to_sorted_json(&self) -> String {
        self.root.to_sorted_json()
    }

    /// Registers a callback for every event of this document and returns its
    /// subscription ID (JS `subscribe('all', cb)`; the JS default
    /// `subscribe(cb)` delivers only snapshot and change events, which callers
    /// select here by matching on the event).
    ///
    /// Callbacks run synchronously, inside the call that produced the event,
    /// while the caller holds `&mut Document`. When the document is shared
    /// behind a mutex, the lock is held during the callback: locking the same
    /// document again from inside it deadlocks.
    pub fn subscribe(
        &mut self,
        callback: impl FnMut(&DocEvent) + Send + 'static,
    ) -> SubscriptionId {
        self.subscribers.add(None, Box::new(callback))
    }

    /// Registers a callback for the local and remote changes that touch
    /// `path` or an element under it (JS `subscribe(targetPath, cb)`). Each
    /// delivered event carries only those operations; a change without any is
    /// not delivered. Paths match on whole segments, so `$.todos` covers
    /// `$.todos.0` but not `$.todos2`. Callbacks run as in `subscribe`.
    pub fn subscribe_path(
        &mut self,
        path: &str,
        callback: impl FnMut(&DocEvent) + Send + 'static,
    ) -> SubscriptionId {
        self.subscribers
            .add(Some(path.to_owned()), Box::new(callback))
    }

    /// Removes a subscription. Unknown IDs are ignored.
    pub fn unsubscribe(&mut self, id: SubscriptionId) {
        self.subscribers.remove(id);
    }

    /// Delivers an event to this document's subscribers (JS `publish`).
    #[doc(hidden)]
    pub fn publish(&mut self, event: DocEvent) {
        self.subscribers.publish(&event);
    }

    fn presence_state(&self, actor_id: &ActorId, was_online: bool) -> PresenceState {
        PresenceState {
            presence: self.presences.get(actor_id).cloned(),
            was_online,
        }
    }

    /// Compares a client's presence and online state with `prev` and returns
    /// the event for the transition (JS `reconcilePresence`). Self is online
    /// while attached; others while in the online set.
    fn reconcile_presence(&self, actor_id: &ActorId, prev: PresenceState) -> Option<DocEvent> {
        let is_self = actor_id == self.change_id.actor_id();
        let is_online = if is_self {
            self.status == DocStatus::Attached
        } else {
            self.online_clients.contains(actor_id)
        };
        let was_ready = prev.was_online && prev.presence.is_some();

        let Some(presence) = self.presences.get(actor_id).filter(|_| is_online) else {
            return match prev.presence {
                Some(presence) if was_ready && !is_self => {
                    Some(DocEvent::Unwatched(actor_id.clone(), presence))
                }
                _ => None,
            };
        };

        let presence = presence.clone();
        if !was_ready && !is_self {
            return Some(DocEvent::Watched(actor_id.clone(), presence));
        }
        Some(DocEvent::PresenceChanged(actor_id.clone(), presence))
    }

    fn apply_changes(&mut self, changes: &[Change], source: OpSource) -> Result<()> {
        for change in changes {
            let actor_id = change.id().actor_id().clone();
            let prev = change
                .presence_change()
                .map(|_| self.presence_state(&actor_id, self.online_clients.contains(&actor_id)));
            // As in JS `applyChangeInternal`, a remote change marks the
            // document as absorbed before it runs; a replayed local change
            // does not (`apply_snapshot` replays them through here).
            if source == OpSource::Remote {
                self.absorbed_remote = true;
            }
            // A replayed local change runs with the source it first ran with.
            let execute_source = match source {
                OpSource::Local => change.replay_source(),
                source => source,
            };
            let executed = change.execute(&mut self.crdt_root, execute_source)?;
            change.apply_presence_change(&mut self.presences);
            self.change_id = if self.attach_disable_gc {
                self.change_id.sync_lamport(change.id())
            } else {
                self.change_id.sync_clocks(change.id())
            };
            // Undo text edits recorded before this change still name the
            // positions they had then; move them past what this edit changed.
            // The change has already run, so a range that does not normalize
            // only skips the reconcile instead of failing the apply.
            if self.history.has_undo() || self.history.has_redo() {
                for operation in &executed.operations {
                    let Operation::Edit(operation) = operation else {
                        continue;
                    };
                    let Ok((from, to)) = operation.normalize_pos(&self.crdt_root) else {
                        continue;
                    };
                    self.history.reconcile_text_edit(
                        operation.parent_created_at(),
                        from,
                        to,
                        operation.content_len(),
                    );
                }
            }

            if !executed.operations.is_empty() {
                let info = change_info(change, executed.op_infos);
                self.publish(if source == OpSource::Remote {
                    DocEvent::RemoteChange(info)
                } else {
                    DocEvent::LocalChange(info)
                });
            }
            if let Some(prev) = prev {
                if matches!(change.presence_change(), Some(PresenceChange::Clear)) {
                    self.online_clients.remove(&actor_id);
                }
                if let Some(event) = self.reconcile_presence(&actor_id, prev) {
                    self.publish(event);
                }
            }
        }

        self.root = crdt_object_to_json_object(self.crdt_root.object())?;
        Ok(())
    }

    fn apply_snapshot(&mut self, pack: &ChangePack) -> Result<()> {
        let snapshot_root = pack
            .snapshot_root()
            .ok_or(YorkieError::UnsupportedSnapshot)?
            .clone();

        self.crdt_root = CrdtRoot::new(snapshot_root);
        self.presences = pack.snapshot_presences().clone();
        self.absorbed_remote = true;
        self.root = crdt_object_to_json_object(self.crdt_root.object())?;

        if let Some(version_vector) = pack.version_vector() {
            self.change_id = self
                .change_id
                .set_clocks(version_vector.max_lamport(), version_vector.clone());
        }

        self.remove_pushed_local_changes(pack.checkpoint().client_seq());
        let local_changes = self.local_changes.clone();
        self.apply_changes(&local_changes, OpSource::Local)?;
        // The history names elements and positions of the root the snapshot
        // replaced, as in JS `applySnapshot`.
        self.clear_history();
        self.publish(DocEvent::Snapshot);
        Ok(())
    }

    fn remove_pushed_local_changes(&mut self, client_seq: u32) {
        while self
            .local_changes
            .first()
            .map(|change| change.id().client_seq() <= client_seq)
            .unwrap_or(false)
        {
            self.local_changes.remove(0);
        }
    }
}

/// A client's presence and online state before a change, for
/// `Document::reconcile_presence`.
struct PresenceState {
    presence: Option<Presence>,
    was_online: bool,
}

/// A copy of the CRDT root a local change ran on, and its public JSON view,
/// waiting to replace the document's (see `Document::stage`).
struct StagedRoot {
    crdt_root: CrdtRoot,
    root: JsonObject,
}

fn change_info(change: &Change, op_infos: Vec<crate::operation::OpInfo>) -> ChangeInfo {
    ChangeInfo {
        actor: change.id().actor_id().clone(),
        message: change.message().map(str::to_owned),
        operations: op_infos.into_iter().map(OperationInfo::from).collect(),
    }
}

#[derive(Debug, Clone)]
struct DocumentEditRecorder {
    context: ChangeContext,
}

impl DocumentEditRecorder {
    fn new(change_id: ChangeId) -> Self {
        Self {
            context: ChangeContext::create(change_id, None),
        }
    }

    fn record_new_value(&mut self, value: &JsonValue) -> Result<(TimeTicket, CrdtElement)> {
        let created_at = self.context.issue_time_ticket();
        let element = json_value_to_crdt_element(value, created_at.clone(), &mut self.context)?;
        Ok((created_at, element))
    }
}

impl JsonEditRecorder for DocumentEditRecorder {
    fn record_object_set(
        &mut self,
        parent_created_at: &TimeTicket,
        key: &str,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue> {
        let (created_at, element) = self.record_new_value(value)?;
        self.context.push(Operation::Set(SetOperation::create(
            key.to_owned(),
            element.deepcopy(),
            parent_created_at.clone(),
            Some(created_at.clone()),
        )));

        recorded_json_value_from_crdt_element(&element)
    }

    fn record_object_remove(
        &mut self,
        parent_created_at: &TimeTicket,
        _key: &str,
        created_at: &TimeTicket,
    ) {
        let executed_at = self.context.issue_time_ticket();
        self.context.push(Operation::Remove(RemoveOperation::new(
            parent_created_at.clone(),
            created_at.clone(),
            Some(executed_at),
        )));
    }

    fn record_array_insert(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue> {
        let (created_at, element) = self.record_new_value(value)?;
        self.context.push(Operation::Add(AddOperation::create(
            parent_created_at.clone(),
            prev_created_at.clone(),
            element.deepcopy(),
            Some(created_at.clone()),
        )));

        recorded_json_value_from_crdt_element(&element)
    }

    fn record_array_set(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue> {
        let (executed_at, element) = self.record_new_value(value)?;
        self.context
            .push(Operation::ArraySet(ArraySetOperation::create(
                parent_created_at.clone(),
                created_at.clone(),
                element.deepcopy(),
                Some(executed_at.clone()),
            )));

        recorded_json_value_from_crdt_element(&element)
    }

    fn record_array_remove(&mut self, parent_created_at: &TimeTicket, created_at: &TimeTicket) {
        let executed_at = self.context.issue_time_ticket();
        self.context.push(Operation::Remove(RemoveOperation::new(
            parent_created_at.clone(),
            created_at.clone(),
            Some(executed_at),
        )));
    }

    fn record_array_move(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> TimeTicket {
        let executed_at = self.context.issue_time_ticket();
        self.context.push(Operation::Move(MoveOperation::create(
            parent_created_at.clone(),
            prev_created_at.clone(),
            created_at.clone(),
            Some(executed_at.clone()),
        )));
        executed_at
    }

    fn record_counter_increase(
        &mut self,
        parent_created_at: &TimeTicket,
        value: CounterValue,
        actor: Option<&str>,
    ) -> Result<()> {
        let executed_at = self.context.issue_time_ticket();
        let value = CrdtElement::primitive(counter_operand(value, executed_at.clone()));
        let operation = if let Some(actor) = actor {
            IncreaseOperation::create_with_actor(
                parent_created_at.clone(),
                value,
                Some(executed_at),
                actor,
            )
        } else {
            IncreaseOperation::create(parent_created_at.clone(), value, Some(executed_at))
        };

        self.context.push(Operation::Increase(operation));
        Ok(())
    }

    fn issue_time_ticket(&mut self) -> TimeTicket {
        self.context.issue_time_ticket()
    }

    fn record_text_edit(
        &mut self,
        parent_created_at: &TimeTicket,
        range: RgaTreeSplitPosRange,
        content: &str,
        attributes: BTreeMap<String, String>,
        executed_at: TimeTicket,
    ) {
        self.context.push(Operation::Edit(EditOperation::create(
            parent_created_at.clone(),
            range.0,
            range.1,
            content,
            attributes,
            Some(executed_at),
        )));
    }

    fn record_text_style(
        &mut self,
        parent_created_at: &TimeTicket,
        range: RgaTreeSplitPosRange,
        attributes: BTreeMap<String, String>,
        executed_at: TimeTicket,
    ) {
        self.context.push(Operation::Style(StyleOperation::create(
            parent_created_at.clone(),
            range.0,
            range.1,
            attributes,
            Some(executed_at),
        )));
    }

    fn last_time_ticket(&self) -> TimeTicket {
        self.context.last_time_ticket()
    }

    fn record_tree_edit(&mut self, operation: TreeEditOperation) {
        self.context.push(Operation::TreeEdit(operation));
    }

    fn record_tree_style(
        &mut self,
        parent_created_at: &TimeTicket,
        from_pos: TreePos,
        to_pos: TreePos,
        attributes: BTreeMap<String, String>,
        executed_at: TimeTicket,
    ) {
        self.context
            .push(Operation::TreeStyle(TreeStyleOperation::create(
                parent_created_at.clone(),
                from_pos,
                to_pos,
                attributes,
                Some(executed_at),
            )));
    }

    fn record_tree_remove_style(
        &mut self,
        parent_created_at: &TimeTicket,
        from_pos: TreePos,
        to_pos: TreePos,
        attributes_to_remove: Vec<String>,
        executed_at: TimeTicket,
    ) {
        self.context.push(Operation::TreeStyle(
            TreeStyleOperation::create_tree_remove_style_operation(
                parent_created_at.clone(),
                from_pos,
                to_pos,
                attributes_to_remove,
                Some(executed_at),
            ),
        ));
    }
}

fn collect_object_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonObject,
    after: &JsonObject,
) -> Result<()> {
    for (key, _) in before.iter() {
        if after.get(key).is_none() {
            push_remove_operation(context, crdt_root, parent_created_at, key)?;
        }
    }

    for (key, after_value) in after.iter() {
        let Some(before_value) = before.get(key) else {
            push_set_or_remove_unsupported(context, parent_created_at, key, after_value)?;
            continue;
        };

        if unchanged(before_value, after_value) {
            continue;
        }

        if let (JsonValue::Object(before_object), JsonValue::Object(after_object)) =
            (before_value, after_value)
        {
            if let Some(CrdtElement::Object(object)) =
                crdt_root.get_object_member(parent_created_at, key)?
            {
                collect_object_changes(
                    context,
                    crdt_root,
                    object.created_at(),
                    before_object,
                    after_object,
                )?;
                continue;
            }
        }

        if let (JsonValue::Array(before_array), JsonValue::Array(after_array)) =
            (before_value, after_value)
        {
            if let Some(CrdtElement::Array(array)) =
                crdt_root.get_object_member(parent_created_at, key)?
            {
                collect_array_changes(
                    context,
                    crdt_root,
                    array.created_at(),
                    before_array,
                    after_array,
                )?;
                continue;
            }
        }

        push_set_or_remove_unsupported(context, parent_created_at, key, after_value)?;
    }

    Ok(())
}

fn collect_array_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonArray,
    after: &JsonArray,
) -> Result<()> {
    if arrays_unchanged(before, after) {
        return Ok(());
    }

    if before.len() == after.len() {
        let pairs = lcs_pairs(before.as_slice(), after.as_slice());
        if pairs
            .iter()
            .any(|(before_index, after_index)| before_index != after_index)
        {
            return collect_sequence_array_changes(
                context,
                crdt_root,
                parent_created_at,
                before,
                after,
            );
        }

        collect_same_length_array_changes(context, crdt_root, parent_created_at, before, after)?;
        return Ok(());
    }

    collect_sequence_array_changes(context, crdt_root, parent_created_at, before, after)
}

fn collect_same_length_array_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonArray,
    after: &JsonArray,
) -> Result<()> {
    for index in 0..before.len() {
        let before_value = before.get(index).ok_or_else(|| {
            YorkieError::InvalidIndex(format!("missing before array index {index}"))
        })?;
        let after_value = after.get(index).ok_or_else(|| {
            YorkieError::InvalidIndex(format!("missing after array index {index}"))
        })?;
        if unchanged(before_value, after_value) {
            continue;
        }

        let current = crdt_root
            .array_by_created_at(parent_created_at)
            .and_then(|array| array.get(index))
            .ok_or_else(|| {
                YorkieError::InvalidIndex(format!("missing CRDT array index {index}"))
            })?;

        match (before_value, after_value, current) {
            (
                JsonValue::Object(before_object),
                JsonValue::Object(after_object),
                CrdtElement::Object(object),
            ) => {
                collect_object_changes(
                    context,
                    crdt_root,
                    object.created_at(),
                    before_object,
                    after_object,
                )?;
            }
            (
                JsonValue::Array(before_array),
                JsonValue::Array(after_array),
                CrdtElement::Array(array),
            ) => {
                collect_array_changes(
                    context,
                    crdt_root,
                    array.created_at(),
                    before_array,
                    after_array,
                )?;
            }
            _ => {
                push_array_set_operation(
                    context,
                    parent_created_at,
                    current.created_at().clone(),
                    after_value,
                )?;
            }
        }
    }

    Ok(())
}

fn collect_sequence_array_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonArray,
    after: &JsonArray,
) -> Result<()> {
    let pairs = lcs_pairs(before.as_slice(), after.as_slice());
    let mut before_matched = vec![false; before.len()];
    let mut after_created_at = vec![None; after.len()];

    for (before_index, after_index) in pairs {
        before_matched[before_index] = true;
        let created_at = crdt_root
            .array_by_created_at(parent_created_at)
            .and_then(|array| array.get(before_index))
            .map(CrdtElement::created_at)
            .ok_or_else(|| {
                YorkieError::InvalidIndex(format!("missing CRDT array index {before_index}"))
            })?
            .clone();
        after_created_at[after_index] = Some(created_at);
    }

    for (index, matched) in before_matched.iter().enumerate() {
        if *matched {
            continue;
        }

        let created_at = crdt_root
            .array_by_created_at(parent_created_at)
            .and_then(|array| array.get(index))
            .map(CrdtElement::created_at)
            .ok_or_else(|| YorkieError::InvalidIndex(format!("missing CRDT array index {index}")))?
            .clone();
        let executed_at = context.issue_time_ticket();
        context.push(Operation::Remove(RemoveOperation::new(
            parent_created_at.clone(),
            created_at,
            Some(executed_at),
        )));
    }

    let mut prev_created_at = TimeTicket::initial();
    for (index, after_value) in after.as_slice().iter().enumerate() {
        if let Some(created_at) = &after_created_at[index] {
            prev_created_at = crdt_root
                .array_by_created_at(parent_created_at)
                .and_then(|array| array.pos_created_at(created_at).ok())
                .unwrap_or_else(|| created_at.clone());
            continue;
        }

        let created_at = context.issue_time_ticket();
        let element = json_value_to_crdt_element(after_value, created_at.clone(), context)?;
        context.push(Operation::Add(AddOperation::create(
            parent_created_at.clone(),
            prev_created_at,
            element,
            Some(created_at.clone()),
        )));
        prev_created_at = created_at;
    }

    Ok(())
}

fn push_array_set_operation(
    context: &mut ChangeContext,
    parent_created_at: &TimeTicket,
    created_at: TimeTicket,
    value: &JsonValue,
) -> Result<()> {
    let executed_at = context.issue_time_ticket();
    let element = json_value_to_crdt_element(value, executed_at.clone(), context)?;
    context.push(Operation::ArraySet(ArraySetOperation::create(
        parent_created_at.clone(),
        created_at,
        element,
        Some(executed_at),
    )));
    Ok(())
}

/// Whether `after` is still `before` for the JSON-diff fallback of
/// `update_with_presence`, which runs only when the update recorded no
/// operation.
///
/// Values compare as `==` does, except that a tree in the document compares
/// by identity. Such a tree changes only through its own methods, and every
/// one that edits records an operation, so a difference the fallback sees
/// can only come from a read helper: `pos_range_to_index_range` and
/// `pos_range_to_path_range` split text nodes of the update's copy. JS splits
/// its live tree there too and records nothing ("these read-path conversions
/// emit no operation", `json/tree.ts` `registerPendingGCPairs`). Compared by
/// content, the split pieces would record a `SET` of the whole tree, which
/// replaces it on every replica.
fn unchanged(before: &JsonValue, after: &JsonValue) -> bool {
    match (before, after) {
        (JsonValue::Tree(before), JsonValue::Tree(after)) => {
            matches!((before.id(), after.id()), (Ok(before), Ok(after)) if before == after)
                || before == after
        }
        (JsonValue::Object(before), JsonValue::Object(after)) => {
            before.iter().count() == after.iter().count()
                && before.iter().all(|(key, before)| {
                    after.get(key).is_some_and(|after| unchanged(before, after))
                })
        }
        (JsonValue::Array(before), JsonValue::Array(after)) => arrays_unchanged(before, after),
        _ => before == after,
    }
}

fn arrays_unchanged(before: &JsonArray, after: &JsonArray) -> bool {
    before.len() == after.len()
        && before
            .iter()
            .zip(after.iter())
            .all(|(before, after)| unchanged(before, after))
}

fn lcs_pairs(before: &[JsonValue], after: &[JsonValue]) -> Vec<(usize, usize)> {
    let mut lengths = vec![vec![0; after.len() + 1]; before.len() + 1];
    for before_index in (0..before.len()).rev() {
        for after_index in (0..after.len()).rev() {
            lengths[before_index][after_index] =
                if unchanged(&before[before_index], &after[after_index]) {
                    lengths[before_index + 1][after_index + 1] + 1
                } else {
                    lengths[before_index + 1][after_index]
                        .max(lengths[before_index][after_index + 1])
                };
        }
    }

    let mut pairs = Vec::new();
    let mut before_index = 0;
    let mut after_index = 0;
    while before_index < before.len() && after_index < after.len() {
        if unchanged(&before[before_index], &after[after_index]) {
            pairs.push((before_index, after_index));
            before_index += 1;
            after_index += 1;
        } else if lengths[before_index + 1][after_index] >= lengths[before_index][after_index + 1] {
            before_index += 1;
        } else {
            after_index += 1;
        }
    }

    pairs
}

fn push_set_or_remove_unsupported(
    context: &mut ChangeContext,
    parent_created_at: &TimeTicket,
    key: &str,
    value: &JsonValue,
) -> Result<()> {
    let created_at = context.issue_time_ticket();
    let element = json_value_to_crdt_element(value, created_at.clone(), context)?;

    context.push(Operation::Set(SetOperation::create(
        key.to_owned(),
        element,
        parent_created_at.clone(),
        Some(created_at),
    )));

    Ok(())
}

fn push_remove_operation(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    key: &str,
) -> Result<()> {
    let Some(element) = crdt_root.get_object_member(parent_created_at, key)? else {
        return Ok(());
    };

    let executed_at = context.issue_time_ticket();
    context.push(Operation::Remove(RemoveOperation::new(
        parent_created_at.clone(),
        element.created_at().clone(),
        Some(executed_at),
    )));

    Ok(())
}

fn json_value_to_crdt_element(
    value: &JsonValue,
    created_at: TimeTicket,
    context: &mut ChangeContext,
) -> Result<CrdtElement> {
    let element = match value {
        JsonValue::Null => primitive(PrimitiveValue::Null, created_at),
        JsonValue::Bool(value) => primitive(PrimitiveValue::Boolean(*value), created_at),
        JsonValue::Integer(value) => primitive(PrimitiveValue::Integer(*value), created_at),
        JsonValue::Long(value) => primitive(PrimitiveValue::Long(*value), created_at),
        JsonValue::Double(value) => primitive(PrimitiveValue::Double(*value), created_at),
        JsonValue::String(value) => primitive(PrimitiveValue::String(value.clone()), created_at),
        JsonValue::Counter(value) => CrdtElement::counter(value.to_crdt_counter(created_at)),
        // As in JS, setting a text always creates an empty one.
        JsonValue::Text(_) => CrdtElement::text(CrdtText::create(created_at)),
        JsonValue::Tree(value) => {
            let root = value.build_root(&mut || context.issue_time_ticket())?;
            CrdtElement::tree(CrdtTree::create(root, created_at))
        }
        JsonValue::Object(value) => {
            let mut members = Vec::new();
            for (key, value) in value.iter() {
                let member_created_at = context.issue_time_ticket();
                let element = json_value_to_crdt_element(value, member_created_at, context)?;
                members.push((key.to_owned(), element));
            }
            CrdtElement::object(CrdtObject::create_with_members(created_at, members))
        }
        JsonValue::Array(value) => {
            let mut elements = Vec::new();
            for value in value.iter() {
                let element_created_at = context.issue_time_ticket();
                elements.push(json_value_to_crdt_element(
                    value,
                    element_created_at,
                    context,
                )?);
            }

            CrdtElement::array(CrdtArray::create_with_elements(created_at, elements)?)
        }
    };

    Ok(element)
}

fn primitive(value: PrimitiveValue, created_at: TimeTicket) -> CrdtElement {
    CrdtElement::primitive(CrdtPrimitive::new(value, created_at))
}

fn recorded_json_value_from_crdt_element(element: &CrdtElement) -> Result<RecordedJsonValue> {
    let created_at = element.created_at().clone();
    Ok(RecordedJsonValue::new(
        crdt_element_to_json_value(element)?,
        created_at.clone(),
        created_at,
    ))
}

fn crdt_object_to_json_object(object: &CrdtObject) -> Result<JsonObject> {
    let mut json_object = JsonObject::with_created_at(object.created_at().clone());

    for (key, element) in object.iter() {
        json_object.set_tracked_unchecked(
            key,
            crdt_element_to_json_value(element)?,
            element.created_at().clone(),
        );
    }

    Ok(json_object)
}

fn crdt_element_to_json_value(element: &CrdtElement) -> Result<JsonValue> {
    let value = match element {
        CrdtElement::Primitive(value) => value.to_json_value(),
        CrdtElement::Counter(value) => JsonValue::Counter(crate::JsonCounter::from_crdt(value)),
        CrdtElement::Object(value) => JsonValue::Object(crdt_object_to_json_object(value)?),
        CrdtElement::Array(value) => {
            let mut array = crate::JsonArray::with_created_at(value.created_at().clone());
            for node in value.iter_all_nodes() {
                let Some(element) = node.element() else {
                    continue;
                };
                if node.is_removed() {
                    continue;
                }
                array.push_tracked_unchecked(
                    crdt_element_to_json_value(element)?,
                    element.created_at().clone(),
                    node.position_created_at().clone(),
                );
            }
            JsonValue::Array(array)
        }
        CrdtElement::Text(value) => JsonValue::Text(crate::JsonText::from_crdt(value)),
        CrdtElement::Tree(value) => JsonValue::Tree(crate::JsonTree::from_crdt(value)),
    };

    Ok(value)
}

#[cfg(test)]
mod adv_sync;
#[cfg(test)]
mod gc_tests;
#[cfg(test)]
mod reissue_tests;
#[cfg(test)]
mod split_link_tests;
#[cfg(test)]
mod text_undo_fuzz_tests;
#[cfg(test)]
mod text_undo_ledger_tests;
#[cfg(test)]
mod update_tests;

#[cfg(test)]
mod tests {
    use super::{crdt_element_to_json_value, DocStatus, Document};
    use crate::change::ChangePack;
    use crate::crdt::counter::{CounterType, CounterValue, CrdtCounter};
    use crate::crdt::element::CrdtElement;
    use crate::crdt::rht::Rht;
    use crate::crdt::text::CrdtText;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::event::DocEvent;
    use crate::wire::WireChangePack;
    use crate::{
        Checkpoint, JsonArray, JsonObject, JsonValue, Result, SchemaRule, TreeNodeRule,
        VersionVector, YorkieError, INITIAL_ACTOR_ID,
    };

    #[test]
    fn creates_document_with_the_given_key() {
        let doc = Document::new("doc-key");
        assert_eq!("doc-key", doc.key());
        assert_eq!(Checkpoint::initial(), doc.checkpoint());
        assert_eq!(DocStatus::Detached, doc.status());
        assert!(!doc.is_attached());
        assert_eq!(INITIAL_ACTOR_ID, doc.actor_id().as_str());
        assert!(!doc.has_local_changes());
        assert_eq!(0, doc.max_size_per_document());
        assert!(doc.schema_rules().is_empty());
    }

    #[test]
    fn stores_attach_response_document_metadata() {
        let mut doc = Document::new("doc-key");
        let rules = vec![SchemaRule::new(
            "$.content",
            "tree",
            vec![TreeNodeRule::new("paragraph", "text*", "bold", "block")],
        )];

        doc.set_max_size_per_document(4096);
        doc.set_schema_rules(rules.clone());

        assert_eq!(4096, doc.max_size_per_document());
        assert_eq!(rules.as_slice(), doc.schema_rules());
    }

    #[test]
    fn applies_status_and_actor_assignment_to_pending_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.set_actor("000000000000000000000001");
        doc.apply_status(DocStatus::Attached);

        let pack = WireChangePack::try_from(&doc.create_change_pack())?;
        assert_eq!(DocStatus::Attached, doc.status());
        assert!(doc.is_attached());
        assert_eq!("000000000000000000000001", doc.actor_id().as_str());
        assert_eq!("000000000000000000000001", pack.changes[0].id.actor_id);

        doc.apply_status(DocStatus::Detached);

        assert_eq!(DocStatus::Detached, doc.status());
        assert!(!doc.is_attached());
        assert_eq!(INITIAL_ACTOR_ID, doc.actor_id().as_str());

        Ok(())
    }

    #[test]
    fn records_supported_root_sets_as_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;

        assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
        assert_eq!(r#"{"title":"hello"}"#, doc.crdt_root.to_sorted_json());
        assert_eq!(1, doc.local_changes.len());
        assert!(doc.has_local_changes());
        assert_eq!(1, doc.change_id.client_seq());
        assert_eq!(1, doc.change_id.lamport());
        assert_eq!(
            "0:00:0.SET.title=\"hello\"",
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn converts_crdt_text_to_public_json_text() -> Result<()> {
        let mut text = CrdtText::create(crate::TimeTicket::new(1, 0, "a"));
        text.edit_by_index(0, 0, "Hi", None, crate::TimeTicket::new(2, 0, "a"), None)?;

        let value = crdt_element_to_json_value(&CrdtElement::text(text))?;

        let JsonValue::Text(text) = &value else {
            panic!("expected text, got {value:?}");
        };
        assert_eq!(&crate::TimeTicket::new(1, 0, "a"), text.id());
        assert_eq!("Hi", text.to_string());
        assert_eq!(r#"[{"val":"Hi"}]"#, value.to_sorted_json());
        Ok(())
    }

    #[test]
    fn converts_crdt_counter_to_public_json_counter() -> Result<()> {
        let counter = CrdtCounter::create(
            CounterType::Long,
            CounterValue::Long(10),
            crate::TimeTicket::new(1, 0, "a"),
        );

        let value = crdt_element_to_json_value(&CrdtElement::counter(counter))?;

        assert_eq!("10", value.to_sorted_json());
        match value {
            JsonValue::Counter(counter) => {
                assert_eq!(CounterType::Long, counter.value_type());
                assert_eq!(CounterValue::Long(10), counter.value());
            }
            _ => panic!("expected counter"),
        }
        Ok(())
    }

    #[test]
    fn converts_crdt_tree_to_public_json_tree() -> Result<()> {
        let mut attrs = Rht::new();
        attrs.set("bold", "\"true\"", crate::TimeTicket::new(4, 0, "a"));
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(crate::TimeTicket::new(1, 0, "a"), 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    TreeNodeId::new(crate::TimeTicket::new(2, 0, "a"), 0),
                    "p",
                    Some(attrs),
                    vec![TreeNode::create_text(
                        TreeNodeId::new(crate::TimeTicket::new(3, 0, "a"), 0),
                        "Hi",
                    )],
                )],
            ),
            crate::TimeTicket::new(1, 0, "a"),
        );

        let value = crdt_element_to_json_value(&CrdtElement::tree(tree))?;

        let JsonValue::Tree(tree) = &value else {
            panic!("expected a tree, got {value:?}");
        };
        // The key order of JS `Tree.toJSON`.
        assert_eq!(
            r#"{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"Hi"}],"attributes":{"bold":"true"}}]}"#,
            value.to_sorted_json()
        );
        assert_eq!(r#"<root><p bold="true">Hi</p></root>"#, tree.to_xml()?);
        Ok(())
    }

    #[test]
    fn records_supported_root_removes_as_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| {
            root.remove("title");
            Ok(())
        })?;

        assert_eq!("{}", doc.to_sorted_json());
        assert_eq!("{}", doc.crdt_root.to_sorted_json());
        assert_eq!(2, doc.local_changes.len());
        assert_eq!(2, doc.change_id.client_seq());
        assert_eq!(1, doc.crdt_root.get_garbage_len());
        assert_eq!(
            "0:00:0.REMOVE.1:00:1",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_member_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            Ok(())
        })?;
        doc.update(|root| {
            let profile = root.get_object_mut("profile")?;
            profile.set("active", true)?;
            profile.remove("name");
            Ok(())
        })?;

        assert_eq!(
            r#"{"profile":{"active":true}}"#,
            doc.crdt_root.to_sorted_json()
        );
        assert_eq!(
            "1:00:1.SET.active=true,1:00:1.REMOVE.1:00:2",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_arrays_as_crdt_elements() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;

            let mut todos = JsonArray::new();
            todos.push("write tests")?.push(profile)?;
            root.set("todos", todos)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"todos":["write tests",{"name":"yorkie"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(5, doc.crdt_root.get_element_map_size());
        assert_eq!(
            r#"0:00:0.SET.todos=["write tests",{"name":"yorkie"}]"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_counter_creation_and_increase_in_the_same_update() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_counter("count", 1)?
                .increase(2i32)?
                .increase(3.5)?;
            Ok(())
        })?;

        assert_eq!(r#"{"count":6}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.count=1,1:00:1.INCREASE.2,1:00:1.INCREASE.3.5"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_counter_increases() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_counter("count", 10)?;
            Ok(())
        })?;
        doc.update(|root| {
            let count = root.get_counter_mut("count")?;
            assert_eq!(CounterValue::Integer(10), count.value());

            count.increase(5i32)?;
            Ok(())
        })?;

        assert_eq!(r#"{"count":15}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.INCREASE.5"#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_counter_increases_inside_arrays() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set("items", JsonArray::new())?;
            root.get_array_mut("items")?
                .push_counter(1)?
                .increase(4i32)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?
                .get_counter_mut(0)?
                .increase(5i32)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":[10]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.items=[],1:00:1.ADD.1,1:00:2.INCREASE.4"#,
            doc.local_changes[0].to_test_string()
        );
        assert_eq!(
            r#"1:00:2.INCREASE.5"#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_long_counter_overflow() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_long_counter("longCount", i64::MAX)?
                .increase(1i64)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"longCount":-9223372036854775808}"#,
            doc.to_sorted_json()
        );
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.longCount=9223372036854775807,1:00:1.INCREASE.1"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_dedup_counter_adds_with_actor() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_dedup_counter("uv")?
                .add("user-1")?
                .add("user-1")?
                .add("user-2")?;
            Ok(())
        })?;

        assert_eq!(r#"{"uv":2}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.uv=0,1:00:1.INCREASE.1,1:00:1.INCREASE.1,1:00:1.INCREASE.1"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_pushes_as_add_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.push("two")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","two"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(2, doc.local_changes.len());
        assert_eq!(r#"1:00:1.ADD."two""#, doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_array_push_after_object_set_in_the_same_update() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set("items", JsonArray::new())?;
            root.get_array_mut("items")?.push("one")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one"]}"#, doc.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.items=[],1:00:1.ADD."one""#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_removes_as_remove_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            assert_eq!(
                Some(JsonValue::String("two".to_owned())),
                root.get_array_mut("items")?.remove(1)
            );
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.REMOVE.1:00:3",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_delete_and_push_as_remove_and_add_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            assert_eq!(Some(JsonValue::String("two".to_owned())), items.remove(1));
            items.push("four")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","three","four"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.REMOVE.1:00:3,1:00:1.ADD."four""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_sets_as_array_set_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.set(1, "updated")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","updated"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.ARRAY_SET.1:00:3="updated""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_array_set_even_when_visible_value_is_unchanged() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.set(0, "one")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one"]}"#, doc.to_sorted_json());
        assert_eq!(
            r#"1:00:1.ARRAY_SET.1:00:2="one""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_inserts_as_add_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.insert(1, "two")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","two","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(r#"1:00:1.ADD."two""#, doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_public_array_insert_after_and_before_by_id() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("four")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let four_id = items.element_id(1).unwrap().clone();

            assert_eq!(
                Some(&JsonValue::String("one".to_owned())),
                items.get_by_id(&one_id)
            );

            items.insert_after(&one_id, "two")?;
            items.insert_before(&four_id, "three")?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"items":["one","two","three","four"]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.ADD."two",1:00:1.ADD."three""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn exposes_public_array_elements_with_ids() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let first = items.get_element_by_index(0).unwrap();
            let first_id = first.id().clone();

            assert_eq!(&JsonValue::String("one".to_owned()), first.value());
            assert_eq!(Some(&first_id), items.element_id(0));
            assert!(items.contains_id(&first_id));
            assert_eq!(Some(0), items.index_of_id(&first_id, None));
            assert_eq!(Some(0), items.last_index_of_id(&first_id, None));
            assert_eq!(
                Some(&JsonValue::String("one".to_owned())),
                items.get_by_id(&first_id)
            );
            assert_eq!(
                &JsonValue::String("one".to_owned()),
                items.get_element_by_id(&first_id).unwrap().value()
            );
            assert_eq!(
                &JsonValue::String("two".to_owned()),
                items.get_last().unwrap().value()
            );
            Ok(())
        })?;

        assert_eq!(1, doc.local_changes.len());
        Ok(())
    }

    #[test]
    fn records_public_array_insert_after_index() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push(1i32)?.push(3i32)?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.insert_integer_after(0, 2)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":[1,2,3]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!("1:00:1.ADD.2", doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_public_array_delete_by_id() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let two_id = items.element_id(1).unwrap().clone();

            assert_eq!(
                Some(JsonValue::String("two".to_owned())),
                items.delete_by_id(&two_id)
            );
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.REMOVE.1:00:3",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_splice_remove_and_insert() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("a")?.push("b")?.push("c")?.push("d")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let removed = root
                .get_array_mut("items")?
                .splice(1, Some(2), ["x", "y"])?;

            assert_eq!(r#"["b","c"]"#, removed.to_sorted_json());
            Ok(())
        })?;

        assert_eq!(r#"{"items":["a","x","y","d"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.REMOVE.1:00:3,1:00:1.REMOVE.1:00:4,1:00:1.ADD."x",1:00:1.ADD."y""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_splice_with_negative_start_and_open_delete_count() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("a")?.push("b")?.push("c")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let removed = root
                .get_array_mut("items")?
                .splice(-2, None, Vec::<JsonValue>::new())?;

            assert_eq!(r#"["b","c"]"#, removed.to_sorted_json());
            Ok(())
        })?;

        assert_eq!(r#"{"items":["a"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.REMOVE.1:00:3,1:00:1.REMOVE.1:00:4"#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_changes_after_array_splice_insert() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set("items", JsonArray::new())?;
            let mut inserted = JsonObject::new();
            inserted.set("name", "yorkie")?;
            let removed = root
                .get_array_mut("items")?
                .splice(0, Some(0), [inserted])?;
            assert_eq!("[]", removed.to_sorted_json());
            root.get_array_mut("items")?
                .get_object_mut(0)?
                .set("active", true)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"items":[{"active":true,"name":"yorkie"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            r#"0:00:0.SET.items=[],1:00:1.ADD.{"name":"yorkie"},1:00:2.SET.active=true"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_move_after_by_id() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let three_id = items.element_id(2).unwrap().clone();

            items.move_after(&three_id, &one_id)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["two","three","one"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!("1:00:1.MOVE", doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_public_array_move_front_and_last() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let three_id = items.last_id().unwrap().clone();

            items.move_last(&one_id)?;
            items.move_front(&three_id)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["three","two","one"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.MOVE,1:00:1.MOVE",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_move_before_and_after_by_index() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let three_id = items.element_id(2).unwrap().clone();

            items.move_before(&one_id, &three_id)?;
            items.move_after_by_index(2, 0)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","two","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.MOVE,1:00:1.MOVE",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_set_after_parent_set_in_the_same_update() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            root.get_object_mut("profile")?.set("active", true)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"profile":{"active":true,"name":"yorkie"}}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            r#"0:00:0.SET.profile={"name":"yorkie"},1:00:1.SET.active=true"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_changes_inside_arrays() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            let mut items = JsonArray::new();
            items.push(profile)?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?
                .get_object_mut(0)?
                .set("active", true)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"items":[{"active":true,"name":"yorkie"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            r#"{"items":[{"name":"yorkie","active":true}]}"#,
            doc.crdt_root.to_json()
        );
        assert_eq!(
            "1:00:2.SET.active=true",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn creates_change_pack_from_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| root.set("done", false).map(|_| ()))?;

        let pack = doc.create_change_pack();

        assert_eq!("doc-key", pack.document_key());
        assert_eq!(Checkpoint::new(0, 2), pack.checkpoint());
        assert!(!pack.is_removed());
        assert!(pack.has_changes());
        assert_eq!(2, pack.change_size());
        assert_eq!(2, pack.changes().len());
        assert_eq!(2, pack.operations_len());
        assert_eq!(Checkpoint::initial(), doc.checkpoint());
        assert!(doc.has_local_changes());
        assert_eq!(
            Some(2),
            pack.version_vector()
                .and_then(|vector| vector.get(doc.change_id.actor_id().as_str()))
        );
        Ok(())
    }

    #[test]
    fn applies_ack_change_pack_and_removes_pushed_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| root.set("done", false).map(|_| ()))?;

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(3, 1),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        );
        doc.apply_change_pack(&pack)?;

        assert_eq!(Checkpoint::new(3, 1), doc.checkpoint());
        assert!(doc.has_local_changes());
        assert_eq!(1, doc.local_changes.len());
        assert_eq!(2, doc.local_changes[0].id().client_seq());
        assert_eq!(r#"{"done":false,"title":"hello"}"#, doc.to_sorted_json());

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(4, 2),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        );
        doc.apply_change_pack(&pack)?;

        assert_eq!(Checkpoint::new(4, 2), doc.checkpoint());
        assert!(!doc.has_local_changes());
        Ok(())
    }

    /// JS `acknowledgePushedChanges`: the confirmed changes go, the server
    /// seq stays, the remote changes are not applied, and a removal is taken.
    #[test]
    fn acknowledges_pushed_changes_without_taking_remote_state() -> Result<()> {
        let mut source = Document::new("doc-key");
        source.update(|root| root.set("remote", 1).map(|_| ()))?;
        let remote_changes = source.create_change_pack().changes().to_vec();

        let mut doc = Document::new("doc-key");
        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| root.set("done", false).map(|_| ()))?;

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(7, 1),
            false,
            remote_changes,
            Some(VersionVector::new()),
            None,
        );
        doc.acknowledge_pushed_changes(&pack);

        assert_eq!(Checkpoint::new(0, 1), doc.checkpoint());
        assert_eq!(1, doc.local_changes.len());
        assert_eq!(2, doc.local_changes[0].id().client_seq());
        assert_eq!(r#"{"done":false,"title":"hello"}"#, doc.to_sorted_json());

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(8, 2),
            true,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        );
        doc.acknowledge_pushed_changes(&pack);

        assert_eq!(Checkpoint::new(0, 2), doc.checkpoint());
        assert!(!doc.has_local_changes());
        assert_eq!(DocStatus::Removed, doc.status());
        Ok(())
    }

    #[test]
    fn applies_removed_status_from_change_pack() -> Result<()> {
        let mut doc = Document::new("doc-key");
        doc.update(|root| root.set("title", "hello").map(|_| ()))?;

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(1, 1),
            true,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        );
        doc.apply_change_pack(&pack)?;

        assert_eq!(DocStatus::Removed, doc.status());
        assert_eq!(Checkpoint::new(1, 1), doc.checkpoint());
        assert_eq!(
            YorkieError::DocumentRemoved("doc-key".to_owned()),
            doc.update(|root| root.set("next", "value").map(|_| ()))
                .unwrap_err()
        );
        Ok(())
    }

    #[test]
    fn applies_remote_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let remote_pack = ChangePack::create(
            "target-doc",
            Checkpoint::new(1, 0),
            false,
            source_pack.changes().to_vec(),
            source_pack.version_vector().cloned(),
            None,
        );

        target.apply_change_pack(&remote_pack)?;

        assert_eq!(Checkpoint::new(1, 0), target.checkpoint());
        assert_eq!(2, target.change_id.lamport());
        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn applies_remote_array_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            let mut array = JsonArray::new();
            array.push("sync")?.push(false)?;
            root.set("items", array)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let remote_pack = ChangePack::create(
            "target-doc",
            Checkpoint::new(1, 0),
            false,
            source_pack.changes().to_vec(),
            source_pack.version_vector().cloned(),
            None,
        );

        target.apply_change_pack(&remote_pack)?;

        assert_eq!(r#"{"items":["sync",false]}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn applies_remote_counter_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            root.set_counter("count", 1)?.increase(2i32)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let remote_pack = ChangePack::create(
            "target-doc",
            Checkpoint::new(1, 0),
            false,
            source_pack.changes().to_vec(),
            source_pack.version_vector().cloned(),
            None,
        );

        target.apply_change_pack(&remote_pack)?;

        assert_eq!(r#"{"count":3}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn applies_remote_dedup_counter_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            root.set_dedup_counter("uv")?
                .add("user-1")?
                .add("user-1")?
                .add("user-2")?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let remote_pack = ChangePack::create(
            "target-doc",
            Checkpoint::new(1, 0),
            false,
            source_pack.changes().to_vec(),
            source_pack.version_vector().cloned(),
            None,
        );

        target.apply_change_pack(&remote_pack)?;

        assert_eq!(r#"{"uv":2}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    /// JS `applySnapshot` clears the history: it names elements and positions
    /// of the root the snapshot replaced.
    #[test]
    fn clears_the_history_when_a_snapshot_replaces_the_root() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| root.set("a", 1).map(|_| ()))?;
        doc.update(|root| root.set("a", 2).map(|_| ()))?;
        doc.history_mut().undo()?;
        assert!(doc.history().can_undo() && doc.history().can_redo());

        let snapshot = ChangePack::create_with_snapshot_root(
            "test-doc",
            Checkpoint::new(7, 3),
            false,
            Vec::new(),
            Some(doc.change_id.version_vector().clone()),
            Some(vec![1]),
            Some(doc.crdt_root.object().clone()),
        );
        doc.apply_change_pack(&snapshot)?;

        assert!(!doc.history().can_undo());
        assert!(!doc.history().can_redo());
        Ok(())
    }

    #[test]
    fn applies_snapshot_change_pack_to_root() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            root.set("title", "snapshot")?;
            let mut items = JsonArray::new();
            items.push("one")?.push(2i32)?;
            root.set("items", items)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let snapshot_pack = ChangePack::create_with_snapshot_root(
            "target-doc",
            Checkpoint::new(7, 0),
            false,
            Vec::new(),
            source_pack.version_vector().cloned(),
            Some(vec![1]),
            Some(source.crdt_root.object().clone()),
        );

        target.apply_change_pack(&snapshot_pack)?;

        assert_eq!(Checkpoint::new(7, 0), target.checkpoint());
        assert_eq!(
            r#"{"items":["one",2],"title":"snapshot"}"#,
            target.to_sorted_json()
        );
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        assert!(target.change_id.lamport() > source_pack.version_vector().unwrap().max_lamport());
        Ok(())
    }

    #[test]
    fn reapplies_remaining_local_changes_after_snapshot() -> Result<()> {
        let mut source = Document::new("source-doc");
        source.update(|root| root.set("title", "remote").map(|_| ()))?;

        let mut target = Document::new("target-doc");
        target.update(|root| root.set("acked1", "old").map(|_| ()))?;
        target.update(|root| root.set("acked2", "old").map(|_| ()))?;
        target.update(|root| root.set("draft", "local").map(|_| ()))?;

        let source_pack = source.create_change_pack();
        let snapshot_pack = ChangePack::create_with_snapshot_root(
            "target-doc",
            Checkpoint::new(7, 2),
            false,
            Vec::new(),
            source_pack.version_vector().cloned(),
            Some(vec![1]),
            Some(source.crdt_root.object().clone()),
        );

        target.apply_change_pack(&snapshot_pack)?;

        assert_eq!(1, target.local_changes.len());
        assert_eq!(
            r#"{"draft":"local","title":"remote"}"#,
            target.to_sorted_json()
        );
        Ok(())
    }

    // JS `applySnapshot`: replayed local changes publish local-change events,
    // then one snapshot event follows.
    #[test]
    fn snapshot_pack_publishes_replayed_changes_then_snapshot() -> Result<()> {
        let mut source = Document::new("source-doc");
        source.update(|root| root.set("title", "remote").map(|_| ()))?;
        let mut target = Document::new("target-doc");
        target.update(|root| root.set("draft", "local").map(|_| ()))?;
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        target.subscribe(move |event| sink.lock().unwrap().push(event.clone()));

        let source_pack = source.create_change_pack();
        let snapshot_pack = ChangePack::create_with_snapshot_root(
            "target-doc",
            Checkpoint::new(7, 0),
            false,
            Vec::new(),
            source_pack.version_vector().cloned(),
            Some(vec![1]),
            Some(source.crdt_root.object().clone()),
        );
        target.apply_change_pack(&snapshot_pack)?;

        let events = events.lock().unwrap();
        assert_eq!(2, events.len());
        assert!(matches!(&events[0], DocEvent::LocalChange(info) if info.operations.len() == 1));
        assert_eq!(DocEvent::Snapshot, events[1]);
        Ok(())
    }

    #[test]
    fn rejects_raw_snapshot_bytes_without_decoded_root() {
        let mut doc = Document::new("doc-key");
        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(1, 0),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            Some(vec![1]),
        );

        let err = doc.apply_change_pack(&pack).unwrap_err();

        assert_eq!(YorkieError::UnsupportedSnapshot, err);
        assert_eq!(Checkpoint::initial(), doc.checkpoint());
    }

    // Presence cases follow JS `test/unit/document/presence_test.ts` and
    // `Document.getMyPresence/getPresences/getOthersPresences`.

    fn presence_of(value: serde_json::Value) -> crate::Presence {
        value.as_object().unwrap().clone()
    }

    fn actor(n: u8) -> crate::ActorId {
        crate::ActorId::from(format!("{n:024x}"))
    }

    fn attached_doc(n: u8) -> Document {
        let mut doc = Document::new("doc-key");
        doc.set_actor(actor(n));
        doc.apply_status(DocStatus::Attached);
        doc
    }

    #[test]
    fn presence_set_in_update_is_reflected_in_my_presence() -> Result<()> {
        let mut doc = attached_doc(1);
        doc.update(|root| {
            root.set("k", 1i32)?;
            Ok(())
        })?;
        let before = doc.to_sorted_json();

        doc.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"name": "a", "n": 1})));
            presence.set(presence_of(serde_json::json!({"n": 2})));
            assert_eq!(Some(&serde_json::json!(2)), presence.get("n"));
            Ok(())
        })?;

        assert_eq!(
            presence_of(serde_json::json!({"name": "a", "n": 2})),
            doc.my_presence()
        );
        assert_eq!(before, doc.to_sorted_json());
        // A presence-only change is still a change and is pushed.
        let pack = doc.create_change_pack();
        assert_eq!(2, pack.change_size());
        assert_eq!(0, pack.changes()[1].operations().len());
        assert!(pack.changes()[1].presence_change().is_some());
        Ok(())
    }

    #[test]
    fn update_can_change_root_and_presence_together() -> Result<()> {
        let mut doc = attached_doc(1);
        doc.update_with_presence(|root, presence| {
            root.set("k", "v")?;
            presence.set(presence_of(serde_json::json!({"a": true})));
            Ok(())
        })?;

        assert_eq!(r#"{"k":"v"}"#, doc.to_sorted_json());
        assert_eq!(
            presence_of(serde_json::json!({"a": true})),
            doc.my_presence()
        );
        assert_eq!(1, doc.create_change_pack().change_size());
        Ok(())
    }

    #[test]
    fn failed_update_does_not_change_presence() {
        let mut doc = attached_doc(1);
        let result = doc.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"a": 1})));
            Err(YorkieError::DocumentRemoved("x".to_owned()))
        });
        assert!(result.is_err());
        assert!(doc.my_presence().is_empty());
        assert!(!doc.has_local_changes());
    }

    #[test]
    fn my_presence_is_empty_unless_attached() -> Result<()> {
        let mut doc = Document::new("doc-key");
        doc.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"a": 1})));
            Ok(())
        })?;
        assert!(doc.my_presence().is_empty());
        doc.apply_status(DocStatus::Attached);
        assert_eq!(presence_of(serde_json::json!({"a": 1})), doc.my_presence());
        Ok(())
    }

    #[test]
    fn set_actor_moves_self_presence() -> Result<()> {
        let mut doc = Document::new("doc-key");
        doc.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"a": 1})));
            Ok(())
        })?;
        doc.set_actor(actor(7));
        doc.apply_status(DocStatus::Attached);

        assert_eq!(presence_of(serde_json::json!({"a": 1})), doc.my_presence());
        assert!(doc.has_presence(&actor(7)));
        assert!(!doc.has_presence(&crate::ActorId::from(INITIAL_ACTOR_ID)));
        Ok(())
    }

    #[test]
    fn applies_remote_put_and_clear_and_lists_online_presences() -> Result<()> {
        let mut source = attached_doc(2);
        source.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"name": "b"})));
            Ok(())
        })?;
        let mut target = attached_doc(1);
        target.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"name": "a"})));
            Ok(())
        })?;

        let put_pack = |changes: Vec<_>| {
            ChangePack::create("doc-key", Checkpoint::new(1, 0), false, changes, None, None)
        };
        target.apply_change_pack(&put_pack(source.create_change_pack().changes().to_vec()))?;

        // Stored, but not listed until the actor is online.
        assert!(target.has_presence(&actor(2)));
        assert_eq!(None, target.presence(&actor(2)));
        assert!(target.others_presences().is_empty());

        target.add_online_client(actor(2));
        assert_eq!(
            Some(presence_of(serde_json::json!({"name": "b"}))),
            target.presence(&actor(2))
        );
        assert_eq!(
            vec![
                (actor(1), presence_of(serde_json::json!({"name": "a"}))),
                (actor(2), presence_of(serde_json::json!({"name": "b"}))),
            ],
            target.presences()
        );
        assert_eq!(1, target.others_presences().len());

        source.update_with_presence(|_, presence| {
            presence.clear();
            Ok(())
        })?;
        let clear = source.create_change_pack().changes()[1..].to_vec();
        target.apply_change_pack(&put_pack(clear))?;
        assert!(!target.has_presence(&actor(2)));
        assert_eq!(1, target.presences().len());

        target.remove_online_client(&actor(2));
        target.set_online_clients([actor(3)]);
        assert!(target.others_presences().is_empty());
        Ok(())
    }

    #[test]
    fn detach_status_drops_self_presence() -> Result<()> {
        let mut doc = attached_doc(1);
        doc.update_with_presence(|_, presence| {
            presence.set(presence_of(serde_json::json!({"a": 1})));
            Ok(())
        })?;
        doc.apply_status(DocStatus::Detached);
        assert!(!doc.has_presence(&actor(1)));
        doc.set_actor(actor(2));
        doc.apply_status(DocStatus::Attached);
        assert!(doc.my_presence().is_empty());
        Ok(())
    }

    #[test]
    fn fallback_diff_root_edit_and_presence_change_share_one_update() -> Result<()> {
        let mut doc = attached_doc(1);
        doc.update(|root| {
            root.set("title", "old")?;
            Ok(())
        })?;

        // Replacing the value in place bypasses the recorder, so the root
        // edit is found by the fallback diff.
        doc.update_with_presence(|root, presence| {
            *root.get_mut("title").unwrap() = JsonValue::from("new");
            presence.set(presence_of(serde_json::json!({"a": 1})));
            Ok(())
        })?;

        assert_eq!(r#"{"title":"new"}"#, doc.to_sorted_json());
        assert_eq!(crdt_root_json(&doc), doc.to_sorted_json());
        assert_eq!(presence_of(serde_json::json!({"a": 1})), doc.my_presence());
        let pack = doc.create_change_pack();
        assert_eq!(2, pack.change_size());
        let last = &pack.changes()[1];
        assert!(!last.operations().is_empty());
        assert!(last.presence_change().is_some());
        Ok(())
    }

    /// The fallback compares a tree in the document by identity, so that the
    /// split of a read helper records nothing (`json/pos_tests.rs`), but a
    /// new tree put in its place is still found.
    #[test]
    fn fallback_diff_finds_a_tree_replaced_in_place() -> Result<()> {
        use crate::{JsonTree, TreeNodeInput};
        let tree = |value: &str| {
            TreeNodeInput::element(
                "doc",
                vec![TreeNodeInput::element(
                    "p",
                    vec![TreeNodeInput::text(value)],
                )],
            )
        };
        let mut doc = attached_doc(1);
        doc.update(|root| root.set_tree("t", tree("old")).map(|_| ()))?;

        doc.update(|root| {
            *root.get_mut("t").unwrap() = JsonValue::Tree(JsonTree::new(tree("new")));
            Ok(())
        })?;

        let JsonValue::Tree(replaced) = doc.get_root().get("t").unwrap() else {
            panic!("expected a tree");
        };
        assert_eq!("<doc><p>new</p></doc>", replaced.to_xml()?);
        assert_eq!(crdt_root_json(&doc), doc.to_sorted_json());
        assert_eq!(2, doc.create_change_pack().change_size());
        Ok(())
    }

    fn crdt_root_json(doc: &Document) -> String {
        doc.crdt_root.to_sorted_json()
    }
}
