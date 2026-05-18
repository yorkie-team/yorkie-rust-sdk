# Current Porting Gaps

Last reviewed: 2026-05-19

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

- `Document::update` clones the public `JsonObject`, attaches an edit recorder
  to that clone, lets the callback mutate it, then executes the recorded
  operations against `CrdtRoot` after the callback succeeds.
- `Document` tracks `Detached`, `Attached`, and `Removed` status, exposes the
  current actor ID, applies actor assignment to pending local changes, and
  rejects edits after removal.
- Public `JsonObject::set` and `JsonObject::remove` record `SetOperation` and
  `RemoveOperation` at the mutation site when called inside `Document::update`.
- Public `JsonArray::push`, `insert`, `insert_after`, `insert_before`,
  `insert_after_index`, `insert_integer_after`, `set`, `set_value`, `remove`,
  `delete`, `delete_by_id`, `move_after`, `move_before`, `move_front`,
  `move_last`, `move_after_by_index`, and `splice` record array operations at
  the mutation site when called inside `Document::update`.
- The public JSON view now carries hidden CRDT identity for object members and
  array elements so nested object/array mutations can be recorded after a
  parent object or array is created in the same update callback.
- The previous clone/diff bridge remains as a fallback only when no public
  mutation method recorded an operation.

JS/Go behavior:

- JS uses proxy-backed objects and arrays. Mutating the proxy records CRDT
  operations through the current change context as the mutation happens.
- Go JSON wrappers also hold document/change context and CRDT references so
  array/object methods can create the corresponding operation directly.

Gap:

- Rust records object and array method intent, but still mutates a cloned
  public JSON view and replays the recorded operations after the callback
  rather than mutating live CRDT containers directly during the callback.
- Direct `JsonValue` mutation through low-level mutable access can still bypass
  the recorder. The fallback diff only covers updates that recorded no
  operations, so mixed direct-value edits and method-based edits remain a gap.
- Rust has ID-based array insert/delete/move APIs and splice-like array edits,
  but these still operate on the cloned public JSON view and hidden identity
  metadata rather than live CRDT containers.
- Counter public editing methods are connected to the edit recorder. Text and
  Tree public editing methods are not connected yet.

Expected direction:

- Continue moving public mutation methods onto the edit recorder.
- Either remove the clone/diff fallback or narrow it to explicitly unsupported
  compatibility cases once direct mutable JSON escape hatches are redesigned.
- Connect Text and Tree public facades through the same update-time operation
  recorder.

## Public JSON API

Current Rust behavior:

- `JsonObject` supports `set`, `get`, `get_mut`, `remove`,
  `get_object_mut`, `get_array_mut`, `get_counter_mut`, `set_counter`,
  `set_long_counter`, and `set_dedup_counter`.
- `JsonArray` supports `push`, index `get`/`get_mut`, `set`/`set_value`,
  `insert`, `insert_after_index`, `insert_integer_after`, `remove`/`delete`,
  element-ID lookup, `get_by_id`, `get_mut_by_id`, `get_element_by_index`,
  `get_element_by_id`, `get_last`, `insert_after`, `insert_before`,
  `delete_by_id`, `move_after`, `move_before`, `move_front`, `move_last`,
  `move_after_by_index`, `splice`, nested `get_object_mut`/`get_array_mut`,
  `get_counter_mut`, `push_counter`, `push_long_counter`,
  `push_dedup_counter`, value search, ID search, `len`, `is_empty`, `iter`,
  and `as_slice`.
  Mutation methods can return errors because they may create CRDT operations
  during `Document::update`.
- `JsonValue` is a simple enum wrapper around primitive, counter, object, and
  array values.
- During `Document::update`, public object, array, and counter values are
  temporarily attached to a recorder and hidden CRDT identity metadata.

JS/Go behavior:

- JS arrays expose many mutation methods that map to CRDT operations, including
  insert/add, set, delete/remove, move, and nested container access.
