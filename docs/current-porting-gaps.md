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
  operations against a copy of `CrdtRoot` after the callback succeeds, and
  replaces the root with the copy once every operation has run.
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
  `get_object_mut`, `get_array_mut`, `get_counter_mut`, `get_text_mut`,
  `get_tree_mut`, `set_counter`, `set_long_counter`, `set_dedup_counter`,
  `set_text`, and `set_tree`.
- `JsonArray` supports `push`, index `get`/`get_mut`, `set`/`set_value`,
  `insert`, `insert_after_index`, `insert_integer_after`, `remove`/`delete`,
  element-ID lookup, `get_by_id`, `get_mut_by_id`, `get_element_by_index`,
  `get_element_by_id`, `get_last`, `insert_after`, `insert_before`,
  `delete_by_id`, `move_after`, `move_before`, `move_front`, `move_last`,
  `move_after_by_index`, `splice`, nested `get_object_mut`/`get_array_mut`,
  `get_counter_mut`, `get_text_mut`, `push_counter`, `push_long_counter`,
  `push_dedup_counter`, `push_text`, value search, ID search, `len`,
  `is_empty`, `iter`, and `as_slice`.
  Mutation methods can return errors because they may create CRDT operations
  during `Document::update`.
- `JsonValue` is a simple enum wrapper around primitive, counter, text,
  tree, object, and array values.
- `JsonText` (JS `Text`) supports `edit`, `delete`, `empty`, `set_style`,
  `to_string`, `len`, `is_empty`, `values`, `id`, and `to_sorted_json`. It
  holds a copy of the CRDT text, applies each edit to it with a ticket from the
  update's change context, and records the matching `EditOperation` or
  `StyleOperation`, so later edits in the same update see earlier ones, as in
  JS.
- `JsonTree` (JS `Tree`) supports `edit` (JS `edit`/`editBulk`, splits
  included), `edit_by_path` (JS `editByPath`/`editBulkByPath`),
  `split_by_path`, `merge_by_path`, `style`, `remove_style`, `to_xml`,
  `to_json`, `size`, `index_to_path`, and `path_to_index`. Like
  `JsonText`, it holds a copy of the CRDT tree, applies each edit or style to
  it, and records the matching `TreeEditOperation` or `TreeStyleOperation`.
  Input nodes are `TreeNodeInput` values (JS `ElementNode`/`TextNode`),
  validated like JS `validateTreeNodes`, and tree edit op infos report the
  inserted nodes as `TreeNodeInput` values too.
- During `Document::update`, public object, array, counter, text, and tree
  values are temporarily attached to a recorder and hidden CRDT identity
  metadata.

JS/Go behavior:

- JS arrays expose many mutation methods that map to CRDT operations, including
  insert/add, set, delete/remove, move, and nested container access.
- JS/Go counters expose regular increase paths and dedup actor-add paths that
  create increase operations during document updates.
- JS/Go JSON values are tied to CRDT element identity and change context during
  document updates.

Gap:

- Rust public object, array, counter, and text values are only partially
  CRDT-aware.
  The existing method calls record operations, but the values are still not
  stable live wrappers around CRDT elements.
- Rust public arrays now expose element-ID access and lightweight
  `JsonArrayElement` values for read-only ID/value lookup. They still do not
  expose mutable JS-style `WrappedElement` proxies tied directly to live CRDT
  containers.
- Rust value-based array search uses `JsonValue` equality. For non-primitive
  containers, JS compares wrapped element identity; Rust callers should use the
  explicit ID search methods when identity matters.
- `JsonTree` does not port JS `styleByPath`,
  `removeStyleByPath`, `getNodeSize`, `getIndexTree`, `getRootTreeNode`, the
  position-range conversions (`indexRangeToPosRange`, `posRangeToIndexRange`,
  `pathRangeToPosRange`, `posRangeToPathRange`), or the test helpers. There is
  no `JsonArray::push_tree` or `JsonArray::get_tree_mut` either.
- Rust rejects two inputs with `YorkieError::InvalidArgument` that JS has no
  check for: an element whose type is `"text"` and a text root. In JS the type
  alone makes a node a text node, so such input would crash JS when it reads
  the missing value or children. As in JS and Go, the attributes of the root
  given to `JsonTree::new` are dropped.
