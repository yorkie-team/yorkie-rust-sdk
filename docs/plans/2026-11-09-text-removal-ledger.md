# Text Removal Ledger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Status:** Planned (2026-11-09). Item 1 of 4 after JS parity (#1).

**Goal:** Text undo and redo in the Rust SDK work by identity, as in the JS
SDK. A Rust undo of a Text edit sends `restore_spans`, `retombstone_spans`,
and `restore_mode`. A Rust replica that receives such an operation (from JS,
Go, or Rust) revives or re-removes the named pieces by identity. Every Text
node keeps the removal ledger of JS `84068f70` / Go `8997daf2`, GC follows
the ledger's retention rules, and snapshots carry `TextNode.removal_events`
byte-identical to JS and Go.

**Architecture:** The ledger lives on `RgaTreeSplitNode` (`yorkie-core`),
`RgaTreeSplit` gains `restore`/`retombstone`/`refresh_spans`, `CrdtText`
exposes them, `EditOperation` gains the identity path and the identity
reverse, `CrdtRoot` gains ledger pruning and the purge hold, `wire` and the
protocol converter carry the new fields. No public API changes: everything is
`pub(crate)` or lives in the already-public `wire` projection.

**Tech Stack:** Rust 1.99.1 (pinned in `rust-toolchain.toml`), prost 0.14,
existing crates only.

**Spec:** this document, `docs/current-porting-gaps.md` (Out-of-Scope Items
row "Identity-preserving undo restore spans and the removal ledger"; sections
Text and RGATreeSplit, Garbage Collection, Undo and Redo, Serialization and
Protocol Conversion), and `docs/porting-from-js-sdk.md`.

References (read-only):