- JS/Go counters expose regular increase paths and dedup actor-add paths that
  create increase operations during document updates.
- JS/Go JSON values are tied to CRDT element identity and change context during
  document updates.

Gap:

- Rust public object, array, and counter values are only partially CRDT-aware.
  The existing method calls record operations, but the values are still not
  stable live wrappers around CRDT elements.
- Rust public arrays now expose element-ID access and lightweight
  `JsonArrayElement` values for read-only ID/value lookup. They still do not
  expose mutable JS-style `WrappedElement` proxies tied directly to live CRDT
  containers.
- Rust value-based array search uses `JsonValue` equality. For non-primitive
  containers, JS compares wrapped element identity; Rust callers should use the
  explicit ID search methods when identity matters.
- Text and Tree do not have public context-backed facades yet.

Expected direction:

- Continue tightening the context-aware public JSON layer around stable
  identity access and direct mutation escape hatches.
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
- Public `JsonArray` tests now cover element-ID lookup, read-only
  `JsonArrayElement` lookup, insert-after/by-before, insert-after-index,
  delete-by-ID, move-after, move-before, move-front, move-last,
  move-after-by-index, splice remove/insert sequences, and open-ended negative
  splice deletion through `Document::update`.

JS/Go behavior:

- `RGATreeList` uses linked nodes plus a splay tree for index lookup.
- It maintains maps for position-node IDs and element IDs.
- Dead position nodes are GC children.

Gap:

- `RgaTreeList` rebuilds its maps and splay index after structural mutations
  instead of using JS/Go-style linked nodes with stable per-node index handles.
  Read lookup now follows the same map/splay route, but write-side performance
  is still conservative.
- Public ID-based and splice-like array tests are connected at the recorder
  facade layer, but the implementation still replays operations after the
  callback rather than mutating live CRDT containers during the callback.
- Protocol `JSONElement.Array` conversion now preserves live RGA nodes, moved
  position nodes, and dead position nodes. Decoded snapshot roots can replace a
  document root through `Document::apply_change_pack`.

Expected direction:

- Keep adding focused JS/Go array replay, converter, and snapshot tests around
  duplicate position IDs, moved positions, dead positions, and GC.
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
- Document tests verify that public arrays can produce add, remove,
  array-set, delete-plus-push, insert, ID-based insert/delete/move,
  index-based insert, splice remove/insert sequences, same-update nested
  object edits after splice insertion, and nested object operations through
  update-time method recording.

JS/Go behavior:

- These operations are created by public array mutation APIs.
- Move operations register dead position nodes for GC.
- Operation execution receives the broader context needed for version-vector
  visibility and sync behavior.

Gap:

- Rust public `JsonArray` methods now create add/move/array-set/remove
  operations directly through the update-time recorder. Mixed direct
  `JsonValue` mutation can still bypass the recorder.
- `Change` passes a version vector to operation execution, but array operations
  do not yet use version-vector visibility rules.
- Rust now has `yorkie_core::wire` projections for set/add/move/remove,
  edit/style/increase/tree-edit/tree-style/array-set operations, including
  protobuf-shaped object/array/tree payload bytes and from-wire operation
  construction. Broader replay tests for text/tree operation payloads are still
  needed before treating this as full sync parity.
- Rust `OpInfo` currently uses a separate `ArrayRemove` enum variant for
  clarity, while JS represents array removal as a `remove` op info carrying an
  index. This internal Rust shape may need to converge before exposing events.

Expected direction:

- Keep extending context-backed public array editing while reducing the
  remaining clone/diff fallback surface.
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
- Public `JsonCounter` exists for integer, long, and integer-dedup counters.
  `JsonObject::set_counter`, `set_long_counter`, `set_dedup_counter`,
  `get_counter_mut`, and `JsonArray::push_counter`, `push_long_counter`,
  `push_dedup_counter`, `get_counter_mut` provide context-backed document
  editing paths.