- Setting a `JsonTree` read from a document into another key copies its
  current nodes. JS builds an empty tree in that case, because such a `Tree`
  has no initial root.
- `JsonText` does not port JS `indexRangeToPosRange`/`posRangeToIndexRange`
  (`TextPosStruct`), `toTestString`, `toJSForTest`, `createRangeForTest`, or
  the index/ID tree accessors.
- Setting a `JsonText` that already has content creates an empty text, as JS
  does for `root.k = new Text()`; there is no API to copy a text's content.

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
- Rust `OpInfo` keeps a separate internal `ArrayRemove` variant; the public
  `OperationInfo` delivered to subscribers folds it into `Remove { index }`,
  as JS does.

Expected direction:

- Keep extending context-backed public array editing while reducing the
  remaining clone/diff fallback surface.
- Add version-vector parameters to operation execution when sync/replay
  behavior is ported.

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
- Concurrent counter undo/redo is covered by
  `crates/yorkie/tests/integration_history.rs`; other change-level concurrent
  counter tests are still missing.

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
  `JsonValue::Text` for document-root materialization.
- The public `JsonText` facade edits through `ChangeContext` (see Public JSON
  API). Local edits and styles reject an index inside a UTF-16 surrogate pair,
  like JS `CRDTText.createRange`.
- `RgaTreeSplit::edit` returns the JS value-change list: one change per run of
  removed nodes between the kept ones, last run first, with the inserted value
  on the change at the insertion index (JS `deleteNodes`/`makeChanges`).
  `EditOperation` turns each change into one `OpInfo::Edit`, so edit events
  match JS `EditOpInfo`s.
- `CRDTTextValue.toJSON` attribute output follows JS: a stored value that
  parses as JSON is re-encoded, and one that does not is emitted as a string.
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
- Integration tests cover concurrent insertion at one position, insertion
  against deletion, overlapping deletions, style against insertion, and
  overlapping styles (`crates/yorkie/tests/integration_text.rs`). The JS
  Peritext cases ex4-ex9 and the LWW tombstone-ticket checks are not ported;
  the latter read CRDT internals the public API does not expose.
- JS `deleteNodes` keeps a node the deleter saw live but someone else removed
  and records the deletion on it (`recordRemoval`, the removal ledger used by
  identity-preserving undo). Rust keeps the earlier rule: a node is removed
  when its creation is known and it is not yet tombstoned.
- Re-encoded numeric attribute values can differ in form from JS:
  `serde_json` writes the stored `1.0` as `1.0` where JS writes `1`.
- Version-vector-aware edit/style conflict behavior is modeled at the internal
  helper level and is passed through `Change`, but it still needs broader
  multi-change replay coverage at the operation layer.
- Rust strings cannot represent invalid standalone UTF-16 surrogate halves.
  Local edits refuse a mid-pair index, but a remote position inside a pair
  (which only an SDK without that check can send) still splits the node with
  lossy UTF-16 decoding, as Go does.

Expected direction:

- Continue porting JS history and multi-client text scenarios at the operation
  layer.
- Replace rebuild-on-mutation indexing with stable node handles to align the
  write-side implementation more closely with JS/Go.

## Tree

Current Rust behavior:

- `TreeNodeId` models the same creation-ticket plus UTF-16 offset identity used
  by JS `CRDTTreeNodeID` and Go `TreeNodeID`.
- `CrdtTree` stores a root `TreeNode`, rebuilds the set of node IDs after
  each structural change, and resolves split-node floor lookups only when the
  requested ID has the same creation ticket. Nodes are found by walking the
  tree, so each lookup costs a tree walk where JS and Go keep an LLRB map.
- `TreeNode` supports element nodes, text nodes, optional `Rht` attributes,
  tombstones, split-link metadata, merge metadata, JSON/XML output, UTF-16 text
  sizing, active attribute data size, and internal GC pair discovery.
- `CrdtTree` can convert between linear tree indexes, paths, and CRDT tree
  positions using the JS/Go element padding and text-child path rules.