- JS SDK `../yorkie-js-sdk` at `8629993e` (written below as `js:`, paths under
  `packages/sdk/`). Identity restore: `5d5cac63` (#1293), then `270ffc66`
  (recreate in order), `8619d095` (relink insertion chain), `4f1f0123`
  (GC pairs for pieces split off tombstones), `06f5587b` (attributes on
  reversed-insertion spans, `refreshSpans`, span validation). Ledger:
  `84068f70` (#1514). Design notes:
  `docs/tasks/archive/2026/10/20261021-text-undo-removal-ledger-{todo,lessons}.md`
  and `docs/tasks/archive/2026/10/20261012-restore-span-text-parent-*.md`.
- Go `../yorkie` at `54b96762` (written `go:`), ledger `8997daf2` (#2310), an
  ancestor of `54b96762`, so the vendored protos already carry every field.
  Design: `docs/design/undo-redo.md`, section "Removal Ledger".

Line ranges below are at those commits.

## Global Constraints

- JS is the behavioral source of truth; Go is the typed cross-check. Every
  rule starts from a named JS function or test. If JS and Go differ, follow
  JS and record the difference in `docs/current-porting-gaps.md` (the known
  ones are listed under "JS vs Go divergences" below).
- GC retention semantics are exactly JS `84068f70`: prune stable
  removal-cancel pairs before collecting, hold a tombstone whose ledger is not
  stable, and never collect a tombstone held by two or more uncancelled
  removals. Do not invent new retention, pruning, or anchoring rules, and do
  not "fix" a JS known limit (the recreate-after-GC anchor case).
- Wire bytes must match JS/Go: same proto fields, same presence rules (JS
  writes `restore_mode` and both span lists only when at least one span list
  is non-empty; a node whose `removed_at` says it all writes no
  `removal_events`), same event order (ticket order).
- Operations do not change shape on the wire; snapshots gain only
  `TextNode.removal_events` (field 6). The proto is already vendored at Yorkie
  `54b96762`; do not regenerate or edit protos.
- No new public API beyond what JS exposes. New types (`RestoreSpan`,
  `RestoreMode`, ledger types) are `pub(crate)` in `yorkie-core`; the only
  public additions are fields on the existing public `wire` projection types
  (`WireOperation::Edit`, `WireTextNode`) and a `WireRestoreSpan` struct they
  need.
- `yorkie-core` takes no new dependency (no logging crate; see Open
  Decisions).
- Tree is untouched. After this plan a received `TreeEdit` with
  `restore_spans`/`retombstone_spans`/`restore_mode` still decodes to its
  positional part only, exactly as today, and a Rust Tree edit still pushes
  no undo entry. The `restore_mode` fields on `Set`, `Add`, and `ArraySet`
  stay ignored on decode and unset on encode, as in the JS converter.
- Toolchain is pinned to 1.99.1. Every task ends with:

  ```sh
  export PATH="$HOME/.cargo/bin:$PATH"
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```

- Each ported test names its JS (and, where used, Go) source in a doc comment
  above it, as in the existing ports.
- Commits are English, subject ≤70 chars, imperative verb, no type prefix;
  body lines ≤80 and say why. End every commit with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. There is no
  commit-msg hook here; check lengths by hand.
- Out of scope: Tree undo/redo and Tree `restore_spans` receive side,
  `reissueActor`, `maxUndoDepth`/`clearHistory`, offline persistence.

## Review Focus

Convergence-risk areas a reviewer must check line by line against JS:

1. **`delete_nodes` live-in-view rule** (Task 1). A remote deletion removes
   only nodes live in the deleter's view (`creation known &&
   !is_removed_in_view(vector)`), and records the deletion without removing
   on a node it saw live but someone else removed. The old rule (creation
   known, not yet tombstoned) must be gone. LWW overwrite of `removed_at`
   through `canRemove` must still happen. Local edits (`None` or empty vector)
   keep `!is_removed()`.
2. **One uncancelled removal per actor** (Task 1). `record_removal` ignores a
   second removal by an actor that already holds an uncancelled one; the
   restore cancels only the undoing actor's own removal. Breaking either
   breaks the `removal_events` pairing on every other replica.
3. **`ticket_known` for an empty vector vs an absent actor** (Task 1). Empty
   or absent vector = local = known; an actor missing from a non-empty vector
   is unknown even at lamport 0 (`js:src/document/time/version_vector.ts`
   173-186). The lamport-0 stand-in removal relies on this.
4. **`held_from_purge` and `prune_ledgers` ordering** (Task 3). Prune runs
   once before the collect loop; the hold is checked after the `removed_at`
   stability check and before the successor barrier. Two uncancelled
   removals hold forever.
5. **Restore of a purged piece** (Task 2). Recreated along the purged pieces'
   own boundaries, anchored by `find_restore_anchor` (nearest following piece
   of the same insertion first), chained left-to-right, insertion chain
   relinked to the nearest surviving pieces on both sides, stand-in removal
   at lamport 0 plus its cancel recorded.
6. **GC accounting order in the identity path** (Task 4): register pending
   pairs from tombstone splits, then unregister revived nodes (gc → live),
   then `acc` the recreated size. `docSize` after delete/undo/redo/undo must
   equal a rebuild.
7. **Reverse shape** (Task 4). The reverse of a redo, and of an insertion's
   undo, names only the pieces the removal was recorded on; an identity
   reverse with no spans degrades to an empty undo edit at the head; an undo
   edit with empty content runs as a no-op at the head *before* any
   `refine_pos` (Rust refines in `Change::execute_undo_redo`, earlier than
   JS); `refresh_spans` runs only when the source is not `Remote`.
8. **Ledger index caching** (Task 1). JS keeps a per-node index because
   rebuilding it per walk made a 2056-pair churn quadratic. The Rust node is
   `Clone + PartialEq`; a cache must not make two equal ledgers compare
   unequal (keep it eager and derived, or exclude it from `PartialEq`).
9. **Snapshot decode of a malformed ledger** (Task 5) leaves the node with
   `removed_at` alone and never fails the snapshot or the change pack.
10. **Span validation at the wire boundary** (Task 5): missing `created_at`,
    negative `start`, `end < start`, `end - start != UTF-16 length of
    content` are rejected as `ErrInvalidArgument`-equivalent errors. Note the
    UTF-16 length, not bytes.

## JS vs Go divergences (follow JS)

- Go bounds removal events per node at 4096 (`go:api/converter/from_pb.go`
  1383-1400, `maxTextRemovalEvents`) in element payloads a client pushes. JS
  has no bound (it decodes only what the server accepted). Rust: no bound.
- Go rejects an identity span whose `created_at` lies beyond the change's
  version vector (`go:pkg/document/crdt/text_restore_test.go`
  `TestTextRestoreRejectsForgedIdentity`). JS does not. Rust: does not.
- Go `refinePos` errors on a stale offset past the end; JS snaps to the end
  (Go fuzz seed 152). Rust `refine_pos` already snaps like JS; keep it.
- JS `history_text_test.ts` still marks the two "correctness" cases
  `it.skip` with a stale comment; Go runs them live
  (`go:test/integration/history_text_reconcile_test.go`
  `TestReconcileOverlappingUndoDuplicatesContent`). Rust un-ignores them
  (Task 7).
- Both share one known limit: after GC, a recreated piece can land after
  text another insertion placed between the gap and the following piece of
  its own insertion. The fuzz setting `gc=true insert=100%` is report-only in
  both; it stays report-only here.

## Open Decisions

- **Warning on a malformed ledger.** JS logs `logger.warn` and drops the
  ledger. Rust has no logging facade, and `yorkie-core` takes no new
  dependency. Default in this plan: drop silently, keep `removed_at`, and
  record the missing log line in the Serialization gap. Alternative (needs
  the maintainer's yes): add the `log` facade crate to `yorkie-core`.
- **Revived-but-registered GC pair.** JS `CRDTRoot.collect`
  (`js:src/document/crdt/root.ts` ~875-882) unregisters a pair whose child
  is live again. It is listed as an existing Rust gap; restore is the first
  Rust path that revives text nodes, so this plan ports it (Task 3). Drop it
  from Task 3 if the reviewer wants that gap handled separately.
- **Golden fixtures from Go.** Task 6 generates JS fixtures; Go fixtures are
  added only if a Go generator runs cleanly in a scratch worktree. Without
  them the Go cross-check is by reading, not by bytes.

---

## Milestone 1: CRDT

### Task 1: Add the removal ledger to Text nodes

**Goal:** `RgaTreeSplitNode` keeps JS's removal ledger, and remote deletions
use the live-in-view rule. Text `edit` reports the removed spans that the
identity reverse needs.

**Files:**
- Modify: `crates/yorkie-core/src/crdt/rga_tree_split.rs` (node ledger,
  `split` copies it, `delete_nodes`, `edit` result gains `removed_spans`)
- Modify: `crates/yorkie-core/src/crdt/text.rs` (`edit_by_range` passes
  `removed_spans` through; `RestoreSpan<TextValue>` type alias if useful)
- Modify: `crates/yorkie-core/src/time/version_vector.rs` (add
  `ticket_known` if no equivalent exists)
- Create: `crates/yorkie-core/src/crdt/text_ledger_tests.rs`
  (`#[cfg(test)]` module declared from `crdt/mod.rs`)

**Port from:**
- `js:src/document/crdt/rga_tree_split.ts` 280-320 (`RestoreSpan`,
  `LedgerCancel`, `LedgerEntry`, `LedgerIndex`), 520-537 (`split`,
  `copyLedger`), 541-886 (`getLedger`, `hasCancels`, `removals`, `ledger`,
  `ledgerIndex`, `recompute`, `recordRemoval`, `cancelRemoval`,
  `pruneStable`, `uncancelledRemovalBy`, `heldFromPurge`,
  `isRemovedInView`, `getRemovalEvents`, `setRemovalEvents`), 888-910
  (`canRemove`), 1185-1220 (`removedSpans` in `edit`), 2145-2237
  (`deleteNodes`).
- `js:src/document/time/version_vector.ts` 173-186 (`ticketKnown`).
- Cross-check `go:pkg/document/crdt/rga_tree_split.go` 310-600 and
  `deleteNodes` at 1144.

**Behavior:**
- `removed_at` stays the latest uncancelled removal. A node with one removal
  and no cancel has no ledger (`ledgered == false`) and behaves exactly as
  today.
- `record_removal`, `cancel_removal`, `prune_stable`,
  `uncancelled_removal_by`, `held_from_purge`, `is_removed_in_view`,
  `removal_events`, `set_removal_events` follow JS line by line, including
  the error cases of `set_removal_events` (repeated ticket; result does not
  reproduce `removed_at`), which leave the node unchanged.
- `delete_nodes`: per JS 2145-2237, including `record_removal` on a kept node
  that was live in the remote deleter's view but removed here, and
  `record_removal(edited_at)` on each removed node (not a bare
  `removed_at = ..`). Removed nodes include ones whose `removed_at` was
  overwritten by LWW (JS `alreadyRemovedIDs`): they produce removed values
  and spans but no new GC pair.
- `edit` returns `removed_spans` in document order: one span per removed
  node, `created_at` of the insertion, `[offset, offset + content_len)`, and
  a deep copy of the value.

**Tests** (port into `text_ledger_tests.rs`, from
`js:test/unit/document/crdt/text_removal_ledger_test.ts`):
- "RGATreeSplitNode removal events": `pairs each actor's events by place`,
  the `it.each` rejection table at line 107 (repeated ticket; does not
  reproduce removed_at), `writes nothing when removedAt says it all`.
- "Text removal ledger": `returns removed spans in document order`,
  `keeps one uncancelled removal per actor`.
- Cross-check with Go `TestTextEditRemovedSpansInDocumentOrder` and
  `TestTextRemovalKeepsOneUncancelledRemovalPerActor`.
- Existing Text, GC, and history tests must stay green unchanged.

- [ ] Write the tests RED, port the ledger and `delete_nodes`, go GREEN.
- [ ] Verification gate. Commit "Add the removal ledger to Text nodes".

### Task 2: Restore and re-remove Text pieces by identity

**Goal:** `RgaTreeSplit::restore`, `retombstone`, and `refresh_spans`, exposed
through `CrdtText`, with the per-split cancelled-node tracking that pruning
needs.

**Files:**
- Modify: `crates/yorkie-core/src/crdt/rga_tree_split.rs`
- Modify: `crates/yorkie-core/src/crdt/text.rs` (`restore`, `retombstone`,
  `has_cancelled_removals`, `prune_ledgers`)
- Modify: `crates/yorkie-core/src/crdt/text_ledger_tests.rs`

**Port from:**
- `js:src/document/crdt/rga_tree_split.ts` 1020-1073 (`cancelled`,
  `trackCancelled`, `scanCancelled`, `hasCancelledRemovals`,
  `pruneLedgers`), 1224-1398 (`restore`), 1400-1506 (`retombstone`),
  1508-1557 (`refreshSpans`), 1559-1718 (`findPiecesOverlapping`,
  `findPieceCovering`, `findRestoreAnchor`, `isolateRange`), 2045-2143
  (`splitNode` pending GC pairs, `bookCopiedAttrTombstones`,
  `drainPendingGCPairs`).
- `js:src/document/crdt/text.ts` 364-425 (`restore`, `retombstone`,
  `hasCancelledRemovals`, `pruneLedgers`); `js:src/util/llrb_tree.ts` 167
  (`ceilingEntry`; Rust: `tree_by_id.range(key..).next()`).
- Cross-check `go:pkg/document/crdt/rga_tree_split.go` 730-770, 1255
  (`restore`), 1380 (`retombstone`), 1549 (`findRestoreAnchor`).

**Behavior:**
- `restore(spans, executed_at, fallback_anchor)`: a live piece is skipped; a
  tombstoned piece is isolated and the undoing actor's uncancelled removal is
  cancelled (revived only if nothing else holds it); a gap is recreated per
  JS, with the lamport-0 stand-in removal by the undoing actor plus its
  cancel, the insertion chain relinked to the nearest surviving pieces on
  both sides, and the chain anchor so a multi-fragment run lands in order.
  Returns revived nodes, recreated nodes, value changes (ascending index),
  live size diff, and pending GC pairs.
- `retombstone(spans, executed_at, vector)`: only pieces live in the
  vector's view are removed; a removed piece the vector saw live records the
  removal without re-tombstoning; returns GC pairs, value changes, split
  diff, and the recorded spans.
- `refresh_spans` re-snapshots spans against current pieces (read-only).
- **First check** whether Rust registers born-dead remainders when it splits
  a tombstoned node (JS `4f1f0123`). The gap doc says a Tree split does not;
  if Text does not either, port `pending_gc_pairs`/`drain_pending_gc_pairs`
  for Text splits here (restore and retombstone split tombstones heavily).
  Pairs are keyed by ID string in `CrdtRoot`, so the remainder's new ID
  string is a new pair.

**Tests:**
- From `text_removal_ledger_test.ts`: `cancels only the undoing actor's
  removal`, `reports nothing tombstoned by a redo of a removed piece`.
- From `go:pkg/document/crdt/text_restore_test.go` (CRDT level, no wire):
  `TestTextRestore` subtests (overlapping undo converges in both orders, only
  one side undone, partial overlap, redo then undo again, duplicate restore
  is a no-op).
- From `js:test/unit/document/text_restore_relink_test.ts`: `keeps a later
  boundary edit correct after a purged interior fragment is recreated`.
- From `js:test/unit/document/gc_split_leak_test.ts` "GC tombstone-split
  leak (Text)": `purges pieces split off a tombstoned text node` (as written
  before `84068f70`; Task 3 applies the `84068f70` change).

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Restore and re-remove Text pieces by
  identity".

### Task 3: Hold and prune Text ledgers in garbage collection

**Goal:** `CrdtRoot` prunes stable ledger pairs before collecting and keeps a
tombstone whose ledger still matters.

**Files:**
- Modify: `crates/yorkie-core/src/crdt/root.rs` (`ledger_holders` set keyed
  by text `created_at`, `register_ledger_holder`, private `prune_ledgers`,
  hold check in `collect`, `unregister_gc_pair_by_id` (gc → live), drop of a
  revived-but-registered pair, holder registration when a root is rebuilt
  from a snapshot or copied)
- Modify: `crates/yorkie-core/src/crdt/text.rs`,
  `crates/yorkie-core/src/crdt/rga_tree_split.rs` (purge drops the node's
  ledger; `held_from_purge` lookup by child ID)
- Modify: `crates/yorkie-core/src/document/gc_tests.rs`

**Port from:**
- `js:src/document/crdt/root.ts` ~170-180 (holders found on construction),
  788-800 (`garbageCollect`: prune first), 805-825 (`registerLedgerHolder`,
  `pruneLedgers`: drop a removed holder or one with nothing left), ~830-910
  (`collect`: revived pair dropped via `unregisterGCPair`, then `removed_at`
  stability, then `heldFromPurge`, then the barrier).
- `js:src/document/crdt/gc.ts` 89-102; `js:src/document/crdt/rga_tree_split.ts`
  2320-2351 (`purge`).

**Behavior:** as JS. A text with cancelled removals is registered as a holder
whenever a restore leaves cancels (Task 4 calls `register_ledger_holder`) and
when a root is built from existing content.

**Tests** (`gc_tests.rs`):
- `js:test/unit/document/gc_split_leak_test.ts` "GC tombstone-split leak
  (Text)" with the `84068f70` change: `purges pieces split off a tombstoned
  text node` and `keeps GC registration when a newer concurrent delete
  overwrites a tombstone` now expect the `cd` piece held by both deletions
  to stay (one tombstone, `garbage_len == 1`, `gc` size equals that node's
  size).
- A CRDT-level test that a tombstone with an unstable cancel is held, then
  collected once the cancel is known everywhere and a single removal holds
  it (mirrors `keeps a piece until its cancelled removals are known
  everywhere`, which Task 6 ports end to end).

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Hold Text tombstones their removal ledger
  still needs".

## Milestone 2: Operations and wire

### Task 4: Undo Text edits by identity

**Goal:** `EditOperation` carries `restore_spans`, `retombstone_spans`, and
`restore_mode`; it executes the identity path and builds the JS reverses.
Undo/redo of a Text edit uses them.

**Files:**
- Modify: `crates/yorkie-core/src/operation/edit_operation.rs` (fields,
  `RestoreMode { Restore, Retombstone }`, identity branch of `execute`,
  `identity_reverse`, `empty_reverse`, `to_reverse_operation`,
  `reconcile_operation` leaves spans untouched, `to_test_string`)
- Modify: `crates/yorkie-core/src/change/change.rs` (`execute_undo_redo`:
  skip positional refine for identity ops; empty-content undo edit moves to
  the head instead of refining)
- Modify: `crates/yorkie-core/src/history.rs` only if `reconcile_text_edit`
  needs to see identity ops (it must still shift `from_pos`, the fallback
  anchor)
- Modify: `crates/yorkie-core/src/operation/mod.rs` if `Operation` helpers
  match on `Edit` fields

**Port from:**
- `js:src/document/operation/edit_operation.ts` 37-118 (fields, `create`),
  121-313 (`execute`: identity branch, `isUndoOp` empty-content head rule,
  refine, positional path), 315-352 (`identityReverse`, `emptyReverse`),
  353-431 (`toReverseOperation`: reversed insertion becomes a retombstone
  span carrying the edit's attributes stamped with `executed_at`),
  469-557 (`reconcileOperation`).
- `js:src/document/crdt/text.ts` 312-362 (`edit` returns removed spans).
- Cross-check `go:pkg/document/operations/edit.go` (`restoreMode`,
  `NewRestoreEdit`).

**Behavior:**
- The identity path is chosen by `restore_mode`, not by span presence.
- Order inside the identity path: `refresh_spans` (only when source is not
  `Remote`), retombstone, restore, `register_ledger_holder` when the text has
  cancelled removals, register pending pairs, unregister revived nodes,
  `acc`, op infos per change.
- Reverse: flips the mode; a redo's reverse names the recorded pieces; empty
  spans degrade to `empty_reverse()` at the head.
- Positional path: unchanged except that a non-empty removal or insertion
  now reverses to an identity op with `restore` mode (removed spans to
  restore, inserted span to retombstone); only an edit that removed and
  inserted nothing keeps the positional reverse.
- An undo edit with empty content runs as a no-op at the head and keeps
  head positions for other replicas; its reverse is `empty_reverse()` unless
  the source is `Remote`.

**Tests** (`edit_operation.rs` tests and `history/tests.rs`):
- `text_removal_ledger_test.ts` "EditOperation empty undo": `runs as a no-op
  wherever it points`.
- `js:test/unit/document/document_test.ts` cases added in `5d5cac63`
  (identity undo/redo of a deletion, overlapping undo in-process).
- `js:test/unit/document/text_restore_convergence_test.ts`: `converges when
  both replicas undo overlapping deletes (d1 first)`, `... (d2 first)`,
  `purges symmetrically with docSize.gc drained after both undos`,
  `reverses GC accounting exactly across delete/undo/redo/undo` (in-process
  exchange here; Task 6 repeats the key ones through the wire).
- Existing `history/tests.rs` and `crates/yorkie-core/tests/history.rs` stay
  green; update only expectations that encoded the old positional reverse,
  each with a comment naming the JS rule.

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Undo and redo Text edits by identity".

### Task 5: Carry restore spans and removal ledgers on the wire

**Goal:** Restore fields and `removal_events` survive core → wire → proto and
back, with JS's presence rules, validation, and malformed-ledger handling.

**Files:**
- Modify: `crates/yorkie-core/src/wire.rs` (`WireRestoreSpan { created_at,
  start, end, content, attributes }`, `WireRestoreMode`, fields on
  `WireOperation::Edit`, `removal_events: Vec<TimeTicket>` on
  `WireTextNode`, domain conversions incl. `wire_text_node_from_domain` and
  `text_from_wire_nodes`)
- Modify: `crates/yorkie-protocol/src/converter.rs` (Edit encode/decode,
  `wire_text_node_to_proto`/`proto_text_node_to_wire`, span validation)
- Leave the `TreeEdit` arms and the `Set`/`Add`/`ArraySet` `restore_mode`
  untouched (see Global Constraints).

**Port from:**
- `js:src/api/converter.ts` 520-560 (`toOperation` Edit), 1609-1675
  (`fromOperation` Edit: `fromPbSpan` validation, attributes stamped with
  `executedAt`, `isUndoOp` set when a mode is present), 780-796
  (`toTextNodes` `removalEvents`), 1343-1405 (`fromTextNode`: drop the
  ledger on a missing ticket or a `setRemovalEvents` refusal).
- Cross-check `go:api/converter/from_pb.go` 617 (`fromRestoreSpans`),
  `go:api/converter/from_bytes.go` 392-470 (`fromTextNode`).

**Behavior:**
- Encode writes both span lists and the mode only when a span list is
  non-empty; otherwise all three stay default (bytes identical to today).
- Decode reads spans only when a list is non-empty; mode `RETOMBSTONE` →
  retombstone, anything else → restore.
- Malformed span → decode error (the whole operation is rejected, as JS
  throws).
- `removal_events` written in ticket order, empty when `removed_at` says it
  all. A malformed ledger is dropped (node keeps `removed_at`), never an
  error (see Open Decisions for the warning).

**Tests:**
- Port all of `js:test/unit/api/restore_converter_test.ts` into
  `converter.rs` tests: `round-trips a restore operation over the wire`,
  `round-trips a retombstone operation`, `round-trips the companion
  retombstone_spans of a replace reverse`, `keeps span attributes, stamped
  with executedAt`, `rejects a malformed span` (missing created_at, negative
  start, inverted interval, interval not covering content — use a UTF-16
  case such as an emoji), `leaves ordinary edits without a restore payload`,
  `decodes to a harmless no-op for peers that ignore restore fields`.
- Snapshot: a text node without a ledger encodes to the same bytes as
  before (assert against a byte vector captured from `main` before the
  change); a ledgered node round-trips its events; a ledger that breaks the
  rules is dropped and the snapshot still applies.
- A `TreeEdit` carrying restore fields still decodes to the positional op
  (regression test pinning "unchanged").

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Carry Text restore spans and ledgers on the
  wire".

## Milestone 3: End-to-end and interop

### Task 6: Port the Text undo ledger and fuzz tests

**Goal:** The JS document-level ledger tests and the fuzz harness pass over
a wire round trip, and the key cases pass over protobuf bytes.

**Files:**
- Create: `crates/yorkie-core/src/document/adv_sync.rs` (`#[cfg(test)]`
  port of `js:test/helper/adv_sync.ts`: `mulberry32`, `min_version_vector`,
  `AdvServer`/`AdvClient` push/pull, every change through
  `ChangePack → wire → ChangePack`, snapshot re-encode through the wire
  root projection)
- Create: `crates/yorkie-core/src/document/text_undo_ledger_tests.rs`
- Create: `crates/yorkie-core/src/document/text_undo_fuzz_tests.rs`
- Modify: `crates/yorkie-core/src/document.rs` (declare the test modules)
- Modify: `crates/yorkie-protocol/src/converter.rs` tests (protobuf-bytes
  replays, using the pattern of `roundtrips_snapshot_root_change_pack_through_proto_bytes`)

`yorkie-core` cannot depend on prost, so core tests round-trip through
`wire`; the protobuf leg is covered by Task 5 and the protocol replays here.

**Tests:**
- All of `js:test/unit/document/text_undo_removal_ledger_test.ts` (20 cases):
  undo vs unseen concurrent deletion (`undoer first`, `deleter first`),
  `does not remove text a concurrent undo revives (yorkie#2237)`, `cancels
  the redo, not the original deletion, on the next undo`, `leaves alone a
  removed piece the deleter saw removed`, `keeps the ledger through a
  snapshot`, `keeps a piece another deletion still holds out of GC`, `does
  not recreate a purged piece a redo never removed`, `revives a deletion
  nobody else made, also after a snapshot`, `redoes only what the redoer saw
  live`, `propagates an undo that leaves the text as it was`, `redo leaves
  alone a piece removed in its view`, `recreates a purged piece before the
  piece that follows it`, `keeps a piece until its cancelled removals are
  known everywhere`, `links a recreated piece to the nearest surviving
  neighbours`, `keeps a recreated piece out of a deletion that saw it
  removed`, `loads a snapshot whose removal events break the rules`, `reads
  back a snapshot whose ledger it grew itself` (must finish well under a
  second in debug; it guards the ledger index), `drops cancelled removals
  every replica knows`, `carries a cancelled removal through a snapshot`,
  `reverses an undo that found nothing left to undo`.
- All of `js:test/unit/document/text_restore_after_gc_test.ts` (multi-
  insertion run order, single insertion, only the undoing replica purged,
  the five redo-after-GC attribute cases).
- `js:test/unit/document/text_undo_fuzz_test.ts`: the five settings, 40
  steps, seeds from `TEXT_UNDO_FUZZ_SEEDS` (default 10), the report-only
  setting behind `TEXT_UNDO_FUZZ_FULL=1`; every step re-encodes each
  replica's text as a snapshot and compares the decoded ledger. Failures
  print the seed to stderr.
- Protocol replays (`converter.rs`): yorkie#2237 and `keeps the ledger
  through a snapshot`, exchanged as protobuf bytes, asserting text and
  convergence.

- [ ] Port; any failure is a bug in Tasks 1-5, fixed there (amend nothing:
  a follow-up commit naming the task).
- [ ] Run once with `TEXT_UNDO_FUZZ_SEEDS=1000 TEXT_UNDO_FUZZ_FULL=1` and
  record the counts in the commit body (JS: 0/1000 on required settings).
- [ ] Verification gate. Commit "Port the Text undo ledger and fuzz tests".

### Task 7: Check interop, un-ignore history cases, update docs

**Goal:** Byte-level interop evidence with JS (and Go if practical), the two
overlapping-undo correctness cases live, and the gap docs rewritten.

**Files:**
- Create: `crates/yorkie-protocol/tests/text_undo_interop.rs` and
  `crates/yorkie-protocol/tests/fixtures/text_undo/*.hex`
- Create: `tools/interop/text_undo_fixtures.test.ts` (JS generator, not
  built by cargo) and, if practical, `tools/interop/text_undo_fixtures_test.go`
- Modify: `crates/yorkie/tests/integration_history.rs` (remove the two
  `#[ignore]`s)
- Modify: `docs/current-porting-gaps.md`, `docs/porting-from-js-sdk.md`

**Steps:**
- [ ] Generate fixtures in a scratch copy of the JS checkout (Node 22,
  pnpm 9; `pnpm sdk exec vitest run <copied generator>`) with fixed actor
  IDs: (a) a `ChangePack` holding an undo of a deletion (`restore` mode) and
  its redo (`retombstone` mode) with a replace reverse (both span lists);
  (b) a snapshot whose text holds a ledgered node (removal, cancel, a second
  actor's removal) and a recreated node with the lamport-0 stand-in. Use at
  most one attribute per map: Go marshals map entries in random order, so
  byte equality holds only for single-entry maps. Write hex plus a README
  line with the exact command and JS commit.
- [ ] If a Go generator runs cleanly in a scratch worktree of `../yorkie`,
  produce the same scenarios from Go and add them; otherwise record "JS
  fixtures only" in the gap doc.
- [ ] `text_undo_interop.rs`: decode each fixture with
  `decode_change_pack`/snapshot decode, re-encode, assert byte equality;
  apply (a) to a Rust replica holding the same base text and assert the text
  JS produced; assert the decoded ledger of (b) node by node.
- [ ] Remove the `#[ignore]` on
  `restores_the_original_after_both_undo_contained_deletions` and
  `restores_the_original_after_both_undo_overlapping_deletions`; update
  their doc comments (JS still skips them with a stale comment, Go runs them
  live). Run `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p
  yorkie --test integration_history` against a server built at `54b96762`
  or later; if either fails, stop and report instead of re-ignoring.
- [ ] Also run the whole integration suite against that server once.
- [ ] Docs: remove the Out-of-Scope row for restore spans and the ledger;
  rewrite the Undo and Redo gap (Text undo works by identity now; Tree undo
  is still not ported, and a received `TreeEdit` with restore fields still
  runs only its positional part, so a JS Tree undo still diverges a Rust
  replica); update Text and RGATreeSplit (live-in-view rule ported),
  Garbage Collection (pruning and `held_from_purge` ported; revived-pair drop
  ported), Serialization (`removal_events`, restore spans, fixtures, no log
  line on a dropped ledger, the JS vs Go divergences above); add concept-map
  rows for `RestoreSpan`, `RestoreMode`, and the ledger methods (internal).
- [ ] Verification gate. Commit "Check Text undo interop and update the
  gaps".

## Self-review checklist (before the PR)

- `git grep -n "removed_at = Some"` in `rga_tree_split.rs` shows no write that
  bypasses `record_removal`/`recompute` outside decode.
- No `pub` item added outside `wire` (`cargo doc` public surface diff).
- `TreeEdit` encode/decode diff is empty.
- Fuzz required settings 0 failures at 1000 seeds.