- During `Document::update`, `JsonCounter::increase` records
  `IncreaseOperation` for regular counters and `JsonCounter::add` records
  actor-based increase operations for dedup counters. Newly created counters
  can be increased in the same callback.

JS/Go behavior:

- JS exposes public `Counter` and `DedupCounter` facades that create counter
  CRDT elements and increase operations directly inside `Document.update`.
- JS and Go support regular int/long counters plus dedup counters backed by
  HLL registers. JS accepts numeric `1` for dedup increments, while Go's typed
  helper rejects float operands for dedup increments.
- Increase operations are converted to/from protocol payloads and are exercised
  through sync, history, and event tests.

Gap:

- Rust uses explicit constructors and object/array helper methods instead of
  JavaScript's `new Counter(...)` syntax or Go's dynamic `any` value
  inference. The semantics are aligned, but the public shape is Rust-specific.
- Counter creation and increase operations now convert to and from generated
  protobuf payloads through `yorkie_core::wire` and
  `yorkie_protocol::converter`. Binary `ChangePack` round-trip tests cover
  counter replay, and full `JSONElement.Counter` conversion carries HLL
  register payloads for dedup counters.
- Change-level concurrent counter tests are still missing because the public
  editing path does not yet include client sync/history.

Expected direction:

- Port JS counter integration tests incrementally, using Go for typed CRDT
  edge cases such as bytes, data size, and dedup behavior.
- Add cross-language protocol fixtures for dedup HLL register bytes and
  increase actors.

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
- Wire conversion now maps text and tree attribute `Rht` nodes to protobuf
  `NodeAttr` payloads. Text attributes follow the JS/Go text-node converter
  shape where removed attribute flags are not restored through `TextNode`
  attributes; tree attributes preserve `is_removed` through full `RHT`
  conversion.

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
- Rust `CrdtText` is not yet exposed through a public Text facade. Wire
  conversion exists for full `JSONElement.Text` snapshots and text
  edit/style operation bodies, but replay coverage is still narrower than
  JS/Go integration tests.
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
- `CrdtTree` now resolves edit positions through parent/left sibling paths,
  advances unknown split siblings via `insNextID`, narrows collection ranges
  across split siblings, and falls back to floor-style text lookup when a text
  node has moved under a concurrent split sibling.
- `CrdtTree` can apply simple style/remove-style ranges to visible element
  tokens, split text nodes at style boundaries, ignore text-only ranges after
  applying the boundary splits, collect tree style operation info, and report
  reverse-operation inputs. It also has the first version-vector-aware split
  sibling propagation used by JS/Go for style/remove-style collection.
- `CrdtTree` can apply split-free element insert/delete edits and text-node
  split insert/delete edits, collect tree edit operation info, register removed
  node GC pairs, and report reverse operation inputs.
- `CrdtTree` can apply multi-level element splits by cloning the split element
  type and attributes, moving the right-side children into new siblings, and
  issuing split node IDs with the same delimiter progression used by JS/Go.
- `CrdtTree` can merge visible element boundaries by tombstoning merge source
  elements, moving live children into the left-side parent, and recording basic
  `mergedFrom`, `mergedAt`, and `mergedInto` metadata.
- `CrdtTree` covers representative concurrent split cases ported from JS/Go:
  same-position splits, different-position splits on the same node, and
  different-level splits.
- `CrdtTree` also covers representative concurrent insert/delete cases around
  split positions, including inserts into the original node, split boundary, and
  split node.
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
  text-node split insert/delete operations plus multi-level element split and
  visible-boundary merge operations, registers removed tree-node GC pairs,
  accumulates root size diff for inserted nodes and splits, and creates reverse
  tree edit operations for insert/delete, pure split, and visible-boundary merge
  cases.

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

- Tree path/index conversion exists for the current in-memory tree, and edit
  and style collection now use JS/Go-like parent/left position resolution and
  token traversal. Rust still recomputes over the current tree instead of
  maintaining the same stable `IndexTree` structure, so removed-node and mixed
  child coverage needs to keep expanding.