- `CrdtTree` resolves edit and style positions like JS
  `findNodesAndSplitText`: it finds the parent and left sibling by node ID,
  removed nodes included, follows `insPrevID` from the start of a split node,
  splits the text node the position falls in, and moves the left sibling past
  nodes inserted at the same position after the edit, as in RGA. It advances
  unknown split siblings via `insNextID` and narrows collection ranges across
  split siblings.
- Edit ranges are walked over indexes that count removed nodes, as JS
  `traverseInPosRange` walks tombstones, and content is inserted right after
  the left sibling. Content inserted into a parent removed concurrently is
  born removed and registered as garbage.
- Tree edit changes follow JS `makeDeletionChanges`: one deletion change per
  run of adjacent removed tokens, last run first, with the inserted nodes
  attached to the change that starts at the insertion index. An edit that
  changes nothing visible reports no change, and style changes skip removed
  nodes and report the range of the node's start token.
- `CrdtTree` can apply simple style/remove-style ranges to visible element
  tokens, split text nodes at style boundaries, ignore text-only ranges after
  applying the boundary splits, collect tree style operation info, and report
  reverse-operation inputs. It also has the first version-vector-aware split
  sibling propagation used by JS/Go for style/remove-style collection.
- `CrdtTree` can apply split-free element insert/delete edits and text-node
  split insert/delete edits, collect tree edit operation info, register removed
  node GC pairs, and report reverse operation inputs.
- `CrdtTree::edit` ports JS `CRDTTree.edit` step by step, by node ID:
  - 01-1: the ends advance past the split siblings the editor did not know,
    stopping at the editor's own split products (§7.7), and the traversal is
    narrowed when a concurrent split put the ends in different parents;
  - 02: the traversal removes what the editor knew, reaches the unknown split
    siblings of a deleted element (JS `collectUnknownSplitSiblings`), and
    resolves concurrent deletions last writer wins;
  - 03: a merge moves every child of a merged element, removed ones too, to
    the resolved merge target (chained merges stay flat), stamps
    `mergedFrom`/`mergedAt` on the first move only, points the source at the
    target, and tombstones in place what would close a cycle;
  - 03-1: deleting a merge source deletes the children it gave away, unless
    the edit's own positions declared it (JS `declaredBoundaries`);
  - 04: splits use physical child offsets, keep merge-moved children of the
    level (Fix 8) and concurrent inserts at the boundary (§7.3) on the left,
    move an empty product in front of a re-parented sibling (§7.4), advance
    per level past unknown split siblings (§7.5), order same-boundary splits
    by ticket (JS `orderSameBoundarySplit`), and link the next split sibling
    back to the product. A split is reported as one empty change where it
    opened;
  - 05: content inserted at a position declared in a merged-away parent is
    stamped with that parent and the merge ticket (§9.4).
- The tickets of the elements a split creates come from the recorded
  `TreeEditOperation.split_tickets` (JS `splitTickets`), then from the JS
  fallback that rebuilds them from `executed_at` and the top-level content
  count. The wire shape and the proto converter carry `split_tickets`.
- Purging a removed node links its split siblings to each other, as JS
  `CRDTTree.purge` does.
- `rebuild_merge_state` follows JS `rebuildMergeState` for trees loaded from
  a snapshot.
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
- Tree undo and redo are not ported. Reverse tree edit operations are still
  built for insert/delete, pure split and merge edits, but the document
  history drops them (see Undo and Redo), and a reverse that
  does not fit the tree after a concurrent merge is dropped rather than
  failing the edit, and the JS reverse machinery (restore spans, split
  reconciliation, `mergeSourceOf`, `redoSplitLevel`, `dropDuplicateContents`
  for copy-reinserted content) is not ported.
- Like `CrdtText`, Tree text-node splitting uses valid Rust strings. Splitting
  inside an invalid standalone UTF-16 surrogate edge would need the same
  deliberate representation choice as Text.
- A split does not register the attribute tombstones its product copies
  (JS `pushPendingGCPair` for `split.getGCPairs()`): the root keys GC pairs by
  ID string, and the copy carries the same attribute IDs as the original. They
  stay uncollected garbage; content is unaffected.
