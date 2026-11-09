# Small JS API Gaps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Status:** Planned (2026-11-09). Item 3 of 4 after JS parity (#1).

**Goal:** Close the small public-API rows of the Out-of-Scope table in
`docs/current-porting-gaps.md`: history options (`maxUndoDepth`,
`clearHistory`), the two `disableGC` options, `ChannelStatus.Removed`, the
Text and Tree position-range helpers, `Tree.styleByPath` and
`removeStyleByPath`, and the `source` of change events. Each behaves as in
JS.

**Architecture:** Mostly facade work. `yorkie-core` gains a public
`DocumentOptions`, `Document::with_options`, `Document::clear_history`, a
lamport-only clock sync for GC-disabled attachments, public position structs
(`TextPosStruct`, `TreePosStruct`, ...), new `JsonText`/`JsonTree` methods,
and a public `OpSource`. `yorkie-client` gains `AttachOptions::disable_gc`,
carried on `AttachDocument` and every `PushPullChanges` of that attachment,
and `ChannelStatus::Removed`. No proto or wire-shape change: the vendored
protos already carry `disable_gc` on both requests.

**Tech Stack:** Rust 1.99.1 (pinned in `rust-toolchain.toml`), existing
crates only.

**Spec:** this document, `docs/current-porting-gaps.md` (Out-of-Scope rows
"Disable-GC attach option", "History options", "`ChannelStatus.Removed`",
"Text and Tree range helpers"; sections Public JSON API, Garbage Collection,
Undo and Redo, Channels and Broadcast), and `docs/porting-from-js-sdk.md`.

References (read-only):

