# Current Porting Gaps

Last reviewed: 2026-05-15

This document records known differences between the current Rust SDK and the
JS/Go implementations. It is intentionally practical: each section explains
what is different today, why it matters, and what should be aligned later.
Detailed per-element CRDT checklists live in
`docs/crdt-parity/README.md`.

The JS SDK remains the behavioral source of truth. The Go implementation should
be used as a typed cross-check, especially for CRDT internals, operation
application, and ownership-like structure.

Skipped upstream CRDT tests are tracked in
`docs/crdt-parity/upstream-skipped-tests.md`. They are not Rust pass targets
while they remain skipped upstream. If ported into Rust, they should stay
ignored unless the upstream behavior changes or a deliberate Rust divergence is
documented.

## Reference Areas

- JS document facade and update flow:
  `packages/sdk/src/document/document.ts`
- JS JSON object and array proxies:
  `packages/sdk/src/document/json/object.ts`,
  `packages/sdk/src/document/json/array.ts`
- JS CRDT containers:
  `packages/sdk/src/document/crdt/object.ts`,
  `packages/sdk/src/document/crdt/array.ts`,
  `packages/sdk/src/document/crdt/rga_tree_list.ts`,
  `packages/sdk/src/document/crdt/rht.ts`,
  `packages/sdk/src/document/crdt/text.ts`,
  `packages/sdk/src/document/crdt/tree.ts`
- JS operations:
  `packages/sdk/src/document/operation/*_operation.ts`
- Go typed document and CRDT implementation:
  `pkg/document/json/`,
  `pkg/document/crdt/`,
  `pkg/document/operations/`

## Document Editing Model

Current Rust behavior:

- `Document::update` clones the public `JsonObject`, lets the callback mutate
  that clone, diffs the before/after JSON values, then creates operations from
  the diff.
- Object member changes can produce `SetOperation` and `RemoveOperation`.
- Arrays can now be converted into CRDT arrays, but public array mutation still
  appears as a whole object-member replacement through the diff bridge.

JS/Go behavior:

- JS uses proxy-backed objects and arrays. Mutating the proxy records CRDT
  operations through the current change context as the mutation happens.
- Go JSON wrappers also hold document/change context and CRDT references so
  array/object methods can create the corresponding operation directly.

Gap:

- Rust does not yet preserve operation intent from public array methods.
  For example, public `JsonArray::push` inside `Document::update` does not emit
  `AddOperation`; it is currently represented by replacing the containing
  object member.
- Rust cannot yet represent in-place array `move`, `set`, or `remove` from the
  public JSON API.
- Rust update flow does not yet expose the same context-backed editing model as
  JS/Go. This is the biggest semantic gap for future local editing behavior.

Expected direction:

- Replace or wrap the clone/diff bridge with context-backed JSON objects and
  arrays.
- Keep operation creation inside the public mutation methods once they are
  connected to CRDT containers.
- Preserve the existing diff bridge only as temporary scaffolding while the
  context-backed model is introduced.

## Public JSON API

Current Rust behavior:

- `JsonObject` supports `set`, `get`, `get_mut`, `remove`,
  `get_object_mut`, and `get_array_mut`.
- `JsonArray` supports `push`, `len`, `is_empty`, and internal iteration.
- `JsonValue` is a simple enum wrapper around primitive, object, and array
  values.

JS/Go behavior:

- JS arrays expose many mutation methods that map to CRDT operations, including
  insert/add, set, delete/remove, move, and nested container access.
- JS/Go JSON values are tied to CRDT element identity and change context during
  document updates.

Gap:

- Rust `JsonArray` is still a plain value container, not a CRDT-aware editing
  facade.
- Rust lacks public APIs for array index access, index removal, index set,
  insert-after, move-after, and stable CRDT identity lookup.
- Rust nested array/object values do not carry a live connection to their CRDT
  element.

Expected direction:

- Add a context-aware public JSON layer that can issue `AddOperation`,
  `ArraySetOperation`, `MoveOperation`, and `RemoveOperation`.
- Keep `JsonValue` as the public value type only if it can preserve the same
  semantics as JS values; otherwise introduce a clearer split between input
  values and live document values.

## CRDT Root and Element Identity

Current Rust behavior:

- `CrdtRoot` stores `CrdtElementPair` values as deep copies in
  `element_pair_by_created_at`.
- Mutations are applied to the owned root object tree, then root index entries
  are refreshed manually, including the mutated container's descendant parent
  snapshots.
- Parent pointers in the index are also stored as copied elements.

JS/Go behavior:

- Root indexes point to actual CRDT elements or typed object references.
- Parent/child relationships are reference-like and naturally stay connected to
  the live tree.

Gap:

- Rust's copied index can become stale if a future mutation path forgets to
  refresh the affected element, descendants, or ancestors.
- The current approach is acceptable for small slices, but it puts more
  responsibility on every root mutation method.
- The ownership model is not yet settled. Future code may need an arena,
  stable IDs with lookup back into the tree, or another explicit ownership
  strategy.

Expected direction:

- Keep all CRDT mutations routed through `CrdtRoot` until the ownership model is
  deliberately redesigned.
- Add tests that mutate nested object/array descendants and immediately verify
  `create_path`, `find_by_created_at`, and visible JSON output.

## CRDT Array and RGA List

Current Rust behavior:

- `CrdtArray` exists and stores elements in `RgaTreeList`.
- `RgaTreeList` preserves the important concepts of position nodes, element
  identity, moved position timestamps, removed elements, and dead move
  positions.
- The backing storage is still a `Vec`, but `RgaTreeList` now keeps
  JS/Go-shaped position and element maps plus a weighted splay tree for visible
  index and path lookup.
- A weighted `SplayTree` utility now exists with JS/Go-shaped insert, splay,
  text cursor lookup, array index lookup, delete, range reweighting, and weight
  verification tests.
- Dead position nodes from array moves are registered in the root GC pair map
  and included in garbage length/stat counters.
- Root garbage collection can now physically purge dead RGA position nodes once
  the supplied version vector covers their removal time.
- RGA-level tests cover same-element and different-element move convergence,
  chained move permutations, independent move destinations, insert/set/remove
  versus move convergence, the array concurrency matrix of
  insert/move/set/remove target combinations, late losing moves that still
  create a position, and inserts after those dead positions.
- Operation-level array matrix tests apply add/move/set/remove operation pairs
  through `CrdtRoot` in both orders and verify JSON, paths, root stats, and GC
  counts converge.

JS/Go behavior:

- `RGATreeList` uses linked nodes plus a splay tree for index lookup.
- It maintains maps for position-node IDs and element IDs.
- Dead position nodes are GC children.

Gap:

- `RgaTreeList` rebuilds its maps and splay index after structural mutations
  instead of using JS/Go-style linked nodes with stable per-node index handles.
  Read lookup now follows the same map/splay route, but write-side performance
  is still conservative.
- Public API array tests still need to be connected once `JsonArray` becomes a
  context-backed editing facade.
- Protocol snapshot conversion is still missing, so `addDeadPosition` and
  `addMovedElement` parity is covered internally but not through wire
  snapshots yet.

Expected direction:

- Keep adding focused JS/Go array replay and snapshot tests around duplicate
  position IDs, moved positions, dead positions, and GC.
- Replace rebuild-on-mutation indexing with stable node handles to align the
  write-side implementation more closely with JS/Go.

## Array Operations

Current Rust behavior:

- Internal `AddOperation`, `MoveOperation`, and `ArraySetOperation` exist.
- `RemoveOperation` can remove either an object member or an array element.
- Tests cover basic add, move, array set, and array remove behavior.
- `MoveOperation` registers dead position nodes as root GC pairs.
- `MoveOperation` tests cover the case where a later winning move is applied
  before an earlier losing move, and a following add references the losing
  move's position.
- Cross-operation tests cover add/move/array-set/remove matrix convergence at
  the root operation layer.

JS/Go behavior:

- These operations are created by public array mutation APIs.
- Move operations register dead position nodes for GC.
- Operation execution receives the broader context needed for version-vector
  visibility and sync behavior.

Gap:

- Rust array operations are not yet created by public `JsonArray` methods.
- `Change` passes a version vector to operation execution, but array operations
  do not yet use version-vector visibility rules.
- Rust operation structs do not yet convert to/from protobuf or wire-level
  operation payloads.