- Rust Tree style operation is still partial. It now propagates style/remove
  style to direct unknown split siblings in the JS/Go shape, but the broader
  concurrent style matrices and range-narrowing edge cases are not yet ported.
- Rust Tree edit operation is still partial. It now supports text split ranges,
  multi-level element split, visible element-boundary merge, representative
  concurrent split/insert/delete cases, range narrowing, and basic merge metadata, but
  redo tagging after split undo, full `insPrevID`/`insNextID` maintenance across
  existing neighbors, and the broader concurrent edit matrix still need JS/Go
  parity work.
- Like `CrdtText`, Tree text-node splitting uses valid Rust strings. Splitting
  inside an invalid standalone UTF-16 surrogate edge would need the same
  deliberate representation choice as Text.
- Split and merge metadata is stored, new text/element split nodes receive the
  basic split identity/link metadata, and merge source children receive basic
  merge metadata. Neighbor-link maintenance and concurrent split/merge metadata
  still need broader parity coverage.
- Operation-time GC registration exists for split-free tree-node deletion,
  text-node split deletion, and visible-boundary merge source tombstones, but
  concurrent element split/merge deletion paths still need JS/Go parity.
- Public Tree facade is missing. Wire conversion exists for full
  `JSONElement.Tree` payloads and tree edit/style operation bodies, but
  broader JS/Go protocol replay fixtures are still needed.
- Tree attribute JSON/XML output follows the same scalar parsing helper as
  Text, but object/array attribute values still need broader JS parity tests
  before public style APIs expose them.

Expected direction:

- Extend path/index conversion tests around removed nodes and mixed
  element/text children so edit/style operations can reuse the same position
  semantics.
- Add Tree edit operation tests from JS/Go around redo after split undo,
  removed-node ranges, mixed-level merge cases, and the broader generated
  concurrent edit matrix before exposing public Tree methods.
- Extend Tree style parity from direct split-sibling propagation toward the full
  version-vector and concurrent style matrices in JS/Go.

## Change, ChangeContext, and ChangePack

Current Rust behavior:

- `ChangeContext` issues time tickets and stores operations.
- `Document::update` uses an update-time recorder backed by `ChangeContext` for
  current public object and array mutation methods.
- `Change` executes operations and stacks reverse operations.
- `ChangePack` carries document key, checkpoint, changes, version vector,
  removal flag, and optional snapshot bytes.
- `Document::apply_change_pack` applies changes or decoded snapshot roots,
  removes acked local changes, reapplies remaining local changes after
  snapshots, advances checkpoints, and applies removed status when the pack is
  marked as removed.

JS/Go behavior:

- Change context is connected to root proxies, presence changes, operation
  creation, and actor assignment.
- Change packs handle snapshot application, document removal, sync status, and
  wire conversion.

Gap:

- Rust change context now mediates the first public object/array mutation
  methods, but it does not yet own full live JSON/CRDT wrappers.
- Presence changes are not implemented.
- Change packs can be projected to generated protobuf payloads and
  reconstructed back into core `ChangePack` values for the current operation
  set. Binary round-trip tests cover nested object/array/counter replay, and
  protocol snapshot tests cover root replacement through
  `Document::apply_change_pack`.
- Raw snapshot bytes that have not been decoded by the protocol converter are
  still rejected by core to keep protobuf parsing out of `yorkie-core`.
- Full sync status transitions are not implemented.

Expected direction:

- Add live change context before expanding public document mutation APIs.
- Extend snapshot handling with presence, GC, and cross-language binary
  fixtures before real client sync.
- Extend status handling with presence, events, and client-driven lifecycle
  transitions.

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

- `yorkie_core::wire` exposes protocol-shaped projections of internal changes,
  operations, simple elements, full JSON elements, RGA/RHT nodes, text nodes,
  tree nodes, and positions without making the internal CRDT and operation
  structs part of the public SDK facade.