- JS SDK `../yorkie-js-sdk` at `8629993e` (written `js:`, paths under
  `packages/sdk/`), plus `origin/main` `f70e9f16` "Reject styleByPath(path)
  for a path that names no node (#1524)", whose behavior Task 4 ports
  (written `js@f70e9f16:`). `json/tree.ts` and `json/text.ts` are the same at
  both commits.
- Go `../yorkie` (written `go:`), only as a cross-check for Tree path
  helpers if JS is ambiguous.

Line ranges below are at those commits.

**Stacking:** this branch (`feat/small-api-gaps`) sits on
`feat/reissue-actor`, which is still moving. Plan against current code; do
not call or depend on re-issue internals. Where JS clears history after a
re-issue (`js:src/document/document.ts` 2578-2588), that call belongs to the
re-issue branch; Task 1 only provides `clear_history` for it.

## Global Constraints

- JS is the behavioral source of truth. Every rule starts from a named JS
  function or test. Go is a cross-check for Tree path helpers only.
- Public Rust API mirrors JS names in Rust style and the existing facade
  conventions: snake_case methods, no `get_` prefix for plain accessors
  (`JsonTree::size` for `getSize`, `JsonText::len` for `length`), `Result`
  for anything JS can throw from, index paths as `&[usize]`/`Vec<usize>`,
  ranges as tuples (`(usize, usize)`, `(Vec<usize>, Vec<usize>)`), error
  kinds `YorkieError::InvalidArgument` for JS `ErrInvalidArgument` and
  `YorkieError::NotInitialized` for `ErrNotInitialized`. JS overloads become
  separate methods (see Open Decisions).
- Include only public API JS exposes. Not ported, with reason recorded in
  the gap doc: `Tree.getIndexTree` and `Text.getTreeByIndex`/`getTreeByID`
  (return internal index structures), `toTestString`, `toJSForTest`,
  `toJSInfoForTest`, `createRangeForTest`, `History.getUndoStackForTest`/
  `getRedoStackForTest`, `DocumentOptions.disablePresence` and
  `enableDevtools`.
- No proto edit, no regeneration. `disable_gc` already exists on
  `api::AttachDocumentRequest` and `api::PushPullChangesRequest`.
- `yorkie-core` takes no new dependency.
- Out of scope: Tree undo/redo (item 4), offline persistence, devtools,
  revisions, epoch re-anchor (and with it the minting-suspended refusal).
- Toolchain is pinned to 1.99.1. Every task ends with:

  ```sh
  export PATH="$HOME/.cargo/bin:$PATH"
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```

  Tasks that touch the client or add integration tests also run, against
  the local v0.7.26 server:

  ```sh
  YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie
  ```

- Each ported test names its JS source (file and `it` title or line) in a
  doc comment above it, as in the existing ports.
- Commits are English, subject ≤70 chars, imperative verb, no type prefix;
  body lines ≤80 and say why. End every commit with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. There is no
  commit-msg hook here; check lengths by hand.

## Review Focus

1. **History depth applies to both stacks** (Task 1). JS `History.pushUndo`
   and `pushRedo` each drop the oldest entry at `maxDepth`
   (`js:src/document/history.ts` 81-106). The Rust stack helper currently
   reads the constant `MAX_UNDO_REDO_STACK_DEPTH` (`history.rs` ~175); the
   redo stack must use the document's depth too (JS test "bounds the redo
   stack by the same depth, not the default one").
2. **`clear_history` after attach** (Task 1). JS calls `doc.clearHistory()`
   after the initial-root update in `attachDocument`, whether or not
   `initialRoot` was given (`js:src/client/client.ts` 1837-1853). Rust
   `Client::attach` (`client.rs` ~498-511) does not; an initial root is
   undoable today. This is a behavior fix inside Task 1.
3. **Two distinct `disableGC`s** (Task 2). `DocumentOptions.disableGC` only
   makes `garbageCollect` return 0 (`js:src/document/document.ts`
   2780-2791). `AttachOptions.disableGC` is the wire contract: sent on
   attach and every push-pull of that attachment, and it switches
   `applyChange` to `syncLamport` (3064-3066). Neither implies the other.
   Do not merge them into one flag.
4. **`sync_lamport` before the attach response is applied** (Task 2). JS
   sets the flag before `applyChangePack` on the attach response
   (`client.ts` 1628-1647) so the first remote changes already take the
   lamport-only path, and sets it on every attach, so a re-attach without
   the option goes back to `syncClocks`.
5. **Read-path splits stay on the facade copy** (Task 3). JS
   `posRangeToIndexRange`/`posRangeToPathRange` split text nodes of the
   live clone root and register the born-removed GC pairs
   (`js:src/document/json/tree.ts` 1080-1098, `registerPendingGCPairs`).
   A Rust `JsonTree` holds a copy; a split there never reaches the
   document root, so the pending pairs are drained and dropped, as
   `JsonTree::style` already does (`json/tree.rs` ~288). The reviewer
   checks that no conversion mutates `CrdtRoot`, and that `garbage_len`
   and `doc_size` after a conversion equal a rebuild.
6. **`find_pos` vs `findPosUnchecked`** (Tasks 3, 4). JS range conversions
   and the end of `pathToPosRange` use `findPosUnchecked` (no surrogate
   check); edits use `findPos`. Rust `path_to_pos_range` (`crdt/tree.rs`
   ~861) calls `find_pos` for both ends. Port JS
   `surrogate_index_test.ts:666` ("styles a single path whose derived end
   lands inside a pair") and make it pass without loosening `find_pos` for
   edits.
7. **`style_by_path` single form bounds** (Task 4). Port
   `js@f70e9f16:src/document/crdt/tree.ts` `pathToPosRange` exactly: an
   element parent rejects `offset >= visible children`, a text-only parent
   rejects `offset >= visible size` only when the text node has no next
   sibling. The range form and `path_to_pos` stay laxer (the end offset is a
   valid insertion point).
8. **Change-event `source`** (Task 5). A replayed local change after a
   snapshot reports the source it first ran with (JS
   `replayLocalChanges` → `replaySourceOf`), not `Local`.

## Deliberate differences from JS

- **Undo/redo during an update is unrepresentable.** `Document::update`
  borrows the document mutably for the closure, so the closure cannot call
  `history_mut()` (or `history()` for `can_undo`): the JS `ErrRefused`
  path (`js:src/document/document.ts` 3606-3612, `canUndo` 970-978) cannot
  be reached. With a `SharedDocument`, locking the same document inside the
  closure deadlocks (std `Mutex` is not reentrant), as with any other
  re-entrant lock. Nothing is ported; Task 6 records it.
- **`maxUndoDepth` type.** Rust takes `Option<usize>`, so JS's
  non-integer, negative, `NaN`, and infinite rejections cannot occur; only
  `Some(0)` is rejected, with JS's message.
- **Read helpers outside `update`.** JS helpers throw `ErrNotInitialized`
  without a context, but JS `doc.getRoot()` provides one, so they work on a
  root read outside `update`. Rust's existing `JsonTree::index_to_path`/
  `path_to_index` work on any tree that belongs to a document; the new
  conversion helpers follow that rule and fail with `NotInitialized` only
  for a tree or text not yet in a document. The mutating `style_by_path`/
  `remove_style_by_path` need the recorder, like `style`.
- **Negative path elements** cannot occur (`&[usize]`), so the negative
  cases of `tree_style_path_bounds_test.ts` 116, 177, 209-280 have no port;
  the doc comment of the bounds test module says so.

## Open Decisions

- **`styleByPath` overload naming.** Default: `style_by_path(path, attrs)`
  for the single form and `style_by_path_range(from, to, attrs)` for the
  range form; `remove_style_by_path(from, to, keys)` (JS has only the range
  form). Alternative: one `style_by_path(from, Option<to>, attrs)`.
- **Where `source` lives.** Default: `pub source: OpSource` on `ChangeInfo`
  (the value of `LocalChange`/`RemoteChange`), and `OpSource` re-exported
  from `yorkie-core`. JS puts `source` on every event, including
  `Snapshot` (always `Remote`); Rust leaves `Snapshot` and presence events
  without it. Alternative: struct variants on `DocEvent`, which breaks
  every match arm in callers.
- **`ChannelStatus::Removed` is dead in JS.** No JS code sets it
  (`git grep "ChannelStatus.Removed" -- packages/sdk/src` is empty). Default:
  add the variant with JS's doc text and set it nowhere, so a Rust match is
  exhaustive over the same three states. Alternative: keep it unported and
  record "unused in JS" in the gap doc.
- **Small Tree accessors.** Default: also add `JsonTree::id` (JS `getID`,
  like `JsonText::id`), `node_size` (JS `getNodeSize`), and
  `root_tree_node` (JS `getRootTreeNode`, returning `TreeNodeInput`). Drop
  them if the reviewer wants only the listed gap rows.
- **`DocumentOptions` shape.** Default: `#[derive(Default)]` struct with
  `pub disable_gc: bool` and `pub max_undo_depth: Option<usize>`,
  `#[non_exhaustive]` so `disablePresence` can be added later; built with
  `DocumentOptions { max_undo_depth: Some(5), ..Default::default() }`.

---

## Milestone 1: Document options

### Task 1: Add the undo depth option and clear_history

**Goal:** A document can be created with a positive `max_undo_depth` that
bounds both stacks, and `Document::clear_history` empties both stacks, as JS
does. Attach clears history after the initial root.

**Files:**
- Modify: `crates/yorkie-core/src/history.rs` (`History::new(max_depth)`,
  `max_depth()` (`pub(crate)`), stack helper takes the depth; keep
  `MAX_UNDO_REDO_STACK_DEPTH` as the default)
- Create: `crates/yorkie-core/src/document/options.rs` or add to
  `document.rs` (`pub struct DocumentOptions`)
- Modify: `crates/yorkie-core/src/document.rs` (`Document::with_options`,
  `Document::new` delegates with defaults, `clear_history`, `apply_snapshot`
  calls `clear_history`)
- Modify: `crates/yorkie-core/src/lib.rs` (re-export `DocumentOptions`)
- Modify: `crates/yorkie-client/src/client.rs` (`attach` calls
  `clear_history` after the initial-root update, unconditionally)
- Create: `crates/yorkie-core/tests/undo_depth.rs` (or a `#[cfg(test)]`
  module beside `history.rs`; keep the History unit cases there)

**Port from:**
- `js:src/document/document.ts` 107-142 (`DocumentOptions`,
  `maxUndoDepth` doc), 945-956 (constructor validation, message
  `maxUndoDepth must be a positive integer: ${value}`, code
  `ErrInvalidArgument`), 2898-2907 (`clearHistory`), 2883
  (`applySnapshot` clears).
- `js:src/document/history.ts` 39-62 (`MaxUndoRedoStackDepth`, constructor,
  `getMaxDepth`), 81-106 (`pushUndo`/`pushRedo` drop the oldest at
  `maxDepth`), 113-125 (`clearRedo`, `clearUndo`).
- `js:src/client/client.ts` 1837-1853 (`clearHistory` after the initial
  root).

**Behavior:**
- `Document::with_options(key, DocumentOptions) -> Result<Document>`;
  `max_undo_depth: None` means 50; `Some(0)` returns
  `YorkieError::InvalidArgument("maxUndoDepth must be a positive integer:
  0")`. `Document::new(key)` keeps its signature and infallibility.
- Each local change with operations is one undo entry; when a push would
  exceed the depth, the oldest entry is dropped first. Redo uses the same
  depth.
- `clear_history` drops both stacks; afterwards `can_undo`/`can_redo` are
  false. It does not touch the document, local changes, or presence.
- `Client::attach` calls it after the optional initial-root update.

**Tests:**
- `js:test/unit/document/undo_depth_test.ts` "Document maxUndoDepth"
  (54-115): `keeps the default depth when the option is not given`,
  `keeps as many undo entries as the option allows`, `drops the oldest
  entries first when the depth is small`, `bounds the redo stack by the
  same depth, not the default one`, and `rejects a maxUndoDepth of 0`
  (the other four values of the loop at 95 are unrepresentable; say so).
  The helper `typed()` (29-40) calls `clearHistory` after setup; port it.
- Same file, "History depth" (117-160): all five cases as `History` unit
  tests.
- Add one Rust case: an attach with `initial_root` leaves `can_undo` false
  (integration, live server), citing `client.ts` 1850-1853.

- [ ] Write the tests RED, implement, go GREEN.
- [ ] Verification gate, plus the live-server run.
- [ ] Commit "Add the undo depth option and clear_history".

### Task 2: Port the two disableGC options

**Goal:** `DocumentOptions::disable_gc` turns local garbage collection off;
`AttachOptions::disable_gc` declares the attachment GC-free to the server
and keeps the document's version vector at one entry.

**Files:**
- Modify: `crates/yorkie-core/src/document.rs` (`disable_gc` from options
  makes `garbage_collect` return 0; new `attach_disable_gc` flag with a
  `#[doc(hidden)] pub fn set_disable_gc(bool)` for the client;
  `apply_changes` uses `sync_lamport` when it is set)
- Modify: `crates/yorkie-core/src/change/id.rs` (`ChangeId::sync_lamport`)
- Modify: `crates/yorkie-client/src/options.rs` (`AttachOptions::disable_gc:
  bool`, default false, doc text from JS)
- Modify: `crates/yorkie-client/src/transport.rs`,
  `crates/yorkie-client/src/protocol.rs` (`disable_gc` on
  `AttachDocumentRequest` and `PushPullChangesRequest`, encoded into the
  proto field)
- Modify: `crates/yorkie-client/src/attachment.rs` (`disable_gc` on
  `Attachment`), `crates/yorkie-client/src/client.rs` (attach sends it and
  calls `set_disable_gc` before applying the response pack; `push_pull`
  sends `attachment.disable_gc`)
- Create: `crates/yorkie/tests/integration_disable_gc.rs`
- Modify: the ignored Rust port of JS `object_test.ts:923` (grep
  `object_test.ts:923` under `crates/yorkie/tests`): create its document
  with `disable_gc: true` and remove the `#[ignore]`.

**Port from:**
- `js:src/document/document.ts` 110-114 (`DocumentOptions.disableGC`),
  897-902 and 939 (attach flag), 2660-2668 (`setDisableGC`), 2780-2791
  (`garbageCollect` returns 0 under `opts.disableGC`, before touching the
  clone or the root), 3064-3066 (`syncLamport` vs `syncClocks`).
- `js:src/document/change/change_id.ts` 133-154 (`syncLamport`: no-op when
  `other` has no clocks; lamport = max(other, self) + 1 rule exactly as
  written; the vector is a copy of the receiver's with only its own actor
  set to the new lamport).
- `js:src/client/client.ts` 476-486 (`AttachOptions.disableGC` doc),
  1588 (`disableGc` on the attach request), 1628-1647 (`setDisableGC`
  before `applyChangePack`, on both attach paths), 1698-1708
  (`Attachment` keeps it), 4025-4037 (`disableGc: attachment.disableGC` on
  every push-pull).

**Behavior:**
- Options default to false; with both false nothing changes on the wire or
  in the clocks (the proto field is already sent as `false` today).
- `apply_change_pack` still calls `garbage_collect` with the server's
  minimum vector; under `DocumentOptions::disable_gc` it returns 0 and
  purges nothing. Undo or redo is unaffected.
- `snapshot` handling keeps `set_clocks` (JS 2865); only the per-change
  path switches.
- Detach does not reset the flag; the next attach sets it from its own
  options.

**Tests:**
- `js:test/unit/document/gc_test.ts` "should not collect garbage if
  disabled" (67-~95) as a `yorkie-core` test.
- `ChangeId::sync_lamport` unit tests: no clocks → unchanged; lamport rule
  for `other` ahead and behind; the vector holds only the own actor's entry
  after syncing with a multi-entry `other`.
- `js:test/integration/disable_gc_test.ts` (31-260), all five cases:
  `Can attach a Counter document with disableGC and sync without error`,
  `Mixed opt-in and opt-out clients converge on the Counter value`,
  `Opt-out clients keep per-Change VV at size 1 under multi-actor fanout`
  (uses the hidden `Document::version_vector`), `Opt-out client picks up
  server lamport when attach returns a snapshot`, `Re-attach without
  disableGC restores normal sync behavior`.
- `protocol.rs` unit test: both requests encode `disable_gc`.

- [ ] Write the tests RED, implement, go GREEN.
- [ ] Verification gate, plus the live-server run.
- [ ] Commit "Port the disableGC document and attach options".

## Milestone 2: Text and Tree helpers

### Task 3: Add Text and Tree position-range conversions

**Goal:** `JsonText::index_range_to_pos_range`/`pos_range_to_index_range`
and `JsonTree::index_range_to_pos_range`, `pos_range_to_index_range`,
`path_range_to_pos_range`, `pos_range_to_path_range`, with public struct
types that match JS `TextPosStruct`/`CRDTTreePosStruct` field for field.

**Files:**
- Create: `crates/yorkie-core/src/json/pos.rs` (public `TextNodeIdStruct`
  {`created_at: TimeTicketStruct`, `offset: usize`}, `TextPosStruct` {`id`,
  `relative_offset`}, `TreeNodeIdStruct` {`created_at`, `offset`},
  `TreePosStruct` {`parent_id`, `left_sibling_id`}; `pub type
  TextPosStructRange = (TextPosStruct, TextPosStruct)` and
  `TreePosStructRange`)
- Modify: `crates/yorkie-core/src/crdt/rga_tree_split.rs`
  (`RgaTreeSplitPos::to_struct`/`from_struct`, node ID likewise)
- Modify: `crates/yorkie-core/src/crdt/tree.rs` (`TreePos`/`TreeNodeId`
  `to_struct`/`from_struct`; `index_range_to_pos_range` with an unchecked
  position lookup; `pos_range_to_index_range` and `pos_range_to_path_range`
  through `find_nodes_and_split_text` + the existing `to_index` /
  `tree_pos_to_path`)
- Modify: `crates/yorkie-core/src/json/text.rs`, `json/tree.rs`,
  `json/mod.rs`, `lib.rs` (methods and re-exports)
- Create: `crates/yorkie-core/src/json/pos_tests.rs` (`#[cfg(test)]`)
- Modify: `crates/yorkie/tests/integration_tree.rs`

**Port from:**
- `js:src/document/json/text.ts` 43-56 (`TextPosStruct`,
  `TextPosStructRange`), 210-246 (`indexRangeToPosRange`,
  `posRangeToIndexRange`).
- `js:src/document/crdt/text.ts` 677-687 (`indexRangeToPosRange`: same
  position for equal indexes, no surrogate check), 896-898
  (`findIndexesFromRange`).
- `js:src/document/crdt/rga_tree_split.ts` 111-165
  (`RGATreeSplitNodeID.fromStruct`/`toStruct`), 205-265 (`RGATreeSplitPos`
  struct), 1729-1745 (`findIndexesFromRange`, `posToIndex`: unknown ID is
  `ErrInvalidArgument`).
- `js:src/document/json/tree.ts` 1002-1098 (`pathRangeToPosRange`,
  `indexRangeToPosRange`, `posRangeToIndexRange`, `posRangeToPathRange`,
  `registerPendingGCPairs`).
- `js:src/document/crdt/tree.ts` 219-245 (`CRDTTreePos.fromStruct`/
  `toStruct`), 384-450 (`CRDTTreeNodeID` struct), 465-498 (struct types),
  4872-4965 (`toPath`, `toIndex`, `indexRangeToPosRange`,
  `indexRangeToPosStructRange`, `posRangeToPathRange`,
  `posRangeToIndexRange`).

**Behavior:**
- `JsonText::index_range_to_pos_range(&self, (usize, usize)) ->
  Result<TextPosStructRange>`; `pos_range_to_index_range(&self,
  &TextPosStructRange) -> Result<(usize, usize)>` (no split in JS or Rust).
- `JsonTree::index_range_to_pos_range(&self, (usize, usize))`,
  `path_range_to_pos_range(&self, (&[usize], &[usize]))` (path → index →
  pos, as JS), `pos_range_to_index_range(&mut self, &TreePosStructRange)`,
  `pos_range_to_path_range(&mut self, &TreePosStructRange)`. The last two
  split text nodes of the facade copy and drop the pending GC pairs (Review
  Focus 5).
- Equal indexes give the same struct twice. Struct round trip is lossless:
  `pos_range_to_index_range(index_range_to_pos_range(r)) == r` for every
  valid `r`.
- `lamport` in `TimeTicketStruct` is the decimal string, as already public.

**Tests:**
- `js:test/integration/tree_test.ts` 926 `Should return correct range from
  index`, 964 `Should return correct range from path`, 1002 `... within
  doc.subscribe` (port as a remote-change case on two local documents; no
  server needed if the existing Rust ports do it that way).
- `js:test/unit/document/gc_split_leak_test.ts` 187 `registers GC pairs
  split during read-path range conversion` — Rust form: after the
  conversion inside `update`, `garbage_len` and `doc_size` equal those of a
  document rebuilt from a snapshot (the JS assertion), which holds because
  the root is never split.
- Text: round-trip cases at both ends, inside a split node, across a
  deleted range, and unknown node ID → error. JS has no dedicated Text
  test; name the JS functions in the doc comments instead.

- [ ] Write the tests RED, implement, go GREEN.
- [ ] Verification gate.
- [ ] Commit "Add Text and Tree position-range conversions".

### Task 4: Add Tree style_by_path and remove_style_by_path

**Goal:** Path-based styling as in JS, with `f70e9f16`'s bounds on the
single-path form, plus the small Tree accessors (Open Decisions).

**Files:**
- Modify: `crates/yorkie-core/src/crdt/tree.rs` (`path_to_pos_range` gets
  the past-the-end check and uses the unchecked lookup for the end;
  `compare_path` helper if none exists)
- Modify: `crates/yorkie-core/src/json/tree.rs` (`style_by_path`,
  `style_by_path_range`, `remove_style_by_path`; `id`, `node_size`,
  `root_tree_node`)
- Create: `crates/yorkie-core/src/json/tree_style_path_tests.rs`
  (`#[cfg(test)]`)
- Modify: `crates/yorkie/tests/integration_tree.rs`

**Port from:**
- `js:src/document/json/tree.ts` 410-516 (`styleByPath`, both forms:
  `path length should be equal`, `path should not be empty`, `from should
  be less than or equal to to` via `comparePath`; single form through
  `pathToPosRange`), 615-688 (`removeStyleByPath`, same three checks),
  242-248 (`getID`), 288-300 (`getNodeSize`), 960-972 (`getRootTreeNode`).
- `js@f70e9f16:src/document/crdt/tree.ts` 4680-4716 (`pathToPosRange` with
  the past-the-end rule), `js@f70e9f16:src/util/index_tree.ts` ~1205-1222
  (negative elements; N/A in Rust).
- `js:src/document/crdt/tree.ts` 4720-4723 (`getNodeSize`: size of the
  node map, Rust `node_map_len`).

**Behavior:**
- The style forms record a `TreeStyleOperation` exactly as `style`/
  `remove_style` do (ticket, attribute stringification, drained pairs);
  only the position resolution differs.
- Validation order follows JS: length mismatch, then empty, then order;
  each `InvalidArgument` with JS's message.
- Single form: a path naming no node fails with `InvalidArgument("unacceptable
  path")` (Review Focus 7); a path naming a node styles exactly that node.

**Tests:**
- `js:test/integration/tree_test.ts` 2188-2440: `Can style a range by
  path`, `Can style multiple elements across a range by path`, `Can style
  single element by path (backward compat)`, `Can remove style by path`,
  `Can remove style from multiple elements across a range by path`,
  `Should throw on mismatched path lengths for range styleByPath`,
  `Should throw on empty paths for styleByPath`, `Should throw on
  mismatched path lengths for removeStyleByPath`, `Should throw on empty
  paths for removeStyleByPath`.
- `js@f70e9f16:test/unit/document/tree_style_path_bounds_test.ts`: every
  case in 62-205 except the negative ones (116, 177); 209-280 are all
  negative-element cases, not portable (Deliberate differences).
- `js:test/unit/document/surrogate_index_test.ts` 666.
- Run the existing Tree suites unchanged; `path_to_pos_range`'s new check
  must not change `merge_by_path`/`split_by_path` results.

- [ ] Write the tests RED, implement, go GREEN.
- [ ] Verification gate.
- [ ] Commit "Add Tree style_by_path and remove_style_by_path".

## Milestone 3: Events, channels, docs

### Task 5: Report the change source and add ChannelStatus::Removed

**Goal:** Change events say whether a change was local, an undo/redo, or
remote; `ChannelStatus` has JS's three states.

**Files:**
- Modify: `crates/yorkie-core/src/operation/mod.rs` (`OpSource` becomes
  `pub`, doc text from JS)
- Modify: `crates/yorkie-core/src/event.rs` (`ChangeInfo::source`),
  `document.rs` (`change_info` takes the source: `Local` in `update`,
  `UndoRedo` in undo/redo, `Remote` for remote changes, the replay source
  for replayed local changes), `lib.rs` (re-export)
- Modify: `crates/yorkie-client/src/channel.rs` (`ChannelStatus::Removed`)
- Modify: tests that build `ChangeInfo` literals (`event.rs` ~269, ~487)

**Port from:**
- `js:src/document/operation/operation.ts` 30-34 (`OpSource`).
- `js:src/document/document.ts` 414-435 (`LocalChangeEvent.source` is
  `Local | UndoRedo`, `RemoteChangeEvent.source` is `Remote`), 2950-2954
  (`replayLocalChanges` → `replaySourceOf`), 2993 onward
  (`applyChangeInternal` publishes with its `source`), 3951 (undo/redo
  publishes `UndoRedo`).
- `js:src/channel/channel.ts` 37-52 (`ChannelStatus`), 260-269
  (`applyStatus`, `isAttached`).

**Behavior:**
- `ChangeInfo.source` is never `Remote` on `LocalChange` and always
  `Remote` on `RemoteChange`.
- `Removed` is set by no code path (as in JS); `is_attached` stays
  `status == Attached`, and the attach-time check in `client.rs` ~727
  (`!= Detached`) keeps its meaning.

**Tests:**
- `yorkie-core`: an update publishes `Local`, an undo and a redo publish
  `UndoRedo`, a remote change publishes `Remote`, and a pending undo
  replayed after a snapshot publishes `UndoRedo` (JS `replaySourceOf`; find
  the JS case that asserts it under `test/unit/document/` and cite it, or
  cite the function if none).
- Extend an existing history integration test to assert the source of the
  undo's `LocalChange`.

- [ ] Write the tests RED, implement, go GREEN.
- [ ] Verification gate.
- [ ] Commit "Report the change source and add ChannelStatus::Removed".

### Task 6: Update the gap docs

**Files:**
- Modify: `docs/current-porting-gaps.md`, `docs/porting-from-js-sdk.md`

**Steps:**
- [ ] Remove the Out-of-Scope rows "Disable-GC attach option", "History
  options", "`ChannelStatus.Removed`", "Text and Tree range helpers".
- [ ] Public JSON API: list the new `JsonText`/`JsonTree` methods and the
  struct types; replace the two "does not port" bullets with only what is
  still missing (`getIndexTree`, `getTreeByIndex`/`getTreeByID`, test
  helpers, `JsonArray::push_tree`/`get_tree_mut`) and why; note read-path
  splits stay on the copy, and the negative-path cases are unrepresentable.
- [ ] Undo and Redo: `max_undo_depth` and `clear_history` ported (attach
  clears history); remove "stacks keep 50 entries" as a fixed fact; the
  refusal during an update is unrepresentable (Deliberate differences);
  the minting-suspended refusal stays with epoch re-anchor; change events
  carry `source`.
- [ ] Garbage Collection: both `disableGC` options ported; drop the
  `disableGC` gap bullet and the ignored `object_test.ts:923` note.
- [ ] Channels and Broadcast: `ChannelStatus::Removed` exists and, as in
  JS, nothing sets it.
- [ ] Concept map in `porting-from-js-sdk.md`: rows for `DocumentOptions`,
  `OpSource`, the position structs, `sync_lamport`.
- [ ] Verification gate (docs only, but run it). Commit "Update the gaps
  for the small API ports".

## Self-review checklist (before the PR)

- `cargo doc -p yorkie-core -p yorkie-client` public surface diff shows
  only the items named in this plan.
- `git grep -n MAX_UNDO_REDO_STACK_DEPTH` shows it only as the default.
- `git grep -n "sync_clocks\|sync_lamport" crates/yorkie-core/src/document.rs`
  shows the per-change path choosing by the attach flag.
- No conversion helper takes the document root mutably.
- Live-server suite green once against v0.7.26.