- TreeEdit content decoded from the wire keeps the split links and merge
  lineage it carries; JS drops them (`dropEngineOnlyLinks`). Content created
  by JS, Go, or Rust editors never carries them.
- Rust style resolution does not port the JS §9.4 interloper guards
  (`mergedAnchorInterloperGuard`, `reversedFromAnchorRecovery`) or the
  declared-lineage checks of `styleTargets`. The ported style matrices,
  merge and split included, converge and match JS.
- Operation-time GC registration exists for split-free tree-node deletion,
  text-node split deletion, and visible-boundary merge source tombstones, but
  concurrent element split/merge deletion paths still need JS/Go parity.
- Wire conversion exists for full `JSONElement.Tree` payloads and tree
  edit/style operation bodies, but broader JS/Go protocol replay fixtures are
  still needed.
- All the matrices of JS `tree_concurrency_test.ts` are ported, merge and
  split operations included. Every combination converges, and on 2026-11-08
  each ended with the same XML as the JS SDK run against the same server.
- Tree attribute JSON output still uses its own scalar parsing helper. JS
  re-encodes every value through `JSON.parse`, emits `"attributes":{}` for a
  node whose attributes were all removed, and does not escape text in
  `toXML`, where Rust escapes `&`, `<`, and `>`; Rust XML attributes also do
  not escape `'` as JS does.
- Rust `TreeNode::can_style` still refuses a removed node unless the style is
  later than the removal, where JS styles removed nodes too. Removed nodes
  report no change either way, so only an undo restore could tell them apart.

Expected direction:

- Extend path/index conversion tests around removed nodes and mixed
  element/text children so edit/style operations can reuse the same position
  semantics.
- Port Tree undo and redo, with the JS split and merge reverse operations.
- Extend Tree style parity from direct split-sibling propagation toward the full
  version-vector and concurrent style matrices in JS/Go.

## Change, ChangeContext, and ChangePack

Current Rust behavior:

- `ChangeContext` issues time tickets and stores operations.
- `Document::update` uses an update-time recorder backed by `ChangeContext` for
  current public object and array mutation methods.
- `Change` executes operations and stacks reverse operations.
- `Document::update` runs the change on a copy of the CRDT root and keeps the
  copy only when every operation succeeds, as JS runs a change on its clone
  root first. A change that fails partway leaves the document as it was:
  JSON, CRDT root, pending changes, history, and change ID. Nothing reached
  the root, so no lamport is burned; JS burns it only when its second pass,
  on the root, fails after the clone pass succeeded, which cannot happen
  here. A presence-only change runs without a copy.
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
- Presence is implemented for `Put`/`Clear`: `Document::update_with_presence`,
  presence-only changes, remote presence application, snapshot presences, and
  online-client tracking. Rust never emits JS `Patch` changes (it decodes them);
  `disablePresence` is not ported. Presence undo/redo is ported (see Undo
  and Redo).
- Document events follow JS `reconcilePresence`: local presence updates,
  remote presence changes, and status transitions publish
  `PresenceChanged`/`Watched`/`Unwatched`. A remote `Clear` takes the actor
  offline. The client publishes `Initialized`, watch-driven
  `Watched`/`Unwatched`, `ConnectionChanged`, and `SyncStatusChanged`.
  Broadcast is a channel event, not a document event, as in JS. JS
  `AuthError`, `EpochMismatch`, `WriteRejected`, `LocalChangesDropped`, and
  `PersistDisabled` events are not ported. `ChangeInfo` omits JS
  `clientSeq`/`serverSeq`/`rawChange`.
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

- `Document::history()` returns a `HistoryRef` with `can_undo`/`can_redo`;
  `Document::history_mut()` returns a `HistoryMut` that also has
  `undo`/`redo` (JS `doc.history`). The stacks keep 50 entries each (JS
  `MaxUndoRedoStackDepth`).
- A local change pushes its reverse operations, newest first, and the
  previous values of the presence keys set with
  `PresenceProxy::set_with_history` (JS `addToHistory: true`). A change with
  operation info clears the redo stack; a presence-only change does not.