- Rust `OpInfo` currently uses a separate `ArrayRemove` enum variant for
  clarity, while JS represents array removal as a `remove` op info carrying an
  index. This internal Rust shape may need to converge before exposing events.

Expected direction:

- Connect public array editing methods to these operations.
- Add version-vector parameters to operation execution when sync/replay
  behavior is ported.
- Align event/op-info shape before exposing watch or local event APIs.

## Counter and Increase Operation

Current Rust behavior:

- `CrdtCounter` exists for regular integer and long counters.
- Integer dedup counters are backed by an internal HLL with precision 14,
  xxhash64 seed 0, max-merge, 16KB register serialization, and restore support.
- Counter values normalize constructor input and numeric operands to fixed-width
  signed integer behavior with wrapping.
- `CrdtElement` can hold counters, and `CrdtRoot` can find and refresh counter
  elements by creation time.
- `IncreaseOperation` applies primitive numeric operands through `CrdtRoot`,
  emits internal increase op info, creates a reverse increase operation for
  undo/redo wiring, and applies actor-based dedup increases without reverse
  operations.

JS/Go behavior:

- JS exposes public `Counter` and `DedupCounter` facades that create counter
  CRDT elements and increase operations directly inside `Document.update`.
- JS and Go support regular int/long counters plus dedup counters backed by
  HLL registers. JS accepts numeric `1` for dedup increments, while Go's typed
  helper rejects float operands for dedup increments.
- Increase operations are converted to/from protocol payloads and are exercised
  through sync, history, and event tests.

Gap:

- Rust does not yet expose a public counter facade, so application code cannot
  create counters through `Document::update`.
- Counter and increase operation wire conversion is missing.
- Change-level concurrent counter tests are still missing because the public
  editing path and history stack are not complete.

Expected direction:

- Add a public counter value/facade once the context-backed editing model is in
  place.
- Port JS counter integration tests incrementally, using Go for typed CRDT
  edge cases such as bytes, data size, and dedup behavior.
- Add protocol conversion for dedup HLL register bytes and increase actors.

## Object and ElementRHT

Current Rust behavior:

- `CrdtObject` stores members in `ElementRht`.
- `ElementRht` handles basic LWW set behavior, tombstones overwritten values,
  and keeps lookup by visible key and creation time.
- Object descendants can now include arrays, text, and counters.

JS/Go behavior:

- Object mutation happens through context-aware document wrappers.
- Object containers participate in a broader GC and presence/sync lifecycle.

Gap:

- Rust object behavior has focused unit coverage but does not yet have broad
  JS test parity.
- Rust object mutation from public API still relies on clone/diff rather than a
  live context-backed object facade.
- Object GC behavior is limited to removed element tracking; broader GC pair
  behavior is not implemented.

Expected direction:

- Keep adding tests from JS object and container behavior.
- Move public object mutation toward the same context-backed model as arrays.

## Attributes and RHT

Current Rust behavior:

- `Rht` and `RhtNode` now model string attributes with LWW updates,
  tombstones, removed-node GC candidates, visible-size accounting,
  deep-copy behavior, node identity, and JSON/object conversion.
- `TextValue` and `TreeNode` use `Rht` for attributes, and `CrdtRoot` can
  rebuild GC pair indexes for removed text/tree attribute nodes when a root is
  created from an existing CRDT tree.
- Root garbage collection can physically purge removed text/tree attribute
  nodes once the synced version vector covers their removal time.
- `StyleOperation` registers removed text attribute nodes as root GC pairs when
  applying or removing text styles through the operation layer.
- Unit tests cover the JS RHT test flow for set/get/has, remove, remove of
  missing keys, set after remove, repeated remove, deep copy, purge, and escaped
  JSON output.

JS/Go behavior:

- JS `RHT.set` returns both the previous removed node and the new node so text
  and tree style operations can register GC pairs and construct reverse
  operation info.
- JS `RHT.remove` creates tombstones even when the key did not exist, which is
  required for concurrent style removal.
- Go exposes the same typed CRDT concept and sorts keys during marshaling.

Gap:

- Public Text facade methods are not connected yet, so users cannot create
  text style operations from the public document API.
