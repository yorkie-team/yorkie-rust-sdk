# Current Porting Gaps

Last reviewed: 2026-11-09

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

## Out-of-Scope Items

These JS SDK features are deliberately not ported. Each has a JS source to
start from when it is picked up. Smaller differences are recorded in the
section of the area they touch.

| Item | JS source | Notes |
| --- | --- | --- |
| Offline persistence (doc-store, session-lock, persist policy) | `packages/sdk/src/client/doc-store.ts`, `client/session-lock.ts`, `client/persist-policy.ts`; `test/integration/offline_persistence_test.ts` | Also the `EpochMismatch`, `WriteRejected` (persist), `LocalChangesDropped`, and `PersistDisabled` events. |
| Devtools | `packages/sdk/src/devtools/` | Browser-only inspector protocol. |
| Revisions API | `packages/sdk/src/api/revision.ts`, `yorkie.ts`; `test/integration/revision_test.ts` | Admin-side document revisions. |
| Epoch re-anchor | `packages/sdk/src/document/document.ts` (`EpochMismatch`, minting suspension), `client/client.ts` | `ErrEpochMismatch` stops the sync loop without telling the document. |
| Disable-GC attach option | `packages/sdk/src/document/document.ts` (`disableGC`), `client/client.ts` | `AttachOptions` has no such field. |
| Tree undo and redo | `packages/sdk/src/document/operation/tree_edit_operation.ts`, `tree_style_operation.ts`, `document/crdt/tree.ts`, `document/history.ts`; receive side `api/converter.ts` (`fromOperation` TreeEdit case) | Split and merge reverse machinery, and the `TreeEdit` restore spans: a received `TreeEdit` with `restore_spans`, `retombstone_spans`, or `restore_mode` runs only its positional part, so a Rust replica diverges from a JS peer that undoes a Tree edit. See Undo and Redo. |
| Auth webhook integration test | `packages/sdk/test/integration/webhook_test.ts` | Needs a server with an auth webhook; the auth path is unit-tested with a mock transport. |
| Schema and project admin API cases | `packages/sdk/test/integration/document_test.ts`, `schema_test.ts` | `CreateSchema`, `UpdateProject`, `UpdateDocument` need the admin API. |
| History options | `packages/sdk/src/document/document.ts` (`maxUndoDepth`, `clearHistory`) | See Undo and Redo. |
| `ChannelStatus.Removed` | `packages/sdk/src/channel/channel.ts` | See Channels and Broadcast. |
| Text and Tree range helpers | `packages/sdk/src/document/json/text.ts`, `json/tree.ts` | Position-range conversions, `styleByPath`, `removeStyleByPath`. See Public JSON API. |

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
- A document edited before its first attach has its tickets re-issued to
  the client's actor by the attach, as JS `Client.attach` does with
  `setActor(actor, { reissue })`. `Client::attach` asks `claim_reissue`
  (generated client key, and the actor has not minted tickets in the
  document of that key) and then calls the hidden
  `Document::set_actor_with_reissue`; otherwise plain `set_actor`, which
  rewrites only each pending change's ID and each operation's
  `executed_at`. The re-issue runs only on a never-synced document
  (`never_synced`, JS `neverSynced`: detached, initial checkpoint, a version
  vector naming no other actor, and no snapshot or remote change taken in).
  `reissue_operations` (`reissue.rs`, JS `api/reissue.ts`) rewrites every
  ticket whose actor is the previous one and whose lamport is not 0, keeping
  lamports and delimiters; `reissue_version_vector` moves the previous
  actor's entry to the new one. The root and presences are rebuilt by
  replaying the rewritten changes on a fresh root, each with the source it
  first ran with. Everything is built aside and assigned at the end, so a
  failure leaves the document unchanged and fails the attach before any
  RPC. The undo and redo stacks are dropped, and no `DocEvent` is
  published, as in JS. Live-server cases: `integration_pre_attach.rs` (JS
  `pre_attach_test.ts`).

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
- Counter, Text, and Tree public editing methods are connected to the edit
  recorder. Text and Tree hold a copy of their CRDT value and apply each edit
  to it as the callback runs.
- Not re-issued, as in JS: a document under an explicit client key keeps
  the initial actor's tickets (every session of the key shares one stable
  actor, so a re-issue in a later session would mint the tickets an earlier
  one pushed); a second pre-attach document of a key whose first document
  already minted under the actor; and a document that has synced. Two
  clients with explicit keys that fill the same key before attaching can
  still push elements with the same `created_at`.
- A position or element ID taken from the document before the attach
  (a Text or Tree position, an array element ID) names the old actor and no
  longer resolves after a re-issue. JS has the same limit.
- No capability token. JS guards the `reissue` option with a module-private
  `ReissueToken`; Rust cannot share a crate-private value with
  `yorkie-client`, so the entry point is a `#[doc(hidden)] pub` method, like
  the other client-only `Document` methods (`apply_change_pack`,
  `garbage_collect`), and `has_minted_operations` is hidden public too. JS
  test "refuses a re-issue asked for without the token" has no counterpart.
- The ticket walk runs over the `wire` projection, destructuring every
  struct and variant without `..`, where JS walks the protobuf message by
  reflection. Coverage is the same and a new field fails to compile until
  the walk visits it. Nested Object, Array and Tree values are structural in
  the projection, so there are no `JSONElement` bytes to decode and
  re-encode (JS `walkNestedElement`). A Text value of `Set`, `Add` or
  `ArraySet` goes through the full Text encoding (content, ledger, links),
  as in JS.
- Rust Tree edits push no undo entry (Tree undo is out of scope), so the
  port of JS "keeps tree splits, undo/redo, tree style and array set"
  covers Tree splits and style only. Its Tree undo parts come back with
  Tree undo.
- A Text piece recreated after GC records a stand-in removal at lamport 0,
  followed by its cancel (see Text and RGATreeSplit). The lamport-0 rule
  leaves the stand-in under the previous actor while its cancel moves to
  the new one, which breaks the per-actor alternation the ledger pairs by.
  JS and Go do the same, so Rust follows; it needs GC on a never-synced
  document before its first attach, and is worth raising upstream.