- Object set/remove, array add/remove/move/set, counter increase, text
  edit/style, and presence are undoable. An undo or redo is a new local
  change: it is queued for the server, publishes `LocalChange`, and replays
  with `OpSource::UndoRedo` after a snapshot (JS `replaySourceOf`). An empty
  stack makes `undo`/`redo` a no-op, as in JS.
- An array set and an undo that adds an element back re-point the stacks to
  the element's new creation time (JS `reconcileCreatedAt`). A remote or
  replayed text edit shifts the undo text edits of the same text (JS
  `reconcileTextEdit`/`reconcileOperation`).
- An undo text edit is refined against the current split chain when it runs,
  and the change keeps the refined positions for other replicas. JS does this
  by mutating the operation inside `execute`; Rust refines in
  `Change::execute_undo_redo` before each edit runs.
- A snapshot clears both stacks, as in JS `applySnapshot`.
- An undo or redo runs on a copy of the CRDT root, like a local update (see
  Change, ChangeContext, and ChangePack). One that fails leaves the document
  as it was and queues nothing. As in JS, the popped entry is dropped. The
  re-pointing of the other entries (`reconcile_created_at`) waits until the
  change has run, so a failed one leaves them as they were; JS has already
  re-pointed them to tickets the failed change never committed.

JS/Go behavior:

- JS reverses Text edits by identity (`RestoreSpan`, `restoreMode`,
  `retombstoneSpans`, and the removal ledger `recordRemoval`/
  `isRemovedInView`), so overlapping undos of concurrent deletions restore
  the original text once.
- JS undoes and redoes Tree edits and styles, with split reconciliation
  (`reconcileTreeNodeID`, `reconcileTreeEdit`, split tickets).
- JS has a `maxUndoDepth` document option, `clearHistory`, and refuses
  undo/redo during an update and while minting is suspended (epoch
  re-anchor). Its change events carry `source: OpSource.UndoRedo`.

Gap:

- Undo reverses are positional, as JS was before identity-preserving restore:
  an undone deletion is inserted again as new content. Two clients undoing
  overlapping deletions converge on duplicated content. The two JS
  correctness cases for this (`history_text_test.ts`, skipped upstream) are
  ignored in `crates/yorkie/tests/integration_history.rs`. The
  `restore_spans`/`restore_mode` proto fields stay at their defaults, and the
  removal ledger is not ported (see the Text section).
- Tree undo and redo are not ported. Tree edits and styles push no reverse
  operation, so an undo never replays a tree change, and a change that mixes
  tree and other edits is undone only for the other edits.
- `maxUndoDepth`, `clearHistory`, and the minting-suspended refusal are not
  ported. Rust change events do not carry the JS `source` field, so an
  undo's `LocalChange` looks like any other local change.

Expected direction:

- Port identity-preserving Text restore together with the removal ledger.
- Port Tree undo and redo with the JS split and merge reverse machinery.

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

- Presence `Put`/`Clear` changes and snapshot presences are converted; each
  value travels as its JSON encoding. As in JS `fromPresenceChange`, every
  type decodes so a stored change cannot wedge sync: UNSPECIFIED and DELETE
  decode as an empty `Put`, and PATCH decodes as the receive-only
  `PresenceChange::Patch` (absent presence becomes `{}`), which merges onto the
  actor's presence on apply and encodes back to PATCH.
- Go has a nil-pack converter error case. Rust takes `&api::ChangePack`, so
  that exact nil case is excluded by the function signature; the missing
  checkpoint case is covered.
- Snapshot bytes are carried in `ChangePack`, and the protocol converter
  decodes `Snapshot.root` into a core snapshot root for
  `Document::apply_change_pack`. Snapshot presences replace the document's
  presences.
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

- `yorkie-client` is split into `client`, `options`, `attachment`,
  `transport`, `connect`, `protocol`, `sync_loop`, `watch`, and `error`
  modules, with `lib.rs` kept as a re-exporting crate entrypoint.
- `ClientOptions` carries the JS/Go-shaped base options: RPC address, client
  key, API key, metadata, user agent, sync-loop duration, retry delay,
  reconnect delay, channel heartbeat interval, and watch heartbeat interval
  (JS `watchHeartbeatInterval`, default 20s). Defaults match the JS/Go values
  where both implementations already agree.