- Rust `Rht::to_json` uses deterministic key ordering through `BTreeMap`.
  This matches Go marshaling and text's sorted attribute output, but JS
  `RHT.toJSON` itself follows `Map` insertion order.
- Wire conversion for tree/text attributes is not implemented.

Expected direction:

- Connect public Text style APIs to `StyleOperation`.
- Keep Text output aligned with JS `CRDTTextValue.toJSON`, including parsing
  attribute values from JSON strings before serializing visible attributes.

## Text and RGATreeSplit

Current Rust behavior:

- `TextValue` stores text content plus `Rht` attributes and reports length in
  UTF-16 code units.
- `RgaTreeSplit` models text blocks, split positions, tombstones, insertion
  predecessor links, basic edit/delete behavior, styling, style removal, JSON
  output, text output, removed text-node GC candidates, and attribute GC
  candidates.
- `CrdtText` wraps `RgaTreeSplit<TextValue>` with CRDT element metadata and
  offers internal index-based and position-range-based edit/style/remove-style
  helpers.
- `CrdtText` is now a `CrdtElement` variant, participates in root path lookup,
  can be found through root text lookup helpers, and converts to public
  `JsonValue::Array` for internal document-root materialization.
- `CrdtRoot` can rebuild GC pair indexes for removed text nodes and removed
  text attribute nodes when it is created from an existing CRDT tree.
- Root garbage collection can physically purge removed text nodes and removed
  text attribute nodes.
- Internal `EditOperation` and `StyleOperation` now execute against `CrdtText`,
  receive the enclosing change's version vector, register removed text or
  attribute nodes as GC pairs, update root document size through `acc`, emit
  internal operation info, and create reverse operations for undo/redo wiring.
- `RgaTreeSplit` now keeps JS/Go-shaped index structures: a weighted splay tree
  for text index lookup and a node-ID map for floor lookup.
- Unit tests cover the Go text CRDT smoke tests and matching JS scenarios:
  split-position lookup, Korean composition replacement, deletion with removed
  boundary nodes, deletion of last nodes, concurrent insert/delete with original
  positions, Peritext-style concurrent insertion, format plus insertion,
  overlapping format, conflicting LWW format, style removal, and UTF-16 code
  unit splitting.

JS/Go behavior:

- JS and Go use `RGATreeSplit` with splay-tree index lookup and an LLRB tree by
  node ID.
- JS public `Text` methods stringify attribute values before passing them into
  internal `CRDTText`, and `CRDTTextValue.toJSON` parses those stringified
  values before serializing visible attributes.
- Text edit/style/remove-style operations register text-node and attribute-node
  GC pairs in the change context.

Gap:

- `RgaTreeSplit` rebuilds its splay and node-ID indexes after structural
  mutations instead of using JS/Go-style linked nodes with stable per-node index
  handles. Read lookup now follows the same splay/ID-map route, but write-side
  index maintenance is still conservative.
- Rust `CrdtText` is not yet exposed through a public Text facade or wire
  conversion.
- `StyleOperation` emits per-block style operation info from the text helper,
  but `EditOperation` still emits a single requested range instead of the full
  value-change list produced by JS `RGATreeSplit.edit`.
- Text operation info is still internal and shaped for Rust tests; it has not
  yet been aligned with the external watch/event payload shape.
- Version-vector-aware edit/style conflict behavior is modeled at the internal
  helper level and is passed through `Change`, but it still needs broader
  multi-change replay coverage at the operation layer.
- Rust strings cannot represent invalid standalone UTF-16 surrogate halves, so
  splitting inside a surrogate pair currently uses lossy UTF-16 decoding. Exact
  JS string parity for that edge case needs a deliberate representation choice.

Expected direction:

- Connect public Text methods through `ChangeContext`.
- Align edit operation info with the JS value-change list before exposing text
  watch/event APIs.
- Continue porting JS history and multi-client text scenarios at the operation
  layer.
- Replace rebuild-on-mutation indexing with stable node handles to align the
  write-side implementation more closely with JS/Go.

## Tree

Current Rust behavior:

- `TreeNodeId` models the same creation-ticket plus UTF-16 offset identity used
  by JS `CRDTTreeNodeID` and Go `TreeNodeID`.
- `CrdtTree` stores a root `TreeNode`, rebuilds a node lookup map by ID, and
  resolves split-node floor lookups only when the requested ID has the same
  creation ticket.