- Where JS and Go differ, Rust follows JS:
  - Go renames every string-keyed map entry whose key is the previous
    actor in hex or base64 (`rekeyMap` in `api/converter/reissue.go`,
    meant for actor-keyed maps such as the deprecated
    `created_at_map_by_actor`), so a Tree or Text attribute named like the
    actor is renamed too. JS never rewrites map keys, and Rust neither.
  - Go sets its absorbed flag in `applyChanges` as a whole; JS only for a
    `Remote` change, because JS also replays local changes through
    `applyChanges`. Rust replays local changes through `apply_changes`
    too (`apply_snapshot`), so `absorbed_remote` follows JS.
  - The claim timing and the per-session ID are client differences (see
    Client, Sync, and Watch).

Expected direction:

- Continue moving public mutation methods onto the edit recorder.
- Either remove the clone/diff fallback or narrow it to explicitly unsupported
  compatibility cases once direct mutable JSON escape hatches are redesigned.

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
- The copies cost time. Every pair holds a deep copy of its parent, so a root
  with `n` members holds `n` copies of the root, and a local update, which
  runs on a copy of `CrdtRoot`, costs time quadratic in `n`. 500 updates on
  a root that grows to 500 members take about six minutes in a debug build.
  The two JS snapshot integration cases that build such a root are ignored
  for this (`integration_snapshot.rs`); both pass in a release build.
- Collection is slow in the same way. For each GC pair it walks the whole
  document twice, once to find the pair's barrier and container
  (`locate_gc_pair`) and once to purge it, so a pass costs time in GC pairs
  times nodes. Each purge then refreshes the copied pairs of the container
  and of every ancestor up to the root, and refreshing the root re-copies
  every pair in the document, so each purge alone costs time in the size of
  the whole document.
- `actual_element_by_created_at` resolves a createdAt to the first match in
  the tree, and `deregister_element` drops the index entries and the gc
  charge (`size_in_gc`) by createdAt. JS `deregisterElement` drops an entry
  only if it names the element being collected, and keys `sizeInGC` by
  element, because undo restores a copy of a removed container whose
  members keep their createdAts while the tombstones are still there.
  Collecting such a tombstone in Rust could evict the live copy's entry, so
  a later remote change addressed at it would fail inside
  `apply_change_pack`. Not verified that Rust undo aliases members this way.

Expected direction:

- Keep all CRDT mutations routed through `CrdtRoot` until the ownership model is
  deliberately redesigned.
- Add tests that mutate nested object/array descendants and immediately verify
  `create_path`, `find_by_created_at`, and visible JSON output.
- Add a test that undoes an array removal, collects the tombstone, and then
  edits a restored member from a remote change; key the index and the gc
  charge by element identity if it fails.

## Garbage Collection

Current Rust behavior:

- `Document::apply_change_pack` collects garbage with the pack's version
  vector after the pack has applied and the checkpoint has moved, and not
  for a snapshot pack, as JS `applyChangePack` does. The server's vector
  covers only what this client pushed, so a pending local removal keeps its
  tombstone.
- A push-only or sync-off reply is taken as a push acknowledgement only
  (`Document::acknowledge_pushed_changes`, JS `acknowledgePushedChanges`):
  its remote state and its version vector never reach collection.
- `CrdtRoot::garbage_collect` holds a purge back until the node behind the
  tombstone is causally stable (the successor barrier of JS `dbff8892`, Go
  `ba82ed91`) for array positions, text nodes, and tree siblings, and
  repeats a pass that both purged and deferred.
- Before the collect loop, `CrdtRoot::prune_ledgers` (JS `pruneLedgers`)
  prunes the removal ledgers of every registered holder Text once: a removal
  whose cancel the vector covers is dropped with its cancel
  (`RgaTreeSplitNode::prune_stable`, JS `pruneStable`). A Text becomes a
  holder when an identity undo cancels a removal in it.
  `CrdtRoot::register_element` registers every Text with cancels that a Set
  or Add payload, a remote element, or an undo's re-set brings in (the
  ledger half of JS `registerInternalGCPairs`), and a root rebuilt from a
  snapshot registers its own. A holder with nothing left
  to prune, a removed one, or one no longer in the document is dropped.
- A Text tombstone whose ledger still matters is held
  (`RgaTreeSplitNode::held_from_purge`, JS `heldFromPurge`): a node with two
  or more uncancelled removals stays for good, and one whose ledger holds a
  removal or cancel the vector does not cover stays until it does. The hold
  is checked after the `removed_at` stability check and before the
  successor barrier, as in JS, and a held node is not counted as deferred.
- The collect loop reads a registered Text node's `removed_at` and size as
  they are at collection time, as JS reads the node it holds; Rust reads
  them for every registered node in one walk per pass
  (`CrdtRoot::text_gc_children`). A pair whose Text left with its container
  keeps a frozen copy of its node (`CrdtGcPair.detached`), which later passes
  read.
- An identity undo that revives a Text node unregisters its GC pair
  (`CrdtRoot::unregister_gc_pair_by_id`, JS `unregisterGCPair`). A pair whose
  node is live again but still registered is dropped by the collect loop,
  with its gc size reversed (JS `collect`).
- Splitting a Text tombstone registers the born-removed piece as a gc-only
  pair and reports no live size for it (JS `4f1f0123`, `pendingGCPairs`).
  Edit, style, restore, and retombstone drain those pairs into the change
  context.
- `Document::garbage_collect`, `garbage_len`, and `version_vector` are
  hidden public hooks, as JS exposes them for tests.

JS/Go behavior:

- JS skips collection for a document attached with `disableGC`.
- JS `registerInternalGCPairs` books both the ledger holders and the GC
  pairs of the tombstones inside an element it registers.

Gap:

- `disableGC` is not ported (see Out-of-Scope Items).
- `register_element` registers ledger holders but not the GC pairs of the
  tombstones inside a Text or Tree it registers (the GC-pair half of JS
  `registerInternalGCPairs`). Such tombstones are found only when a root is
  rebuilt.