- `SyncMode`, `DeactivateOptions`, `AttachOptions`, `AttachChannelOptions`,
  and `DetachOptions` are public client-surface types. `AttachOptions` has
  `initial_root`, `initial_presence` (default empty), `sync_mode` (default
  `Realtime`), `document_poll_interval`, and `schema`, as in JS.
- `Client` is a cloneable handle over `Arc<ClientInner>`. It owns an
  `Arc<dyn ClientTransport>`: `Client::new` builds a `ConnectTransport`, and
  `Client::with_transport` takes any transport (tests use a mock or wrap
  `ConnectTransport`). Its methods take `&self`. Because it spawns tokio
  tasks, it must be used inside a tokio runtime.
- Client calls (`activate`, `deactivate`, `attach`, `detach`, `sync`,
  `remove`, `change_sync_mode`, and the channel calls) and each sync-loop
  round run one at a time on an async task queue (JS `enqueueTask`). Watch
  streams do not use it; each broadcast try does, but not the waits between
  tries.
- Documents are shared as `SharedDocument` (`Arc<std::sync::Mutex<Document>>`).
  The client locks a document only for synchronous steps (build a pack, apply
  a response, publish an event) and never across an `.await`, and it never
  holds a document lock together with its own state lock. Event callbacks run
  under the document lock, so a callback must not lock the same document
  again.
- `Client::activate` sends client key, metadata, and shard key and starts the
  sync loop. The actor stamped into changes is the response `actor_id`, or
  `client_id` when it is empty (JS `res.actorId || res.clientId`); requests
  keep sending `client_id`.
- `Client::deactivate` sends client ID and synchronous flag, marks sync/watch
  conditions inactive, stops the sync loop and every watch loop, drops every
  attachment, and sets each attached document to `Detached` (JS
  `deactivateInternal`).
- `Client::attach` rejects an inactive client, a document that is not
  `Detached`, and a key that is already attached or attaching
  (`ClientError::AlreadyAttached`, JS `ErrAlreadyAttached`). It stamps the
  actor, sets `initial_presence` in a local change, sends the attach request,
  stores returned max-size/schema-rule metadata, and applies the returned
  pack. It then re-checks that the client is still active in the same session
  (otherwise the document goes back to `Detached` and the call fails with
  `ClientNotActivated`), records the attachment, and in a realtime mode opens
  the watch stream and waits until it is initialized or its first attempt
  ends, as JS awaits `runWatchLoop`. Finally it applies `initial_root` for
  missing keys and returns the same `SharedDocument`.
- `Client::detach` clears the presence first (JS `p.clear()` before detach),
  sends client ID, document ID, local `ChangePack`, `remove_if_not_attached`,
  and shard key, applies the returned pack, updates document status, and
  removes the attachment, which stops its watch loop (JS `detachInternal`).
- `Client::remove` stamps the actor, sends a `ChangePack` marked as removed,
  applies the returned pack, and removes the attachment.
- `Client::sync(Some(doc))` pushes and pulls that document; `sync(None)` syncs
  every attachment in its own sync mode, one after another (JS runs them in
  parallel). The push-only response guard follows the client-side JS
  behavior. A pulled removal drops the attachment. Every sync publishes
  `SyncStatusChanged(Synced | SyncFailed)`.
- `detach`, `sync`, `remove`, and `change_sync_mode` find the attachment by
  key and require it to hold the same `SharedDocument`. Work that resumes
  after an `.await` (a sync, a watch message) touches or removes an
  attachment only if it still holds the document it started with, so a newer
  attachment of the same key is never dropped.
- The sync loop (`sync_loop.rs`, JS `runSyncLoop`) is one tokio task per
  active client. Every `sync_loop_duration` it syncs, in parallel, each
  attachment that needs it (JS `needRealtimeSync`): `Realtime` with local
  changes or a pending remote-change notice, `RealtimePushOnly` with local
  changes (push-only request), `Polling` once per poll interval, never
  `RealtimeSyncOff` or `Manual`. The notice is cleared before the request and
  restored if the sync fails. After a retryable error (network failures and
  the Connect codes JS retries) it waits `retry_sync_loop_delay`; any other
  error stops the loop.