- `TreeNode` supports element nodes, text nodes, optional `Rht` attributes,
  tombstones, split-link metadata, merge metadata, JSON/XML output, UTF-16 text
  sizing, active attribute data size, and internal GC pair discovery.
- `CrdtTree` can convert between linear tree indexes, paths, and CRDT tree
  positions using the JS/Go element padding and text-child path rules.
- `CrdtTree` can apply simple style/remove-style ranges to visible element
  tokens, split text nodes at style boundaries, ignore text-only ranges after
  applying the boundary splits, collect tree style operation info, and report
  reverse-operation inputs.
- `CrdtTree` can apply split-free element insert/delete edits and text-node
  split insert/delete edits, collect tree edit operation info, register removed
  node GC pairs, and report reverse operation inputs.
- `CrdtElement::Tree` participates in metadata dispatch, JSON conversion,
  data-size accounting, removal, and deep copy.
- `CrdtRoot` can find tree elements by creation time, rebuild tree internal GC
  pairs from an existing root object, deep-copy them, and physically purge
  removed tree nodes or removed tree attributes once a version vector covers
  their removal time.
- `TreeStyleOperation` executes tree style/remove-style operations, applies
  text-boundary splits, registers removed attribute GC pairs, accumulates root
  size diff, and creates reverse tree style operations.
- `TreeEditOperation` executes split-free element insert/delete operations and
  text-node split insert/delete operations, registers removed tree-node GC
  pairs, accumulates root size diff for inserted nodes and text splits, and
  creates reverse tree edit operations.

JS/Go behavior:

- JS and Go back Tree with an index tree for path/index conversion and an LLRB
  map for node IDs.
- Tree edit/style operations split text and element nodes, maintain
  `insPrevID`/`insNextID`, redirect merged parents through `mergedFrom`,
  `mergedAt`, and `mergedInto`, register tree-node and attribute GC pairs, and
  produce reverse operation info.
- Public Tree wrappers create context-backed tree edits instead of direct node
  mutation.

Gap:

- Tree path/index conversion exists for the current in-memory tree, but it is
  not yet maintained by tree edit operations and still needs broader
  removed-node and mixed child coverage.
- Rust Tree style operation is only partial. It splits text nodes at style
  boundaries, but it does not yet advance unknown split siblings or propagate
  style/remove-style across unknown split siblings the way JS/Go do for
  concurrent Tree split cases.
- Rust Tree edit operation is only partial. It can split text nodes for simple
  insert/delete ranges, but it does not split element nodes, merge element
  boundaries, fully maintain `insPrevID`/`insNextID` across existing neighbors,
  propagate merge metadata, or handle unknown split siblings like JS/Go.
- Like `CrdtText`, Tree text-node splitting uses valid Rust strings. Splitting
  inside an invalid standalone UTF-16 surrogate edge would need the same
  deliberate representation choice as Text.
- Split and merge metadata is stored, and new text split nodes receive the
  basic split identity/link metadata, but broader split/merge metadata
  maintenance is still incomplete.
- Operation-time GC registration exists for split-free tree-node deletion and
  text-node split deletion, but element split/merge deletion paths still need
  JS/Go parity.
- Public Tree facade and wire conversion are missing.
- Tree attribute JSON/XML output follows the same scalar parsing helper as
  Text, but object/array attribute values still need broader JS parity tests
  before public style APIs expose them.

Expected direction:

- Extend path/index conversion tests around removed nodes and mixed
  element/text children so edit/style operations can reuse the same position
  semantics.
- Add Tree edit operation tests from JS/Go around element split, merge, and
  unknown split sibling cases before exposing public Tree methods.
- Extend Tree style parity from text-boundary split support toward version
  vectors, unknown split siblings, and split sibling propagation.

## Change, ChangeContext, and ChangePack

Current Rust behavior:

- `ChangeContext` issues time tickets and stores operations.
- `Change` executes operations and stacks reverse operations.
- `ChangePack` carries document key, checkpoint, changes, version vector,
  removal flag, and optional snapshot bytes.
- `Document::apply_change_pack` applies changes and advances checkpoints.

JS/Go behavior:

- Change context is connected to root proxies, presence changes, operation
  creation, and actor assignment.