- `yorkie_protocol::yorkie::v1` includes checked-in generated Rust sources
  under `crates/yorkie-protocol/src/generated`, produced from vendored Yorkie
  protobuf files.
- `yorkie_protocol::converter::to_change_pack` converts a core `ChangePack`
  into generated protobuf payloads for set/add/move/remove/edit/style/increase/
  tree-edit/tree-style/array-set operations.
- `yorkie_protocol::converter::from_change_pack` reconstructs core
  `ChangePack` values from generated protobuf payloads.
- `encode_change_pack` and `decode_change_pack` provide prost binary
  round-trip helpers.
- Object, array, and tree simple element values follow the JS/Go converter
  shape: the `JSONElementSimple.value` field contains an encoded full
  `JSONElement` payload.
- Protocol conversion tests cover actor bytes, version-vector base64 actor
  keys, counter set/increase operations, dedup counter increase actors,
  object/array `JSONElementSimple` full-payload bytes, protobuf-to-domain
  reconstruction, decoded snapshot application, snapshot-root `ChangePack`
  protobuf bytes, binary change-pack round trips, and mixed operation replay
  through object set/remove, array add/remove/move, text edit/style, and
  counter increase. They also cover missing-checkpoint change-pack errors, tree
  node arrays, and full
  `JSONElement.Tree` protobuf payloads, including node IDs, depths,
  attributes, removed markers, and persisted merge metadata.
- Core wire tests now port the JS root bytes scenario for nested object, array,
  text, and counter values, Go's standalone array bytes scenario, the JS/Go
  object-GC bytes scenario, and JS/Go tree bytes scenarios around plain tree
  nodes, edit/style state, and merge metadata.

JS/Go behavior:

- Operations, changes, checkpoints, snapshots, and packs convert to protobuf
  payloads for sync.

Gap:

- Presence changes are not converted yet.
- Go has a nil-pack converter error case. Rust takes `&api::ChangePack`, so
  that exact nil case is excluded by the function signature; the missing
  checkpoint case is covered.
- Snapshot bytes are carried in `ChangePack`, and the protocol converter
  decodes `Snapshot.root` into a core snapshot root for
  `Document::apply_change_pack`. Snapshot presences are currently ignored
  because presence is not implemented.
- No cross-language binary compatibility tests. This is intentionally not the
  immediate focus while JS/Go in-repo converter scenarios are still being
  ported.
- JS and Go both have direct converter tests for root/object bytes and tree
  bytes. Go also has direct array bytes, change-pack, presence, and snapshot
  converter tests. Rust currently covers object/array simple payload bytes,
  root/object bytes at core wire level, standalone array/tree bytes at core
  wire level, object-GC bytes at core wire level, tree edit/style and
  merge-state root bytes at core wire level, tree protobuf payload round trips
  at protocol level, snapshot-root `ChangePack` protobuf round trips,
  simple and mixed change-pack replay, and a generated-protobuf snapshot apply
  path. Presence remains blocked by the missing presence model.

Expected direction:

- Continue porting JS/Go in-repo converter scenarios before adding external
  binary fixtures.
- Add snapshot GC/presence behavior once sync and presence are pulled into the
  document lifecycle.
- Use JS converter files and Yorkie proto definitions as the main reference,
  with Go as a server/client cross-check.

## Client, Sync, and Watch

Current Rust behavior:

- Client crates and facade are scaffolded.
- `yorkie-client` is split into `client`, `options`, `attachment`,
  `transport`, and `error` modules, with `lib.rs` kept as a re-exporting crate
  entrypoint.
- `ClientOptions` carries the JS/Go-shaped base options that do not require a
  network runtime yet: RPC address, client key, API key, metadata, user agent,
  sync-loop duration, retry delay, reconnect delay, and channel heartbeat
  interval. Defaults match the JS/Go values where both implementations already
  agree.
- `SyncMode`, `DeactivateOptions`, `AttachOptions`, `AttachChannelOptions`,
  `DetachOptions`, and `SyncOptions` are defined as public client-surface
  types.