- GC pairs are keyed by the child's ID string; JS keys them by parent and
  child ID. An undo's copy of a removed Text carries the original's node
  IDs, so a split in the copy can register a piece under an ID whose pair
  the original left behind, purged (`detached`) or replaced by the copy.
  JS books two pairs. Rust lets the copy's piece replace the old pair
  (`CrdtRoot::register_gc_pair_by_id`): the old charge leaves gc as
  `collect` would take it, and the new one is booked. So a detached node
  held by two removals stops being counted once a copy's piece takes its
  ID, where JS keeps counting it. Since the copy's own tombstones are not
  booked (above), gc after such an undo still differs from a rebuild.
- Only removed attributes and Array and Tree children toggle off on a
  second registration. A Text node replaces its pair instead: every revive
  unregisters first, so a second registration is never a revive. Were a
  revived node left registered and removed again, JS would drop the pair
  and keep the tombstone. Telling a Text node apart walks every Text, but
  only when a pair already holds the ID. This replaces a `debug_assert!`
  that refused such a registration, with release builds toggling the old
  pair off and leaving the new piece unbooked.
- `unregister_gc_pair_by_id` takes a revived node's size out of gc only. JS
  moves it from gc back to live, because its registration took it out of
  live. Rust registration leaves a text removal's size in live (the
  `doc_size` gap below), so moving it back would count it twice. Live size
  after a Text undo therefore differs from JS until that gap is fixed; gc
  matches a rebuild.
- A pair whose Text left with its container skips the successor barrier.
  JS still checks `purgeBarrierAt` on the detached split and can defer when
  a concurrent insertion is not yet covered; Rust drops a stable, unheld
  detached pair at once. Only the timing of `garbage_collect`'s count,
  `garbage_len`, and `doc_size.gc` can differ: the dropped node is no longer
  in the document, so content and snapshot bytes do not. A detached node
  held by two removals stays registered, with its node copy and ledger,
  for good, as in JS.
- `retombstone` registers a GC pair only for a piece whose `removed_at` it
  set. JS registers one unconditionally. They differ only on a replayed
  retombstone that changes nothing.
- The attribute tombstones a Text split or a restore's recreate copies are
  not registered (JS `bookCopiedAttrTombstones`): the root keys GC pairs by
  ID string, and a copy carries the original's attribute IDs, so booking it
  would toggle the original's pairs off. They stay uncollected garbage;
  content is unaffected. Tree splits have the same gap (see Tree).
- The split `find_restore_anchor` makes in its last fallback
  (`find_node_with_split`) reports no size diff. JS and Go discard it too.
- Collection cost: once any GC pair is registered, every pass walks every
  Text node of the document and builds an ID string for each
  (`text_gc_children`). The walk is taken before any purge in the pass; a
  purge does not change another registered node's state, so the snapshot
  holds for the pass.
- The running `doc_size` disagrees with a root rebuilt from the same content:
  removing a text or tree node does not move its size from `live` to `gc`,
  and a moved element's ticket is not charged the way a rebuild charges it
  (JS `accMovedElement`). This holds with or without collection. The JS
  barrier tests check the running size against a rebuild; the Rust ports
  check the garbage count only. Removed elements and their descendants do
  match a rebuild (JS `sizeInGC`), before and after collection.
- JS keeps a clone root and collects it too (`getGarbageLenFromClone`).
  Rust has no clone to collect: a local change runs on a copy that replaces
  the root.
- The JS case "gc targeting nodes made by deactivated client"
  (`gc_test.ts:1397`) deactivates with `synchronous: true`, which the
  memory-backed server fails ("change not found"), as the JS NOTE says. It
  needs a Mongo-backed server. CI runs the memory backend, so the Rust port
  (`integration_gc.rs`) is ignored and never runs there.