- Change packs handle snapshot application, document removal, sync status, and
  wire conversion.

Gap:

- Rust change context does not own or mediate live JSON/CRDT wrappers.
- Presence changes are not implemented.
- Snapshot application is explicitly unsupported.
- `ChangePack::is_removed` is stored but document removal behavior is not
  applied.
- Change and operation serialization is not implemented.
- Sync/status transitions are not implemented.

Expected direction:

- Add live change context before expanding public document mutation APIs.
- Implement serialization and snapshot handling before real client sync.
- Add document removal behavior when `ChangePack::is_removed` is applied.

## Undo and Redo

Current Rust behavior:

- Some operations create reverse operations.
- Undo/redo source handling exists as an enum and has narrow operation tests.

JS/Go behavior:

- Undo/redo is integrated with history, operation grouping, and array/object
  operation semantics.

Gap:

- There is no full document history manager.
- Reverse operations are not yet exercised through a user-facing undo/redo API.
- Array reverse operation behavior is only covered in small internal slices.

Expected direction:

- Port history behavior after public mutation methods produce correct
  operation types.
- Use JS history tests as the main source for expected grouping and reverse-op
  behavior.

## Serialization and Protocol Conversion

Current Rust behavior:

- CRDT, change, and operation types are in-memory only.
- Protocol crates are scaffolded but not connected to operation/change
  conversion.

JS/Go behavior:

- Operations, changes, checkpoints, snapshots, and packs convert to protobuf
  payloads for sync.

Gap:

- No operation-to-protobuf conversion.
- No protobuf-to-operation conversion.
- No snapshot encoding/decoding.
- No wire compatibility tests.

Expected direction:

- Port operation converters after the in-memory operation set is stable.
- Use JS converter files and Yorkie proto definitions as the main reference,
  with Go as a server/client cross-check.

## Client, Sync, and Watch

Current Rust behavior:

- Client crates and facade are scaffolded.
- Document local change packs can be created and applied in-memory.

JS/Go behavior:

- Clients support activate/deactivate, attach/detach, sync, watch, presence,
  stream handling, retry/error behavior, and document status transitions.

Gap:

- No real client lifecycle.
- No RPC transport.
- No watch stream.
- No presence.
- No document status model.
- No sync mode handling.

Expected direction:

- Keep CRDT and document behavior stable before implementing network sync.
- Start client work only after protocol conversion and change pack behavior are
  closer to JS/Go.

## Error Behavior

Current Rust behavior:

- `YorkieError` is small and strongly typed for the current implementation.

JS/Go behavior:

- JS uses Yorkie error codes and messages.
- Go has typed errors that often map to server/client operation failure modes.

Gap:

- Rust error variants and messages are not yet aligned with JS error codes.
- Some temporary errors use generic `MissingCrdtElement` or
  `UnexpectedCrdtElement` where JS/Go may report more specific cases.

Expected direction:

- Align public-facing errors with JS error codes before stabilizing the SDK
  surface.
- Keep internal errors specific enough to debug CRDT state issues.

## Test Coverage

Current Rust behavior:

- Unit tests cover current CRDT primitives, object/RHT behavior, array/RGA
  basics, operations, change packs, and document JSON round trips.

JS/Go behavior:

- JS and Go have broader test suites covering local editing, sync, concurrency,
  GC, snapshots, and client behavior.

Gap:

- Rust tests are hand-picked slices, not a systematic JS test port.
- No automated parity harness exists.
- Array tests do not yet cover the full JS/Go RGA matrix.
- Document tests do not yet verify public array operation intent because the
  public JSON layer cannot emit those operations yet.

Expected direction:

- Port tests feature by feature from JS, with Go cross-checks for CRDT
  internals.
- Add regression tests whenever a Rust implementation detail intentionally
  differs from JS/Go.

## Safe Assumptions for Future Work

- Treat current array CRDT behavior as a semantic scaffold, not a finished
  performance implementation.
- Do not expose current operation event shapes as stable public API.
- Route CRDT mutations through `CrdtRoot` to keep the copied root index fresh.
- Prefer adding missing public JSON behavior before expanding network/client
  behavior.
- Keep JS/Go references in docs and planning notes, not in public code comments.