- The watch loop (`watch.rs`, JS `runWatchLoop`/`runWatchStream`) is one
  tokio task per attachment in `Realtime`, `RealtimePushOnly`, or
  `RealtimeSyncOff`. It opens the unified `Watch` RPC with a document
  descriptor, `actor_id`, and `heartbeat_interval_ms`, publishes
  `ConnectionChanged(Connected)`, and marks a pull as needed. Initialization
  sets the online clients and publishes `Initialized`
  (`Document::apply_watch_init`); `DOCUMENT_CHANGED` marks a pull as needed;
  `DOCUMENT_WATCHED`/`UNWATCHED` update the online set and publish
  `Watched`/`Unwatched` (`Document::apply_watched`/`apply_unwatched`);
  heartbeats are ignored. When the stream fails or ends it clears the online
  clients, publishes `Initialized` and `ConnectionChanged(Disconnected)`, and
  reconnects after `reconnect_stream_delay`; a non-retryable error stops it.
  Once the server reports a nonzero heartbeat interval, a stream silent for
  three times that interval (1s to 24h) is treated as half-open and
  reconnected (JS `watchIdleTimeout`).
- `Client::change_sync_mode` follows JS `changeDocumentSyncMode`: switching to
  `Manual` or `Polling` closes the watch stream (publishing the same
  disconnect events), switching from those to a realtime mode opens it and
  waits for it, and switching to `Realtime` marks a pull as needed.
- Loops hold `Weak<ClientInner>`, and their handles live in the client state
  and abort the loop when dropped, so dropping every `Client` handle stops
  them as well. `Client::running_task_count` (hidden) lets tests check that.
- `yorkie-client::protocol` converts activate/deactivate, attach/detach/remove,
  push-pull, and watch transport DTOs to and from generated protobuf messages.
  Shard keys remain on transport DTOs because JS/Go send them as request
  metadata. Unknown watch bodies (channel events, unknown event types) are
  skipped.
- `ConnectTransport` implements the unary RPCs and the server-streaming
  `Watch` RPC over the Connect protocol (reqwest, binary protobuf,
  `application/connect+proto` envelopes, end-stream errors decoded into
  `ClientError::Rpc`).
- `Document` stores max-size and schema-rule metadata received during attach,
  and `Document::update` checks each local change against them before
  committing it, as JS `update` does: schema rules first, then the size. A
  rejected change leaves the document untouched. Remote changes and undo/redo
  are not checked, as in JS.

JS/Go behavior:

- Clients support activate/deactivate, attach/detach, sync, watch, presence,
  stream handling, retry/error behavior, and document status transitions.
- JS `handleConnectError` refreshes auth tokens on `Unauthenticated`, parks
  documents whose writes the server rejects for size, retries
  `ErrTooManySubscribers`, and deactivates the client on
  `ErrClientNotActivated`/`ErrClientNotFound`.

Gap:

- Retry decisions use only the Connect code. `ClientError::Rpc` carries the
  Yorkie error code from the error details (JS `errorCodeOf`), but only
  `ErrSessionNotFound` (channel refresh) is acted on: `ErrTooManySubscribers`
  retries, write-rejected parking, the auth-token refresh and its backoff,
  and deactivation on `ErrClientNotActivated`/`ErrClientNotFound` are not
  ported. `AuthError`/`EpochMismatch`/`WriteRejected` events are not
  published.
- `DOCUMENT_BROADCAST` watch events are ignored, as in JS: broadcasts travel
  on channels.
- Detach does not mark the attachment as detaching (JS `markDetaching`); the
  task queue keeps the sync loop from running during a detach instead.
- A push-only or sync-off reply that carries changes is skipped whole; JS
  still takes the push acknowledgement (`acknowledgePushedChanges`).
- Integration tests set schema rules and the size limit with
  `Document::set_schema_rules` and `set_max_size_per_document` after
  attaching. JS creates them through the admin API (`CreateSchema`,
  `UpdateProject`) and attaches with a schema key; those cases, and the
  `UpdateDocument` schema cases, are not ported.
- When a tree text node carries several disallowed marks, the schema error
  names the first by key; JS names the first set.
