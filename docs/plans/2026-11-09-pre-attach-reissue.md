# Pre-Attach Ticket Re-issue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Status:** Planned (2026-11-09). Item 2 of 4 after JS parity (#1), stacked
on the Text removal ledger (item 1, `feat/text-removal-ledger`).

**Goal:** A document edited before `Client::attach` re-issues every ticket it
minted under the initial actor to the client's actor, as JS does. After the
attach, the pushed changes and the local root name the client's actor, the
local root is the root the server builds from those changes, and two clients
that filled the same key before attaching converge.

**Architecture:** A new `pub(crate)` module `yorkie-core/src/reissue.rs`
rewrites tickets by round-tripping each operation through the existing
`wire` projection (the Rust counterpart of JS's protobuf round trip) with an
exhaustive walk over every `Wire*` type. `Document` gains the never-synced
guard, the minted flag, and `reissue_actor`, which rebuilds the root and
presences by replaying the re-issued changes on a fresh `CrdtRoot` and swaps
everything in at once. `yorkie-client` gains the per-client claim
bookkeeping and calls the re-issue from `attach`. The Text wire projection
gains JS's insertion-link hardening, which the round trip depends on.

**Tech Stack:** Rust 1.99.1 (pinned in `rust-toolchain.toml`), existing crates
only.

**Spec:** this document, `docs/current-porting-gaps.md` (Out-of-Scope Items
row "Pre-attach ticket re-issue"; section Document Editing Model), JS
`docs/design/pre-attach-ticket-reissue.md`, and `docs/porting-from-js-sdk.md`.

References (read-only):

- JS SDK `../yorkie-js-sdk` at `8629993e` (written below as `js:`, paths under
  `packages/sdk/`). Origin: `ed25355e` "Re-issue pre-attach tickets to the
  client's actor on attach (#1449)". Later commits touching these files
  (`61ca06c9` in-place recovery, `b50f8900` presence patch, `0306c4b5`,
  `84068f70` ledger) do not change the re-issue itself; `84068f70` only adds
  ticket-carrying fields (`restore_spans`, `retombstone_spans`,
  `TextNode.removal_events`) that the reflective walk reaches for free and
  the Rust walk must name explicitly. Design:
  `docs/design/pre-attach-ticket-reissue.md`; task notes
  `docs/tasks/archive/2026/10/20261005-reissue-pre-attach-tickets-{todo,lessons}.md`
- Go `../yorkie` at `54b96762` (written `go:`). Go has the counterpart:
  `1061bc35` "Re-issue pre-attach tickets to the client's actor on attach
  (#2111)" — `api/converter/reissue.go` 47-239 (`ReissueOperations`,
  `walk`, `rekeyMap`, `walkNestedElement`),
  `pkg/document/internal_document.go` 312-395 (`setActorWithReissue`,
  `neverSynced`, `reissueOnlineClients`, `reissueVersionVector`),
  `pkg/document/document.go` 996-1060 (`SetActor`, `WithReissue`,
  `SetActorWithOptions`), `client/client.go` 945-966 (`claimReissue`). Design:
  `docs/design/pre-attach-ticket-reissue.md`.

Line ranges below are at those commits.

## Global Constraints

- JS is the behavioral source of truth; Go is the typed cross-check. Every
  rule starts from a named JS function or test. If JS and Go differ, follow
  JS and record the difference in `docs/current-porting-gaps.md` (the known
  ones are listed under "JS vs Go divergences" below).
- No new public API beyond what JS exposes. JS exposes nothing new to apps:
  `SetActorOptions`/`ReissueToken` are `@internal` and not re-exported, and
  `hasMintedOperations` is called only by `Client`. Rust follows the crate's
  existing pattern for client-only `Document` entry points
  (`apply_change_pack`, `garbage_collect`, `set_online_clients`): new
  `Document` methods the client needs are `#[doc(hidden)] pub`; everything
  else is `pub(crate)` or private. `reissue.rs` is a private module. No new
  `ClientOptions`/`AttachOptions` field (JS adds none).
- The re-issue rewrites a ticket iff its actor is the previous actor and its
  lamport is non-zero (`js:src/api/reissue.ts` 161-166). Lamports and
  delimiters are kept. Map keys are never rewritten.
- The swap is all or nothing: every new piece (changes, `CrdtRoot`, public
  `JsonObject` view, presences, online clients, `change_id`) is built in
  locals and assigned only after the last replay step succeeds. An error
  leaves the document byte-for-byte unchanged (`Document: PartialEq` makes
  this testable).
- The replay publishes no `DocEvent` (JS `reissueActor` publishes none). Do
  not route it through `Document::apply_changes`, which publishes
  `LocalChange` and presence events and syncs clocks.
- `yorkie-core` takes no new dependency; `yorkie-client` takes none either.
- No protocol change and no proto regeneration. Wire bytes of an operation
  that names no previous-actor ticket are unchanged; the only byte change is
  Task 1's insertion-link emit rule, which matches JS and Go.
- Toolchain is pinned to 1.99.1. Every task ends with:

  ```sh
  export PATH="$HOME/.cargo/bin:$PATH"
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```

  Live-server tests (Task 5) also run with
  `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie` against
  the v0.7.26 server running there.
- Each ported test names its JS (and, where used, Go) source in a doc comment
  above it, as in the existing ports.
- Commits are English, subject ≤70 chars, imperative verb, no type prefix;
  body lines ≤80 and say why. End every commit with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. There is no
  commit-msg hook here; check lengths by hand.
- Out of scope: `maxUndoDepth`/`clearHistory`/`disableGC` (item 3), Tree
  undo/redo (item 4), offline persistence and everything only it reaches
  (`fromBytes`, `restoreFromBytes`, `restoreAppendedChanges`, the
  serialized undo mark), epoch re-anchor (`resetForReanchor`), devtools
  (`resetDevtoolsRecording`, `toReplayBaseline`).

## Review Focus

Convergence-risk areas a reviewer must check line by line against JS:

1. **Walk coverage** (Task 2). Every `TimeTicket` a `WireOperation` or
   `WireJsonElement` can carry is visited: `executed_at`,
   `parent_created_at`, `prev_created_at`, `created_at`, Text/Tree node IDs
   and positions (`from`, `to`, `ins_prev_id`, `ins_next_id`,
   `merged_from`, `merged_at`), `removed_at`/`moved_at`, attribute
   `updated_at`, `WireRgaNode.position_*`, `TreeEdit.split_tickets`,
   `WireRestoreSpan.created_at` in both span lists, and
   `WireTextNode.removal_events`. Patterns are written without `..` so a new
   field fails to compile instead of silently escaping the walk (the Rust
   stand-in for JS's reflection). Nested elements recurse through
   `WireJsonElementSimple.element`.
2. **Text value special case** (Task 2). A `Set`/`Add`/`ArraySet` whose value
   is a `CrdtElement::Text` is re-issued through the full
   `WireJsonElement::Text` encoding (content, ledger, links), not through
   `WireJsonElementSimple`, which drops the content
   (`js:src/api/reissue.ts` 80-86, 109-151).
3. **`never_synced`** (Task 3). Detached, `Checkpoint::initial()`, the version
   vector names no actor but the current one, and `absorbed_remote` is
   false. `absorbed_remote` is set by `apply_snapshot` and by any change
   applied with `OpSource::Remote`, and by nothing else in Rust (no
   persistence, no re-anchor).
4. **Replay source** (Task 3). Each re-issued change replays with
   `Change::replay_source()` (`UndoRedo` for an undo/redo change) and keeps
   its `undo_redo` mark, or the Set/Remove skip guards run an operation that
   never ran.
5. **Version vector rewrite** (Task 3). `reissue_version_vector` moves the
   `from` entry to `to`, keeping the larger lamport if `to` already has one,
   for every change ID and for `change_id` itself. An entry left under the
   initial actor makes every replica wait on an actor that never syncs.
6. **Claim timing** (Task 4). `claim_reissue` is a pure query; the claim is
   taken after `set_actor_with_reissue` iff it re-issued or the document has
   minted operations, before the RPC, and at every place an attachment is
   dropped if the document minted operations while attached. Never released.
   Option validation (`resolve_document_poll_interval`) runs before both.
7. **Insertion-link decode** (Task 1). A link is kept only if it names, by
   exact ID, an already-decoded node of the same insertion at a lower offset;
   otherwise it is dropped, never an error. The encoder writes a link only if
   its target was emitted earlier in the same node list.
8. **Offset underflow** (Task 1). `rga_tree_split.rs` ~1720 computes
   `absolute_id.offset() - node.offset()` in `usize`; a forged link would
   underflow (panic in debug, wrap in release). It must become an
   `InvalidTextPosition`-style error, as JS `splitNode` rejects a negative
   offset.

## JS vs Go divergences (follow JS)

- Go re-keys a map entry whose key reads like the old actor (`rekeyMap`,
  test "rename an actor-keyed map entry"); JS leaves map keys alone, so a
  node attribute named like the actor is not renamed. Rust: never re-keys.
- Go records the claim on every attach (`c.minted[k] = actor`) and declines
  any repeat attach of the key under the same actor. JS records it only when
  the document re-issued or has minted operations, at attach or at detach,
  so a key whose first document minted nothing can still re-issue. Rust:
  JS rule.
- Go attaches under the per-session client ID and re-issues for any key. JS
  attaches under the stable actor of the client key and re-issues only for a
  generated key. Rust attaches under `response.actor_id` (the stable actor,
  as JS), so it follows JS's generated-key gate.
- Go sets its absorbed flag in `applyChanges` as a whole; JS only for a
  `Remote` change, because JS also replays local changes through
  `applyChanges`. Rust replays local changes through `apply_changes` too
  (`apply_snapshot`), so: JS rule.

## Rust divergences (record in the gaps doc)

- Ticket walk over the `wire` projection with exhaustive patterns instead of
  protobuf reflection. Same coverage, enforced at compile time; nested
  Object/Array/Tree values are structural in the projection, so there is no
  decode/re-encode of `JSONElement` bytes.
- No capability token. JS guards the option with a module-private symbol;
  Rust cannot share a crate-private value with `yorkie-client`, so the entry
  point is a `#[doc(hidden)] pub` method like the other client-only ones.
  JS test "refuses a re-issue asked for without the token" has no
  counterpart (see Open Decisions).
- `generate_client_key` uses `getrandom` and has no weak fallback, so JS's
  "refuses to generate a key when it cannot be unguessable" path does not
  exist; a missing OS source still panics as today.
- Plain `set_actor` already moves the self presence and online entry (an
  existing recorded divergence); the re-issue path replaces presences with
  the replay result, as JS.
- Rust Tree edits push no undo entry (item 4), so the Tree part of "keeps
  tree splits, undo/redo, tree style and array set" covers splits and style
  only.

## Open Decisions

- **Re-issue entry point shape.** Default: `#[doc(hidden)] pub fn
  set_actor_with_reissue(&mut self, actor_id) -> Result<bool>` next to the
  unchanged `set_actor`. Alternative (needs the maintainer's yes): a
  `#[doc(hidden)] pub struct ReissueToken` with a private field and a
  `#[doc(hidden)]` constructor, passed in a `SetActorOptions`, mirroring JS's
  shape without its guarantee.
- **Error variant for a failed re-issue.** JS throws `ErrInvalidArgument`
  when an operation does not decode. Default: reuse the `YorkieError` the
  failing wire conversion returns and let `attach` surface it before any
  RPC; add no new variant.
- **Seeded history test size.** JS runs 300 seeds of `fillRandomly`. Default:
  300 seeds in `cargo test` if it stays under ~5 s in debug; otherwise 50 in
  the default run and 300 behind `#[ignore]` with a note.

---

## Milestone 1: Core

### Task 1: Keep Text insertion links resolvable on the wire

**Goal:** The Text wire projection writes and reads `ins_prev_id` exactly as
JS `ed25355e` does, so a Text survives the re-issue round trip (and every
snapshot) with its split links, and a forged link cannot mis-link a node.

**Files:**
- Modify: `crates/yorkie-core/src/wire.rs` (`wire_text_from_domain` ~977
  and `wire_text_node_from_domain` ~1299: emit a link only if its target ID
  was emitted earlier in the same list;
  `text_from_wire_nodes` ~1343-1380: exact-ID, same-insertion, lower-offset,
  already-decoded rule, else drop)
- Modify: `crates/yorkie-core/src/crdt/rga_tree_split.rs` (~1720: checked
  offset subtraction; `split_node` ~1756 error text as JS)
- Modify: the existing wire unit tests module (or create
  `crates/yorkie-core/src/wire_text_link_tests.rs`, `#[cfg(test)]`)

**Port from:**
- `js:src/api/converter.ts` 768-800 (`toTextNodes`, the `emitted` set),
  1925-1975 (`fromText`, the `decoded` map and the link rule).
- `js:src/document/crdt/rga_tree_split.ts` 2045-2070 (`splitNode`, negative
  offset).
- `js:src/document/crdt/split_links.ts` 24-57 (why a Text payload keeps its
  links while tree links are dropped; confirm Rust
  `drop_split_links_in_element` leaves Text alone).
- Cross-check `go:api/converter/from_pb.go` text node decode (Go rejects an
  unresolvable link; JS drops it — follow JS).

**Behavior:**
- Encode: `ins_prev_id` is `Some` only when the node has a link and that ID
  appears earlier in the same node list. A dangling link into a purged
  tombstone is dropped.
- Decode: keep a link iff `created_at` equals the node's, its offset is lower
  than the node's, and a node with exactly that ID was decoded earlier;
  otherwise leave the link unset. Never fail the element.
- A position resolved through a link into a higher-offset node returns an
  error instead of underflowing.

**Tests** (from `js:test/unit/api/snapshot_converter_test.ts`, at the
projection level):
- `should round-trip the insertion links of split text nodes`
- `should only emit an insertion link a decoder can resolve`
- `should drop an insertion link that names no decoded node`
- `should drop an insertion link that names another insertion`
- New: a forged higher-offset link followed by an edit returns an error and
  does not panic.

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Keep Text insertion links resolvable on the
  wire".

### Task 2: Re-issue operation tickets to another actor

**Goal:** `reissue_operations(ops, from, to) -> Result<Vec<Operation>>` and
`reissue_version_vector(vector, from, to) -> VersionVector`.

**Files:**
- Create: `crates/yorkie-core/src/reissue.rs` (private module, declared in
  `lib.rs`; `TicketReissuer` with `walk_operation`, `walk_element_simple`,
  `walk_element`, `walk_text_node`, `walk_tree_node`, `walk_span`,
  `ticket`; `reissue_text_value`)
- Create: `crates/yorkie-core/src/reissue_tests.rs` (`#[cfg(test)]`)

**Port from:**
- `js:src/api/reissue.ts` 43-89 (`reissueOperations`), 95-151
  (`TicketReissuer.reissueTextValue`), 153-202 (`walk`), 204-225
  (`walkNestedElement`).
- `js:src/document/document.ts` 699-722 (`reissueVersionVector`).
- Cross-check `go:api/converter/reissue.go` 47-239 (skip `rekeyMap`).

**Behavior:**
- For each operation: `WireOperation::try_from(&op)`, walk, then
  `Operation::try_from(wire)`. The input is borrowed and never mutated.
- A ticket is rewritten iff `lamport != 0 && actor == from`.
- `Set`/`Add`/`ArraySet` whose original value is Text: encode the original
  value with `WireJsonElement::try_from`, walk it, decode with
  `CrdtElement::try_from`, and rebuild the decoded operation with that value
  (same key/parent/prev/created_at/executed_at).
- `reissue_version_vector`: copy; if `from` is absent return the copy;
  otherwise unset `from` and set `to` to the max of the two lamports.
- Every operation kind round-trips; any conversion error is returned, not
  swallowed.

**Tests** (from `js:test/unit/api/reissue_test.ts` "reissueOperations"):
- `re-issues the tickets nested in Object, Array and Tree values` (also
  asserts the lamport-0 root ticket is untouched, as Go
  "re-issue tickets of a nested value and keep the lamport-0 root ticket").
- `does not mutate the given operations`.
- `keeps the content of a Text value`.
- `leaves an attribute named like the actor alone`.
- New (no JS case; JS gets it from reflection): an Edit carrying
  `restore_spans` and `retombstone_spans`, a TreeEdit with `split_tickets`,
  and a Set of a Text whose node holds `removal_events` come back with every
  previous-actor ticket rewritten.
- `reissue_version_vector`: absent `from`; `to` already larger; `to`
  smaller.

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Re-issue operation tickets to another
  actor".

### Task 3: Re-issue a never-synced document's tickets

**Goal:** `Document` re-issues its local changes, root, and presences to a
new actor when, and only when, it never synced.

**Files:**
- Modify: `crates/yorkie-core/src/document.rs` (fields `absorbed_remote`,
  `minted_operations`; set them in `apply_snapshot`, `apply_changes`
  (`Remote` only), and wherever a local change is pushed (`update` path
  ~237, `execute_undo_redo` ~440); `never_synced`, `reissue_actor`,
  `#[doc(hidden)] pub fn set_actor_with_reissue`, `#[doc(hidden)] pub fn
  has_minted_operations`; update the `set_actor` doc comment)
- Modify: `crates/yorkie-core/src/change/change.rs` (a constructor or setter
  that builds a change from a new ID and operations while keeping the
  presence change, message, and `undo_redo` mark)
- Create: `crates/yorkie-core/src/document/reissue_tests.rs` (`#[cfg(test)]`,
  declared like the sibling test modules)

**Port from:**
- `js:src/document/document.ts` 144-172 (`ReissueToken`,
  `SetActorOptions`), 814-829 (`absorbedRemote`, `mintedOperations`), 1170
  and 3936 (`mintedOperations ||=`), 2484-2512 (`setActor`), 2514-2600
  (`reissueActor`), 2627-2659 (`neverSynced`, `hasMintedOperations`), 2855-2864
  (`applySnapshot` sets absorbed), 2912-2995 (`applyChanges` sets absorbed
  for `Remote`).
- Cross-check `go:pkg/document/internal_document.go` 312-395.

**Behavior:**
- `set_actor_with_reissue(actor) -> Result<bool>`: if `reissue_actor`
  returns `true`, done; otherwise behave exactly as `set_actor` and return
  `false`.
- `reissue_actor`: return `Ok(false)` without touching anything when
  `prev == actor`, `local_changes` is empty, or `!never_synced()`. Otherwise
  build each new change (ID with the new actor and re-issued vector,
  `reissue_operations`, same presence change, message, `undo_redo`), replay
  them in order on `CrdtRoot::create()` with `change.execute(&mut root,
  change.replay_source())` and `apply_presence_change` into a fresh map,
  build the `JsonObject` view, move `prev` to `actor` in `online_clients`,
  re-issue `change_id`'s actor and vector, then assign all of it and clear
  the undo and redo stacks. Return `Ok(true)`.
- `never_synced`: per Review Focus 3.
- `minted_operations` is never cleared.

**Tests** (from `js:test/unit/document/set_actor_reissue_test.ts`
"Document.setActor with reissue"; helpers `fill_everything`,
`actors_of` (count actors over the wire projection of the root and of
`create_change_pack`), `server_build` (apply the wire round trip of each
document's local changes as `Remote` to a fresh document),
`assert_local_vectors`, and a seeded `fill_randomly` with the same
mulberry32 PRNG):
- `leaves no ticket under the initial actor`
- `builds the same root the server builds` (compare
  `WireJsonElement::try_from(root)` equality instead of snapshot bytes)
- `continues edits under the new actor`
- `re-issues a retried attach to the next actor`
- `clears the undo history it invalidates`
- `replays an undone change under the source it ran with`
- `replays an undone change under its own source after a snapshot`
- `does not re-issue a document that has synced`
- `does not re-issue a document that absorbed a snapshot`
- `is plain setActor without the option`
- `only sets the actor of an empty document`
- `keeps the content of a Text restored by undo (edit after: false/true)`
- `keeps the split links of a Text restored by undo (wrap: none/object/array)`
- `keeps tree splits, undo/redo, tree style and array set` (Tree undo parts
  dropped; see divergences)
- `converges two documents that filled the same key`
- `renames an online client entry to the re-issued actor`
- `leaves the document untouched when the replay fails`: Rust cannot mock
  `SetOperation::execute`; push a crafted local change whose operation
  fails on a fresh root through a `#[cfg(test)]` helper, and assert the
  document `==` its clone taken before, actor still initial, `can_undo`
  still true.
- `keeps the content of random pre-attach histories` (seed count per Open
  Decisions).
- Not ported (no Rust counterpart): `carries the undo mark through the
  persisted change log`, `does not re-issue a document restored from bytes`,
  `reports a document decoded from bytes as having minted`, `refuses a
  re-issue asked for without the token`.
- Cross-check with Go `TestSetActorWithReissue` subtests of the same names.

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Re-issue a never-synced document's tickets".

## Milestone 2: Client

### Task 4: Re-issue pre-attach tickets on attach

**Goal:** `Client::attach` re-issues a never-synced document to the client's
actor under JS's claim rules.

**Files:**
- Modify: `crates/yorkie-client/src/client.rs` (`key_generated: bool` on the
  client inner, set in `with_transport` from whether `options.key` was
  `None` or empty; `reissue_claims: BTreeMap<String, ActorId>` that survives
  deactivate/activate; `claim_reissue`, `mark_reissue_claim`; `attach` ~423
  calls `set_actor_with_reissue` when `claim_reissue` allows, else
  `set_actor`, then marks; `remove_attachment` ~1350 and
  `deactivate_internal` ~367 mark when the document has minted operations;
  update the `attach` doc comment)

**Port from:**
- `js:src/client/client.ts` 696-704 (`keyGenerated`, `reissueClaims`),
  775-800 (constructor), 1107-1156 (`claimReissue`, `markReissueClaim`),
  1222-1247 (`attach`), 3835-3849 (`detachInternal` claim).
- Cross-check `go:client/client.go` 945-966 (Go rule differs; follow JS).

**Behavior:**
- `claim_reissue(actor, key)`: `false` if the key was not generated;
  otherwise `reissue_claims.get(key) != Some(actor)`. Pure query.
- In `attach`, after option validation and before the presence update and
  the RPC: re-issue (or plain set), then `mark_reissue_claim` iff it
  re-issued or `has_minted_operations()`. An error from the re-issue returns
  from `attach` before any RPC, the claim untaken.
- Wherever an attachment is dropped (detach, remove, server-side removal,
  deactivate), take the claim with the client's actor (not the document's,
  which `apply_status(Detached)` reset) iff the document has minted
  operations.
- A failed attach RPC leaves the re-issued document as is (no rollback).

**Tests** (from `js:test/unit/client/reissue_claim_test.ts` "Client.attach
re-issues pre-attach tickets", with the stub transport in
`crates/yorkie/tests/integration_support/stub_transport.rs` or the
`client.rs` test transport, whichever the existing claim-free attach tests
use):
- `pushes the pre-attach changes under the client actor`
- `keeps the re-issued state when the attach fails`
- `does not re-issue when the attach options are rejected` (zero
  `document_poll_interval`)
- `declines a second never-synced document of a key on one client`
- `re-issues a later document of a key the first left untouched`
- `declines a later document when the first minted while attached`
- `re-issues a second never-synced document of another key`
- `does not re-issue under an explicit client key`
- Not ported: `refuses to generate a key when it cannot be unguessable`
  (no weak fallback in Rust).
- Cross-check Go `TestClaimReissue`.

- [ ] RED, port, GREEN.
- [ ] Verification gate. Commit "Re-issue pre-attach tickets on attach".

### Task 5: Test against a server and update the gaps

**Goal:** Live-server evidence and the gap docs rewritten.

**Files:**
- Create: `crates/yorkie/tests/integration_pre_attach.rs`
- Modify: `docs/current-porting-gaps.md`, `docs/porting-from-js-sdk.md`

**Steps:**
- [ ] Port `js:test/integration/pre_attach_test.ts` "Pre-attach edits",
  gated on `rpc_addr()` like the other integration files:
  `converges two clients that filled the same key before attach`,
  `re-issues pre-attach tickets to the client actor`, `does not re-issue a
  second pre-attach document of a key`, `keeps an earlier session of an
  explicit client key`. Cross-check Go `test/integration/pre_attach_test.go`.
- [ ] Run `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie
  --test integration_pre_attach`, then the whole `-p yorkie` suite once. If
  a case fails against v0.7.26, stop and report instead of ignoring it.
- [ ] Docs: remove the Out-of-Scope row "Pre-attach ticket re-issue";
  rewrite the Document Editing Model bullet to describe the ported
  behavior and what remains (an explicit client key keeps the initial
  actor, as in JS; positions taken before the attach no longer resolve;
  no capability token; the divergences above). Add the insertion-link rule
  to Serialization and Protocol Conversion and the claim bookkeeping to
  Client, Sync, and Watch. Add concept-map rows for `reissueOperations`,
  `reissueVersionVector`, `setActor` with `reissue`, `neverSynced`,
  `hasMintedOperations`, `claimReissue`, `markReissueClaim` (internal).
- [ ] Verification gate. Commit "Test pre-attach re-issue against a server".

## Self-review checklist (before the PR)

- `git grep -n "\.\." crates/yorkie-core/src/reissue.rs` shows no `..` rest
  pattern on a `Wire*` destructure.
- No `pub` item added outside the `#[doc(hidden)]` Document/Client entry
  points (`cargo doc` public surface diff).
- `reissue_actor` touches `self` only in its final assignment block.
- No `DocEvent` is published by the re-issue path.