- `Client` stores a key, optional actor ID, deactivated status, sync/watch
  condition flags, and document attachment metadata.
- `ClientTransport` is a testable transport boundary for activate/deactivate,
  document attach/detach/remove, and client-side push-pull sync.
  `Client::activate` sends client key, metadata, and shard key through this
  boundary; `Client::deactivate` sends client ID and synchronous flag, clears
  local attachment metadata, and marks sync/watch conditions inactive.
- `Client::attach` sends client ID, local `ChangePack`, schema key, and shard
  key through the transport boundary, stores returned max-size/schema-rule
  metadata on the document, applies the returned change pack, records
  attachment metadata, and marks the watch loop condition active for realtime
  sync modes.
- `Client::detach` sends client ID, attached document ID, local `ChangePack`,
  `remove_if_not_attached`, and shard key through the transport boundary,
  applies the returned change pack, updates document status, removes attachment
  metadata, and refreshes the watch loop condition.
- `Client::remove` sends client ID, attached document ID, local `ChangePack`
  marked as removed, and shard key through the transport boundary, applies the
  returned change pack, removes attachment metadata, and refreshes the watch
  loop condition. The request/response shape follows the client-side remove
  flow in JS, with Go client behavior used as a typed cross-check.
- `Client::sync` and `Client::sync_with_options` send client ID, attached
  document ID, local `ChangePack`, push-only mode, and shard key through the
  transport boundary, apply the returned change pack, and remove attachment
  metadata when the document is removed. The push-only response guard follows
  the client-side JS behavior.
- `Client::has` and `change_sync_mode` cover local lifecycle bookkeeping and
  precondition errors for documents.
- Document local change packs can be created and applied in-memory.
- `Document` stores max-size and schema-rule metadata received during attach,
  but update-time max-size/schema validation is not implemented yet.
- `yorkie-protocol` converts schema rules between generated protobuf types and
  Rust document metadata using the same field shape as JS/Go converters.

JS/Go behavior:

- Clients support activate/deactivate, attach/detach, sync, watch, presence,
  stream handling, retry/error behavior, and document status transitions.

Gap:

- No concrete RPC transport.
- Activate/deactivate/attach/detach/remove/push-pull sync use a transport
  trait, but there is no Connect/gRPC-web implementation yet.
- No watch stream.
- No presence.
- Schema rules and max-size limits are stored after attach but are not enforced
  during `Document::update` yet.
- Deactivation clears client attachment metadata but cannot yet update attached
  document statuses because documents are not owned by the client.
- Sync modes are represented in attachment metadata but not executed by a sync
  loop.
- Auth token refresh, gRPC-web/connect transport choice, Go TLS options, and
  Go receive-size/logger options are not modeled yet.

Expected direction:

- Add a concrete protocol transport for the existing client lifecycle boundary.
- Keep Client state transitions aligned with document status and attachment
  metadata as the network layer lands.

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
- Array/RGA and operation tests now cover the JS/Go-style insert/move/set/remove
  matrix at CRDT and root-operation layers.
- Document tests verify common public array operation intent, including
  ID-based move/insert/delete APIs and splice-like remove/insert sequences.
  The facade is recorder-backed during `Document::update`, but still not a live
  CRDT container wrapper.

Expected direction:

- Port tests feature by feature from JS, with Go cross-checks for CRDT
  internals.
- Add regression tests whenever a Rust implementation detail intentionally
  differs from JS/Go.

## Safe Assumptions for Future Work

- Treat current array CRDT behavior as semantically covered for the ported
  internal matrix, but not as a finished performance implementation.
- Do not expose current operation event shapes as stable public API.
- Route CRDT mutations through `CrdtRoot` to keep the copied root index fresh.
- Prefer adding missing public JSON behavior before expanding network/client
  behavior.
- Keep JS/Go references in docs and planning notes, not in public code comments.