- `DeactivateOptions::keepalive`, `disableGC`, auth token refresh,
  gRPC-web/Connect transport choice, Go TLS options, and Go receive-size/logger
  options are not modeled yet.

Expected direction:

- Use the decoded Yorkie error codes so retry, parking, and auth handling
  can follow `handleConnectError`.

## Channels and Broadcast

Current Rust behavior:

- `Channel` (`yorkie-client/src/channel.rs`) ports JS `Channel`: key
  validation, status, actor, session ID, session count with seq ordering
  (`updateSessionCount`), subscriptions for all events or one broadcast
  topic, and `broadcast`. It is shared as `SharedChannel`
  (`Arc<std::sync::Mutex<Channel>>`), locked like `SharedDocument`.
- Channels follow the JS RefreshChannel-only lifecycle. `attach_channel`
  registers the channel and starts the sync loop; the client need not be
  active. The first `RefreshChannel` (no session ID) carries the client key
  and metadata, and its reply gives the session ID (the channel becomes
  `Attached`) and, for a client without one, a client ID that activates the
  client. Later refreshes run every heartbeat interval (default 5s, JS
  `DefaultChannelHeartbeatMs`) and publish `PresenceChanged` when the count
  changes. `ErrSessionNotFound` clears the session so the next round
  creates one; any other failure publishes `SyncError` and goes through the
  sync loop's retry rules.
- `Realtime` channels open the unified `Watch` RPC with a channel descriptor
  after their first refresh, and publish `Initialized`, `PresenceChanged`,
  and `Broadcast`. `Polling` channels only refresh; `Manual` channels refresh
  only through `sync_channel`. `change_channel_sync_mode` opens or closes the
  stream and resets an unpinned heartbeat interval.
- `detach_channel` is local, as in JS: the server expires the session by TTL
  (15s, checked every 10s), so peers see the count fall up to ~25s later.
  `deactivate` detaches channels too. `peek_channel` reads a count without
  activation or a session.
- `Channel::broadcast` publishes `LocalBroadcast`; the client that attached
  the channel sends it in the background through the `Broadcast` RPC with
  the JSON encoding of the payload, retrying retryable failures after 1s,
  2s, 4s, ... (at most 20s) up to `BroadcastOptions::max_retries`, and passes
  a final failure to `BroadcastOptions::error`. `Client::broadcast` sends one
  and waits for it.

Gap:

- JS `ChannelStatus.Removed` and the `AuthError` channel event are not
  ported (there is no auth-token handling yet).
- Deliberate differences from JS: `detach_channel` clears the session ID (JS
  keeps it, so a re-attached channel stays `Detached` until the old session
  expires); every broadcast try checks the channel is still attached (JS
  checks once); `Client::broadcast` waits for its retries (JS resolves after
  the first failed try) and returns a non-retryable failure (JS swallows it);
  `change_channel_sync_mode` to `Realtime` on a client that is not active
  yet leaves the stream to the first refresh (JS rejects the call with the
  same end state). `attach_channel` rejects a key already attached as a
  document or channel (JS overwrites the map entry).
- JS "Should throw error when broadcasting unserializeable payload" has no
  Rust case: a `serde_json::Value` payload always serializes.
- Payload numbers are encoded by `serde_json`, which can differ from JS
  `JSON.stringify` for edge cases (for example `1.0`).

## Error Behavior

Current Rust behavior:

- `YorkieError` is small and strongly typed for the current implementation.

JS/Go behavior:

- JS uses Yorkie error codes and messages.
- Go has typed errors that often map to server/client operation failure modes.

Gap:

- Rust error variants and messages are not yet aligned with JS error codes.
  `SchemaValidationFailed` and `DocumentSizeExceedsLimit` carry the JS
  messages and name their JS codes in their docs, but `YorkieError` has no
  code accessor.
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
- `OperationInfo` is public; keep new op infos in the JS `OpInfo` shape.
- Route CRDT mutations through `CrdtRoot` to keep the copied root index fresh.
- Prefer adding missing public JSON behavior before expanding network/client
  behavior.
- Keep JS/Go references in docs and planning notes, not in public code comments.