- With collection on, JS object case `object_test.ts:923` fails: the undo
  names an object that has been purged. JS runs it with `disableGC`, which
  is not ported, so the Rust port stays ignored.

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
- Wire conversion maps text and tree attribute `Rht` nodes to protobuf
  `NodeAttr` payloads and carries `is_removed` both ways, as JS
  `toTextNodes`/`fromTextNode` (since yorkie-js-sdk#1364) and Go do. A
  removed text attribute decodes as a tombstone through `set_internal`, so
  it stays hidden and registers its GC pair after a snapshot, a Text value
  payload, or a re-issue. JS keeps a removed attribute's value on the wire
  and Rust follows it; Go clears that value in `RHT.Remove` and
  `SetInternal`, so Go writes an empty `value` for the same tombstone.
- `RhtNode::data_size` charges a tombstone's value, and charges the stored
  string's UTF-16 length. JS `RHTNode.getDataSize` (since
  yorkie-js-sdk#1392, `8fb5a71c`) charges a tombstone's key only, and the
  logical value's UTF-8 length, so `doc_size` differs from JS for removed
  Text and Tree attributes and for non-ASCII or JSON-quoted values. Within
  Rust a round trip and a live document agree; only the absolute sizes and
  the size limit allowance differ from JS.

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
- Each `RgaTreeSplitNode` keeps the JS removal ledger
  (`crdt/rga_tree_split.rs`). `removed_at` is the latest uncancelled
  removal; a node's first removal is just `removed_at`, and a second removal
  or a cancel builds the ledger of removals and cancels. `record_removal`,
  `cancel_removal`, `recompute`, `prune_stable`, `held_from_purge`,
  `is_removed_in_view`, `removal_events`, and `set_removal_events` port the
  JS methods of the same names, and a split copies the ledger (JS
  `copyLedger`). An actor holds at most one uncancelled removal per node:
  `record_removal` ignores a second one, and a restore cancels only the
  undoing actor's own removal.
- A remote deletion removes only the nodes live in the deleter's view
  (creation known and `!is_removed_in_view(vector)`), and records the
  deletion without removing on a node it saw live but someone else removed
  (JS `deleteNodes`, `recordRemoval`). The LWW overwrite of `removed_at`
  through `can_remove` still happens. A local edit (no vector, or an empty
  one) keeps the `!is_removed()` test. `time::version_vector::ticket_known`
  (JS `ticketKnown`) reads an absent or empty vector as local, which knows
  every ticket, and an actor missing from a non-empty vector as unknown,
  even at lamport 0.
- `RgaTreeSplit::edit` also returns the removed spans the identity reverse
  needs: one per removed node in document order, LWW-overwritten nodes
  included, as JS `removedSpans`.
- `RgaTreeSplit::restore` and `retombstone` (JS `restore`, `retombstone`)
  revive and re-remove pieces by identity for an undo or redo, splitting at
  span boundaries. A piece GC purged is recreated along the purged pieces'
  own boundaries, anchored by `find_restore_anchor` (the nearest following
  piece of the same insertion first), chained left to right, with the
  insertion chain relinked to the nearest surviving pieces on both sides,
  and a stand-in removal by the undoing actor at lamport 0 recorded with its
  cancel. `refresh_spans` (JS `refreshSpans`) re-reads the
  attributes of each span from the pieces that still exist when a local
  undo or redo runs, so a style applied after the reversed edit is not
  lost.
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
- JS and Go share the removal ledger (JS `84068f70`, Go `8997daf2`). Where
  they differ, Rust follows JS:
  - A deletion over a node whose `removed_at` it overwrites (LWW) counts the
    node as removed in JS: it is in `removedValues` and `removedSpans`, and
    the edit's value changes run across it. Go keeps such a node
    (`deleteNodes` appends it to `nodesToKeep`). So a deletion over
    [live A][LWW-overwritten B][live C] reports one edit change in JS and
    Rust, and two in Go.
  - JS `deleteNodes` tests `createdAtVV && createdAtVV >= lamport`, so a
    vector entry of exactly 0 reads as unknown. Go (`ok && l >= ...`) and
    Rust `ticket_known` read it as known. Not reachable: only the head is
    created at lamport 0, and the only removal at lamport 0 is the restore
    stand-in, always recorded with its cancel, so it never becomes
    `removed_at`.
  - A piece an undo recreates after GC keeps the attribute tickets of the
    removed value in JS and Rust; Go stamps them with the undo's ticket. The
    interop fixtures show it (see Serialization).

Gap:

- `RgaTreeSplit` rebuilds its splay and node-ID indexes after structural
  mutations instead of using JS/Go-style linked nodes with stable per-node index
  handles. Read lookup now follows the same splay/ID-map route, but write-side
  index maintenance is still conservative. Restore and retombstone rebuild
  them after every revive, removal, and insertion; JS splays one node.
- Integration tests cover concurrent insertion at one position, insertion
  against deletion, overlapping deletions, style against insertion, and
  overlapping styles (`crates/yorkie/tests/integration_text.rs`). The JS
  Peritext cases ex4-ex9 and the LWW tombstone-ticket checks are not ported;
  the latter read CRDT internals the public API does not expose.
- The ledger's per-actor index is derived eagerly from the removals and
  cancels and rebuilt on every change. JS keeps a lazy index, so a
  `recordRemoval` whose cancel is already indexed enters as live in JS until
  the next rebuild, and as cancelled in Rust. A cancel always comes causally
  after its own removal, so the case does not arise. The node's derived
  `PartialEq` compares the ledger lists in insertion order, so two converged
  replicas whose ledgers were built in different orders can compare unequal
  node by node; their JSON, text, and `removal_events` (ticket order) agree.
- A span decoded from the wire stamps its attributes with the operation's
  `executed_at` (JS `fromPbSpan`), while the undoing replica restores the
  removed value with its original attribute tickets. A replica that purged
  the piece and recreates it from a remote undo therefore holds later
  attribute tickets than the undoing replica. By reading, a concurrent style
  whose ticket falls between the two could win on one and lose on the
  other. JS has the same split; not tested.
- With GC on and insertions, a recreated piece can land after text another
  insertion placed between the gap and the following piece of its own
  insertion. JS and Go share this limit; the fuzz setting `gc=true
  insert=100%` is report-only in all three
  (`document/text_undo_fuzz_tests.rs`), with the same failing seeds as JS.
- A long undo churn on one piece is quadratic. Each local update or undo
  copies the whole Text several times (`Document::stage` clones the root,
  the JSON view is rebuilt, and result vectors clone nodes with their
  ledgers), and each copy costs O(ledger). JS keeps one clone root and
  applies each change to it in place. The ledger operations themselves are
  O(log n) or O(actors). Ledgers are pruned once their cancels are known
  everywhere, so only a long-offline replica churning one piece grows one
  this large; the port of JS "reads back a snapshot whose ledger it grew
  itself" (4112 events) takes about 8 s in a debug build.
- `crdt/rga_tree_split.rs` holds the restore and retombstone port in the
  same file (about 1960 lines); it could move to a submodule.
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
  A Set, Add or ArraySet element payload, by contrast, does drop the split
  links of every tree it holds, removed members included, both on decode and
  on the copy an undo restores (JS `dropSplitLinksInElement` in
  `crdt/split_links.ts`); its merge lineage is kept, as in JS.
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
  `Watched`/`Unwatched`, `ConnectionChanged`, `SyncStatusChanged`, and
  `AuthError`. Broadcast is a channel event, not a document event, as in
  JS. JS `EpochMismatch`, `WriteRejected`, `LocalChangesDropped`, and
  `PersistDisabled` events are not ported. `ChangeInfo` omits JS
  `clientSeq`/`serverSeq`/`rawChange`.
- Change packs can be projected to generated protobuf payloads and
  reconstructed back into core `ChangePack` values for the current operation
  set. Binary round-trip tests cover nested object/array/counter replay, and
  protocol snapshot tests cover root replacement through
  `Document::apply_change_pack`.
- Raw snapshot bytes that have not been decoded by the protocol converter are
  still rejected by core to keep protobuf parsing out of `yorkie-core`.
- `DocStatus` transitions (`Detached`, `Attached`, `Removed`) and the
  `SyncStatusChanged` and `ConnectionChanged` events are driven by the client
  (see Client, Sync, and Watch).

Expected direction:

- Add live change context before expanding public document mutation APIs.
- Extend snapshot handling with presence, GC, and cross-language binary
  fixtures before real client sync.

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
- Text edits reverse by identity, as in JS
  (`operation/edit_operation.rs`). The reverse of an edit that removed or
  inserted anything is an identity edit: `restore_spans` name the removed
  pieces by insertion ticket and UTF-16 interval, `retombstone_spans` the
  inserted ones, and `RestoreMode` (`Restore` for an undo, `Retombstone` for
  its redo) says which list is revived and which re-removed. Running it
  (`EditOperation::execute_identity`, JS `execute` with `restoreMode`)
  re-removes first and then revives, through `RgaTreeSplit::retombstone`
  and `restore`, so two clients undoing overlapping deletions restore the
  original text once. The two correctness cases JS still skips
  (`history_text_test.ts` "Case 3/5 correctness") run live in
  `crates/yorkie/tests/integration_history.rs`, as Go runs them.
- The reverse of a redo, and of an insertion's undo, names only the pieces
  the removal was recorded on. An identity reverse left with no spans
  becomes an empty undo edit at the head (JS `emptyReverse`), and an edit
  that changed nothing keeps a positional reverse.
- An undo text edit with content is refined against the current split
  chain when it runs, and the change keeps the refined positions for other
  replicas. An undo edit with no content moves to the head instead and runs
  as a no-op, before any refine could fail on a stale offset. An identity
  edit is not refined: its positions only anchor a recreated piece. JS
  refines inside `execute` and keeps the result on the operation; Rust
  refines in `Change::execute_undo_redo` before each edit runs
  (`EditOperation::refine_undo_positions`).
- An array set and an undo that adds an element back re-point the stacks to
  the element's new creation time (JS `reconcileCreatedAt`). A remote or
  replayed text edit shifts the undo text edits of the same text (JS
  `reconcileTextEdit`/`reconcileOperation`).
- A snapshot clears both stacks, as in JS `applySnapshot`.
- An undo or redo runs on a copy of the CRDT root, like a local update (see
  Change, ChangeContext, and ChangePack). One that fails leaves the document
  as it was and queues nothing. As in JS, the popped entry is dropped. The
  re-pointing of the other entries (`reconcile_created_at`) waits until the
  change has run, so a failed one leaves them as they were; JS has already
  re-pointed them to tickets the failed change never committed.

JS/Go behavior:

- JS undoes and redoes Tree edits and styles, with split reconciliation
  (`reconcileTreeNodeID`, `reconcileTreeEdit`, split tickets), and reverses
  a Tree edit by identity with `TreeEdit` restore spans.
- JS has a `maxUndoDepth` document option, `clearHistory`, and refuses
  undo/redo during an update and while minting is suspended (epoch
  re-anchor). Its change events carry `source: OpSource.UndoRedo`.
- Go `refinePos` fails on a stale offset past the end of a node (Go fuzz
  seed 152); JS snaps it to the end. Rust `refine_pos` snaps, as JS does.

Gap:

- Tree undo and redo are not ported. Tree edits and styles push no reverse
  operation, so an undo never replays a tree change, and a change that mixes
  tree and other edits is undone only for the other edits.
- **Interop: a Rust replica does not converge with a JS peer that undoes a
  Tree edit.** JS sends the undo of a Tree edit with `restore_spans`,
  `retombstone_spans`, and `restore_mode` on `TreeEdit` (`toOperation`
  TreeEdit case in `packages/sdk/src/api/converter.ts`,
  `document/operation/tree_edit_operation.ts`), and a JS receiver revives or
  re-removes the nodes by identity. The Rust decoder
  (`crates/yorkie-protocol/src/converter.rs`, the `TreeEdit` arm of the
  operation decode) drops the three fields and runs only the positional
  part, so the content a JS Tree undo brings back never reappears on the
  Rust replica. Text undo has no such gap: Rust sends and applies the
  identity fields of `Edit` as JS does.
- The positional part of an identity edit is not a harmless no-op for a
  peer that ignores the restore fields. It is the normalized `from` position
  (the head with a relative offset, as JS `toReverseOperation` writes it),
  and a decoder that drops the spans runs it as a plain edit: a Rust
  `Document` given such an edit fails with `InvalidTextPosition`, and by
  reading JS `splitNode` throws on it too. An SDK from before the identity
  fields would therefore fail on an undo from a current peer, rather than
  ignore it. The wire shape matches JS and Go; this is an upstream
  observation.
- `maxUndoDepth`, `clearHistory`, and the minting-suspended refusal are not
  ported. Rust change events do not carry the JS `source` field, so an
  undo's `LocalChange` looks like any other local change.
- JS `getUndoStackForTest`/`getRedoStackForTest` have no public Rust
  counterpart: the stacks are readable only from yorkie-core unit tests.
  The integration port of `doc_presence_test.ts:689` checks the stack depth
  with `can_undo` and the entries through undo and redo results instead.

Expected direction:

- Port Tree undo and redo with the JS split and merge reverse machinery,
  and the `TreeEdit` restore spans on both sides of the wire.

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
- A Text `Edit` carries `restore_spans`, `retombstone_spans`, and
  `restore_mode` both ways (`WireOperation::Edit`, `WireRestoreSpan`,
  `WireRestoreMode`). As in JS, `restore_mode` and the span lists are
  written only when a span list is non-empty, and an unspecified mode with
  spans reads as `Restore`. A span's attributes travel as a bare map and are
  stamped with the operation's `executed_at` on decode (JS `fromPbSpan`). A
  span with no `created_at`, a negative `start`, `end < start`, or
  `end - start` other than the UTF-16 length of its content fails the
  operation, and with it the change pack, with an `InvalidArgument` error,
  as JS throws `ErrInvalidArgument`.
- A snapshot `TextNode` carries the node's ledger beyond `removed_at` in
  `removal_events` (field 6), in ticket order (JS `toTextNodes`,
  `getRemovalEvents`). A node whose `removed_at` says it all writes none, so
  it encodes to the bytes it did before the field existed. On decode
  `set_removal_events` rebuilds the ledger; one that breaks the rules (a
  repeated ticket, or a result that does not reproduce `removed_at`) is
  dropped and the node keeps `removed_at` alone, without failing the
  snapshot or the pack (JS `fromTextNode`).
- A Text node's insertion link (`ins_prev_id`) follows JS `toTextNodes`
  and `fromText` (`ed25355e`). The encoder writes a link only if its target
  was emitted earlier in the same node list, so a link into a purged
  tombstone is dropped. The decoder keeps a link only if it names, by exact
  ID, a node already decoded from the same insertion at a lower offset, and
  then sets that node's `ins_next_id` too (JS `setInsPrev`); any other link
  is dropped, never an error. A position resolved through a link to a
  higher offset fails with `InvalidTextPosition` instead of underflowing.
  The re-issue's Text value round trip and every snapshot rely on this.
- `TreeEdit` restore fields, and `restore_mode` on `Set`, `Add`, and
  `ArraySet`, are ignored on decode and unset on encode. For `Set`, `Add`,
  and `ArraySet` this matches the JS converter; for `TreeEdit` it is the
  Tree undo gap (see Undo and Redo).
- Cross-language bytes for Text undo
  (`crates/yorkie-protocol/tests/text_undo_interop.rs`): JS `84068f70`
  wrote a change pack holding a deletion's undo (`Restore`) and redo
  (`Retombstone`) and a replace's undo and redo (both span lists), and a
  snapshot pack whose Text holds a ledgered node (a removal, its cancel, and
  a second actor's removal) and a piece recreated after GC with the lamport-0
  stand-in. Rust decodes each and encodes it back to the same bytes, the
  snapshot from the root it decoded; applies the change pack change by
  change and loads the snapshot, reading the text and JSON JS read; reads
  the ledgers node by node; and, making the same edits, writes the change
  pack byte for byte. `yorkie-core` builds the same snapshot root
  (`builds_the_root_of_the_js_ledger_snapshot_fixture`). The generators are
  `tools/interop/text_undo_fixtures_test.ts` and `.go`; the fixture README
  has the commands.

JS/Go behavior:

- Operations, changes, checkpoints, snapshots, and packs convert to protobuf
  payloads for sync.
- Go yorkie `54b96762` writes the Text undo change pack byte for byte as JS
  does. Its ledger snapshot differs in one place: a piece an undo recreates
  after GC carries its attribute under the undo's ticket, where JS keeps the
  insertion's (see Text and RGATreeSplit). That snapshot is a fixture of its
  own (`go_ledger_snapshot_pack.hex`), which Rust also decodes, re-encodes
  to the same bytes, and loads to the JS document.
- Where JS and Go differ on decode, Rust follows JS:
  - Go bounds the removal events of one node at 4096 in element payloads a
    client pushes (`api/converter/from_pb.go`, `maxTextRemovalEvents`). JS
    decodes only what the server accepted and has no bound; nor has Rust.
  - Go rejects an identity span whose `created_at` lies beyond the change's
    version vector (`pkg/document/crdt/text_restore_test.go`
    `TestTextRestoreRejectsForgedIdentity`). JS and Rust do not.
  - Go `fromJSONText` (`api/converter/from_bytes.go`) fails the element
    when a Text node's insertion link names no node it has decoded
    ("insPrevNode should be presence"); JS `fromText` drops the link and
    keeps the node, and so does Rust.

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
- Cross-language binary tests cover Text undo only (see above). Other
  payloads are checked against JS/Go in-repo converter scenarios, not
  against bytes JS or Go wrote.
- JS logs a warning (`logger.warn`) when it drops a malformed ledger. Rust
  drops it silently: `yorkie-core` has no logging facade and takes no
  dependency for one.
- JS also drops a ledger when one of its tickets is missing. prost cannot
  decode a missing element of a repeated message field, so Rust has no such
  branch.
- The span interval check runs twice, in the protocol decoder
  (`proto_restore_spans_to_wire`) and in the core wire decoder
  (`restore_spans_from_wire`), with different messages ("missing timestamp
  or invalid range" and "invalid range"). Only the first can see a missing
  `created_at` or a negative offset.
- Two kinds of span pass validation, as in JS, and Rust follows JS rather
  than reject them. A span whose `created_at` is the initial ticket names
  the head; recreating it after GC would put a node under the head's ID
  that shadows the head. A span with `start == end` (empty content) is an
  interval of nothing; by reading, `retombstone` then isolates and removes
  the piece that starts at `start`, as JS `findPiecesOverlapping` and
  `isolateRange` do, so replicas still converge. Hardening either belongs
  upstream.
- The public `wire` projection grew for the ledger: `WireTextNode` gained
  `removal_events`, `WireOperation::Edit` gained `restore_spans`,
  `restore_mode`, and `retombstone_spans`, and `WireRestoreSpan` and
  `WireRestoreMode` are new. Code that builds `WireTextNode` or
  `WireOperation::Edit` with a struct literal, or destructures either
  without `..`, no longer compiles: a semver-breaking change for users of
  `yorkie_core::wire`. None of these is `#[non_exhaustive]`, so a later
  field or variant would break the same way.
- JS and Go both have direct converter tests for root/object bytes and tree
  bytes. Go also has direct array bytes, change-pack, presence, and snapshot
  converter tests. Rust currently covers object/array simple payload bytes,
  root/object bytes at core wire level, standalone array/tree bytes at core
  wire level, object-GC bytes at core wire level, tree edit/style and
  merge-state root bytes at core wire level, tree protobuf payload round trips
  at protocol level, snapshot-root `ChangePack` protobuf round trips,
  simple and mixed change-pack replay, and a generated-protobuf snapshot apply
  path. Presence changes and snapshot presences are converted (see above).

Expected direction:

- Continue porting JS/Go in-repo converter scenarios before adding external
  binary fixtures for payloads other than Text undo.
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
- A client created without a `key` (or with an empty one) gets a random
  version 4 UUID from the OS CSPRNG, as JS `uuid()` does (`util/uuid`). It
  panics if the OS has no random source, where JS throws
  `ErrInvalidArgument`.
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
- Before stamping the actor, `Client::attach` decides whether to re-issue
  the document's pre-attach tickets (see Document Editing Model), as JS
  `claimReissue` and `markReissueClaim` do. `claim_reissue` is a pure
  query: true only for a generated client key (`key_generated`, JS
  `keyGenerated`) whose actor has no claim on the document key
  (`reissue_claims`, JS `reissueClaims`). The claim is taken after
  `set_actor_with_reissue` if the document re-issued or has minted
  operations, before the RPC, since a lost response may still have pushed.
  `remove_attachment` and `deactivate_internal` take it for a dropped
  attachment whose document minted while attached (JS `detachInternal`).
  Claims are never released and outlive deactivation, since a later
  session of the key has the same actor. Option validation runs before
  either step, so a rejected option neither re-issues nor claims.
  `AttachingGuard` is taken only while the attach's session is current, so
  an attach cannot claim a key a deactivation just freed before the
  deactivation has marked its claims.
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
  restored if the sync fails. A failed round is handled after it leaves the
  task queue, as in JS: after a retryable error it waits
  `retry_sync_loop_delay` (up to 5s longer while the auth token refresh
  backs off); any other error stops the loop. A document whose push is
  refused for its size (`ErrDocumentSizeExceedsLimit`, `ErrChangeTooLarge`)
  is parked instead, as JS `markWriteRejected` does: the loop skips it and
  goes on with the rest until a sync of it succeeds, such as an explicit
  `Client::sync`.
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
  clients, publishes `Initialized` and `ConnectionChanged(Disconnected)` (and
  `AuthError` for `ErrUnauthenticated`), and reconnects after
  `reconnect_stream_delay` (longer while the auth token refresh backs off);
  a non-retryable error stops it.
  Once the server reports a nonzero heartbeat interval, a stream silent for
  three times that interval (1s to 24h) is treated as half-open and
  reconnected (JS `watchIdleTimeout`).
- `Client::change_sync_mode` follows JS `changeDocumentSyncMode`: switching to
  `Manual` or `Polling` closes the watch stream (publishing the same
  disconnect events), switching from those to a realtime mode opens it and
  waits for it, and switching to `Realtime` marks a pull as needed. A switch
  to `RealtimePushOnly` or `RealtimeSyncOff` is counted on the attachment
  before it waits on the task queue (JS `pendingPullPauses`), so a sync
  already in flight drops the remote state of its reply and marks a pull as
  needed again. This check reads the attachment's mode when the reply
  arrives and skips a `Manual` document. The pull is marked in `Polling`
  too, as in JS; `Polling` ignores the mark and pulls on its interval.
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
- Server errors are handled as JS `handleConnectError` does
  (`Client::handle_connect_error`), using the Yorkie code and reason from
  the error's `ErrorInfo` metadata: size rejections are not retried,
  transient Connect codes and `ErrTooManySubscribers` are retried,
  `ErrUnauthenticated` refreshes the auth token and is retried, and
  `ErrClientNotActivated`/`ErrClientNotFound` deactivate the client locally
  (JS `deactivateInternal`). One-shot calls run it and return the error, as
  JS does in their `catch`.
- `ClientOptions::auth_token_injector` (JS `authTokenInjector`) gives the
  token `ConnectTransport` sends as the `authorization` header (JS
  `createAuthInterceptor`): at activation with no reason, and after each
  `ErrUnauthenticated` with the auth webhook's reason. A failing injector
  fails `activate` and one-shot calls with its error; the loops keep the old
  token and back off their injector calls (1s doubling to 30s, jittered),
  with the JS rules for overlapping calls (`refreshAuthToken`). `deactivate`
  refused with a token that cannot be refreshed deactivates locally and
  returns the injector's error. Documents publish `AuthError` for refused
  syncs (`ErrUnauthenticated`, `ErrPermissionDenied`) and watch streams
  (`ErrUnauthenticated`); channels publish it for refused broadcasts.

JS/Go behavior:

- Clients support activate/deactivate, attach/detach, sync, watch, presence,
  stream handling, retry/error behavior, and document status transitions.
- JS `handleConnectError` refreshes auth tokens on `Unauthenticated`, parks
  documents whose writes the server rejects for size, retries
  `ErrTooManySubscribers`, and deactivates the client on
  `ErrClientNotActivated`/`ErrClientNotFound`.

Gap:

- `EpochMismatch` and `WriteRejected` events are not published: a parked
  document only gets `SyncStatusChanged(SyncFailed)`, and an
  `ErrEpochMismatch` stops the sync loop without telling the document.
- JS's `deactivating` flag is not modeled: Rust `deactivate` holds the task
  queue, so the sync loop cannot run during it. The auth webhook path is
  covered by unit tests with a mock transport only; JS `webhook_test.ts`
  needs a server configured with an auth webhook and is not ported.
- `DOCUMENT_BROADCAST` watch events are ignored, as in JS: broadcasts travel
  on channels.
- Detach does not mark the attachment as detaching (JS `markDetaching`); the
  task queue keeps the sync loop from running during a detach instead.
- A push-only or sync-off reply is taken as a push acknowledgement only, as
  in JS, and so is a reply that arrives while a pause is pending. For the
  push-only and sync-off part, Rust takes the request's mode and the
  attachment's mode when the sync started, where JS reads the attachment's
  mode when the reply arrives. The pending-pause part reads the mode when
  the reply arrives, as JS does.
- Integration tests set schema rules and the size limit with
  `Document::set_schema_rules` and `set_max_size_per_document` after
  attaching. JS creates them through the admin API (`CreateSchema`,
  `UpdateProject`) and attaches with a schema key; those cases, and the
  `UpdateDocument` schema cases, are not ported.
- When a tree text node carries several disallowed marks, the schema error
  names the first by key; JS names the first set.
- `DeactivateOptions::keepalive` exists but has no effect: in JS it keeps a
  browser `fetch` alive during page unload, which has no Rust counterpart.
  `disableGC`, gRPC-web/Connect transport choice, Go TLS options, and Go
  receive-size/logger options are not modeled yet.
- Re-issue claims follow JS where JS and Go differ. Go records the claim
  on every attach (`c.minted[k] = actor`) and declines any repeat attach of
  the key under the same actor; JS records it only when the document
  re-issued or minted operations, at attach or at detach, so a key whose
  first document minted nothing can still re-issue. Go also attaches under
  the per-session client ID and re-issues for any client key; JS and Rust
  attach under the stable actor of the client key (`response.actor_id`)
  and re-issue only for a generated key.
- One theoretical window is left between a loop's `deactivate_internal`
  and a new session. That function runs outside the task queue: it ends
  the session and frees every key under the state lock, then marks the
  freed documents' claims after the lock is released. If `activate`
  completes a whole RPC round trip, and an attach of a freed key asks
  `claim_reissue`, while that marking loop (which never awaits) is still
  running, the attach can re-issue onto an actor that already minted in
  the key. JS marks synchronously and has no such window. Closing it would
  need marking under the state lock, which takes the document lock inside
  it (a lock-order change), or marking every freed key whether or not its
  document minted, which departs from JS.

Expected direction:

- Publish `WriteRejected` and `EpochMismatch` when those events are ported.

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

- JS `ChannelStatus.Removed` is not ported.
- Deliberate differences from JS: `detach_channel` clears the session ID (JS
  keeps it, so a re-attached channel stays `Detached` until the old session
  expires); every broadcast try checks the channel is still attached (JS
  checks once); `Client::broadcast` waits for its retries (JS resolves after
  the first failed try) and returns a non-retryable failure (JS swallows it);
  `BroadcastOptions::error` gets any final failure of a `Channel::broadcast`
  (JS calls it only when the first try fails and no retry is left, that is
  with `maxRetries: 0`: a failure after retries and a non-retryable failure
  never reach it). `attach_channel` rejects a key already attached as a
  document or channel (JS overwrites the map entry).
- JS "Should throw error when broadcasting unserializeable payload" has no
  Rust case: a `serde_json::Value` payload always serializes.
- Payload numbers are encoded by `serde_json`, which can differ from JS
  `JSON.stringify` for edge cases (for example `1.0`).

## Error Behavior

Current Rust behavior:

- `YorkieError` is small and strongly typed for the current implementation.
- `ErrorCode` has one variant per JS `Code`, with the JS strings
  (`as_str`, `FromStr`). `YorkieError::code` maps every core variant to the
  JS code thrown for the same failure, and `ClientError::code` gives the
  server's Yorkie code for `Rpc`, the JS code for lifecycle errors, and the
  core code for `Core`.

JS/Go behavior:

- JS uses Yorkie error codes and messages.
- Go has typed errors that often map to server/client operation failure modes.

Gap:

- Rust variants are finer than JS codes, so several map to one code: missing
  elements, keys, and execution times and invalid positions, indexes,
  lamports, primitives, and counter operations are all
  `ErrInvalidArgument`; `UnexpectedType`/`UnexpectedCrdtElement` are
  `ErrInvalidType`. Messages other than the schema and size ones are not
  the JS messages.
- `ClientError::code` is `None` for `Transport` errors and for server codes
  this SDK does not know; JS keeps the raw string.
- JS `ChangeApplyError` (`ErrChangeApplyFailed` with change and operation
  details) has no Rust counterpart.

Expected direction:

- Align public-facing messages with JS before stabilizing the SDK surface.
- Keep internal errors specific enough to debug CRDT state issues.

## Test Coverage

Current Rust behavior:

- Unit tests cover current CRDT primitives, object/RHT behavior, array/RGA
  basics, operations, change packs, and document JSON round trips.
- Integration tests (`crates/yorkie/tests/integration_*.rs`) run against a
  live server when `YORKIE_TEST_RPC_ADDR` is set and skip themselves
  otherwise. They cover the client lifecycle, realtime sync and presence,
  primitives, objects, arrays (including the concurrency table), counters,
  text, tree (including split and merge matrices), history, schema and size
  limits, channels with broadcast, documents, snapshots, and garbage
  collection (including push-only and containment cases). CI runs them
  against a pinned server image (`.github/workflows/ci.yml`).

JS/Go behavior:

- JS and Go have broader test suites covering local editing, sync, concurrency,
  GC, snapshots, and client behavior.

Gap:

- Rust tests are hand-picked slices, not a systematic JS test port, though
  the integration suites port the JS cases of their areas by name.
- No automated parity harness exists.
- Array/RGA and operation tests now cover the JS/Go-style insert/move/set/remove
  matrix at CRDT and root-operation layers.
- Document tests verify common public array operation intent, including
  ID-based move/insert/delete APIs and splice-like remove/insert sequences.
  The facade is recorder-backed during `Document::update`, but still not a live
  CRDT container wrapper.
- Some Text ledger ports check less than their JS source:
  `reports_nothing_tombstoned_by_a_redo_of_a_removed_piece`
  (`crdt/text_ledger_tests.rs`) asserts only that GC pairs were returned,
  not which; `purges_pieces_a_style_split_off_a_tombstoned_text_node`
  (`document/gc_tests.rs`) does not assert that gc returns to 0;
  `drops_a_malformed_ledger_and_still_applies_the_snapshot`
  (`yorkie-protocol` converter tests) checks only the rendered JSON; and the
  hand-written snapshot root of
  `replays_keeps_the_ledger_through_a_snapshot_over_proto_bytes` is guarded
  against drift only by the replica's JSON and version vector.
- The Text undo fuzz port reads `TEXT_UNDO_FUZZ_SEEDS` as a number and falls
  back to 10 seeds on anything else; JS runs no seed then.
- The live-server integration suites run only when `YORKIE_TEST_RPC_ADDR`
  is set: in CI that is the integration job (`cargo test -p yorkie`), not
  the default `cargo test --workspace` step, where each case returns early
  and reports a pass. The two history correctness cases this repo runs and
  JS skips (`integration_history.rs`, "Case 3/5 correctness") are among
  them.

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
