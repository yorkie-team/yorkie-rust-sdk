# Porting from the JS SDK

The Rust SDK is a port of the Yorkie JS SDK. The JS SDK is the source of truth
for SDK behavior, terminology, state transitions, and test expectations.

## Source of Truth

Use the following order when deciding how a concept should work:

1. JS SDK tests for observable behavior.
2. JS SDK implementation for field names, state flow, and event semantics.
3. Go SDK/client implementation for cross-checking typed SDK flow, CRDT
   internals, and edge cases.
4. Yorkie protobuf files for wire format.
5. Go server internals for algorithmic background only.

In the local development workspace, the main references are:

- JS SDK public facade: `../yorkie-js-sdk/packages/sdk/src/yorkie.ts`
- JS SDK source: `../yorkie-js-sdk/packages/sdk/src/`
- JS SDK tests: `../yorkie-js-sdk/packages/sdk/test/`
- Go SDK/client source: `../yorkie/client/`
- Go SDK document source: `../yorkie/pkg/document/`
- Go SDK document tests: `../yorkie/pkg/document/**/*_test.go`
- Yorkie proto files: `../yorkie/api/yorkie/v1/`
- Yorkie design docs: `../yorkie/docs/design/`

## Non-Negotiable Porting Rules

- Do not invent Rust-only domain concepts unless the concept is explicitly
  documented as an implementation detail and mapped to the JS SDK concept it
  supports.
- Do not rename domain fields casually. If Rust uses idiomatic snake_case, the
  corresponding JS name must still be obvious in docs, comments, tests, or type
  names.
- Do not change the lifecycle flow. `Client`, `Document`, `Change`,
  `ChangePack`, `Operation`, `TimeTicket`, `VersionVector`, presence, sync
  modes, watch streams, and document statuses must follow the JS SDK model.
- Do not use the Go client as the behavioral source of truth when it differs
  from the JS SDK. Go is useful for CRDT internals and server-side context, but
  JS defines the SDK behavior we are porting.
- Cross-check the Go SDK/client when the JS implementation relies on
  JavaScript-specific runtime behavior, proxies, reference sharing, or dynamic
  typing. Use Go to clarify how the same concept works in a typed SDK, then
  keep the Rust behavior aligned with JS.
- Public Rust APIs may be idiomatic Rust, but their semantics must match the JS
  SDK. For example, a Rust method may be named `to_sorted_json`, but it maps to
  JS `toSortedJSON()`.
- Every ported feature must start from a JS SDK test or a JS SDK source section.
  If no JS test exists, add the Rust test from the JS source behavior and record
  the source path in the test or nearby module docs.
- Do not treat skipped JS/Go CRDT tests as pass targets. If they are represented
  in Rust, keep them ignored and reference
  `docs/crdt-parity/upstream-skipped-tests.md`.

## Porting Workflow

1. Pick one JS SDK test file or one small behavior inside a test file.
2. Add a Rust test that states the same observable behavior.
3. If the source test is skipped upstream, document it and keep the Rust test
   ignored instead of implementing ahead of upstream behavior.
4. Keep source references in docs and planning notes, not in public code
   comments unless they are necessary for maintainability.
5. Cross-check the corresponding Go SDK/client implementation when the feature
   touches CRDT internals, operation application, typed API flow, ownership-like
   structure, or error handling.
6. Implement the smallest Rust code needed to pass the test.
7. Keep the Rust names mapped to JS names. Add a mapping note when the Rust name
   differs for idiomatic reasons.
8. Run `cargo fmt` and `cargo test`.

Prefer vertical slices over broad scaffolding. A small feature is done only when
it has:

- a JS SDK source/test reference,
- a Rust test,
- matching observable behavior,
- no undocumented Rust-only concepts.

## Current Concept Map

| Rust concept | JS SDK source of truth | Notes |
| --- | --- | --- |
| `yorkie` facade crate | `packages/sdk/src/yorkie.ts` | Public entrypoint for SDK users. |
| `Document` | `packages/sdk/src/document/document.ts` | Rust `Document::new` maps to JS `new Document(key)`. |
| `Document::to_sorted_json` | `Document.toSortedJSON()` | Rust naming is idiomatic; behavior must match JS. |
| `ActorId` | `packages/sdk/src/document/time/actor_id.ts` | Rust newtype for the actor identifier string. |
| `TimeTicket` | `packages/sdk/src/document/time/ticket.ts` | Logical timestamp ordered by lamport, actor, then delimiter. |
| `VersionVector` | `packages/sdk/src/document/time/version_vector.ts` | Actor-to-lamport vector used for causal visibility checks. |
| `ChangeId` | `packages/sdk/src/document/change/change_id.ts` | Change identifier with client sequence, server sequence, actor, lamport, and version vector. |
| `Checkpoint` | `packages/sdk/src/document/change/checkpoint.ts` | Tracks sent and received client/server sequences for sync. |
| `ChangeContext` | `packages/sdk/src/document/change/context.ts` | Records operations during a local edit and issues time tickets from the next change ID. |
| `Change` | `packages/sdk/src/document/change/change.ts` | Unit of document modification that executes operations and collects operation info and reverse operations. |
| `ChangePack` | `packages/sdk/src/document/change/change_pack.ts` | Bundle of document key, checkpoint, changes, version vector, removal flag, and optional snapshot for sync. |
| `yorkie_core::wire` | `packages/sdk/src/api/converter.ts`, proto files | Narrow projection from internal core changes and operations to protocol-facing values without exposing CRDT internals through the top-level SDK facade. |
| `CrdtElementMeta` | `packages/sdk/src/document/crdt/element.ts` | Rust metadata holder for CRDT element timestamps. |
| `CrdtElement` | `packages/sdk/src/document/crdt/element.ts` | Internal enum that exposes common element behavior across primitive, object, array, text, counter, and tree values. |
| `CrdtPrimitive` | `packages/sdk/src/document/crdt/primitive.ts` | Internal CRDT element for leaf primitive values. |
| `Hll` | `packages/sdk/src/document/crdt/hll.ts` | Internal HyperLogLog register set for dedup counters, using precision 14 and xxhash64 seed 0. |
| `CrdtCounter` | `packages/sdk/src/document/crdt/counter.ts` | Internal CRDT counter element for integer, long, and integer-dedup counters. |
| `ElementRht` | `packages/sdk/src/document/crdt/element_rht.ts` | Internal replicated hash table for object members, keyed by visible key and element creation time. |
| `CrdtObject` | `packages/sdk/src/document/crdt/object.ts` | Internal CRDT container for object members stored in `ElementRht`. |
| `RhtNode` | `packages/sdk/src/document/crdt/rht.ts` | Internal node for string attributes used by text and tree values. |
| `Rht` | `packages/sdk/src/document/crdt/rht.ts` | Internal replicated hash table for string attributes. Cross-check with Go `pkg/document/crdt/rht.go`. |
| `RgaTreeSplitNodeId` | `packages/sdk/src/document/crdt/rga_tree_split.ts` | Internal block identifier for text nodes, ordered by creation ticket and UTF-16 offset. |
| `RgaTreeSplit` | `packages/sdk/src/document/crdt/rga_tree_split.ts` | Internal block-based RGA for text. Rust currently keeps the same node semantics with a linear backing structure. |
| `TextValue` | `packages/sdk/src/document/crdt/text.ts` | Internal text block content plus `Rht` attributes. |
| `CrdtText` | `packages/sdk/src/document/crdt/text.ts` | Internal CRDT text element backed by `RgaTreeSplit<TextValue>` and indexed by `CrdtRoot` as a CRDT element. |
| `TreeNodeId` | `packages/sdk/src/document/crdt/tree.ts` | Internal tree node identifier ordered by creation ticket and UTF-16 offset. |
| `TreeNode` | `packages/sdk/src/document/crdt/tree.ts` | Internal tree node for element/text content, attributes, tombstones, split links, and merge metadata. |
| `CrdtTree` | `packages/sdk/src/document/crdt/tree.ts` | Internal CRDT tree element with node lookup, serialization, data-size accounting, deep copy, GC-pair discovery, split-free element insert/delete edit behavior, text-node split insert/delete edit behavior, multi-level element split behavior, visible-boundary merge behavior, and text-boundary split style/remove-style behavior. |
| `RgaTreeList` | `packages/sdk/src/document/crdt/rga_tree_list.ts` | Internal replicated growable array list. Rust currently keeps the same node semantics with a linear backing structure. |
| `CrdtArray` | `packages/sdk/src/document/crdt/array.ts` | Internal CRDT container for array elements stored in `RgaTreeList`. |
| `CrdtRoot` | `packages/sdk/src/document/crdt/root.ts` | Internal root that indexes CRDT elements by creation time and creates element paths. |
| `SetOperation` | `packages/sdk/src/document/operation/set_operation.ts` | Operation that sets an object member and updates the root index and GC candidates. |
| `RemoveOperation` | `packages/sdk/src/document/operation/remove_operation.ts` | Operation that removes an object member or array element by creation time and updates GC candidates. |
| `AddOperation` | `packages/sdk/src/document/operation/add_operation.ts` | Operation that inserts a CRDT element into an array after a position identity. |
| `MoveOperation` | `packages/sdk/src/document/operation/move_operation.ts` | Operation that moves an existing array element using position-node identity. |
| `ArraySetOperation` | `packages/sdk/src/document/operation/array_set_operation.ts` | Operation that replaces an array element by inserting a new element and tombstoning the previous one. |
| `IncreaseOperation` | `packages/sdk/src/document/operation/increase_operation.ts` | Operation that increases regular counters with numeric operands and dedup counters with actor-scoped unit increments. |
| `EditOperation` | `packages/sdk/src/document/operation/edit_operation.ts` | Operation that edits a text range, registers removed text-node GC pairs, and creates a reverse edit. |
| `StyleOperation` | `packages/sdk/src/document/operation/style_operation.ts` | Operation that applies or removes text attributes, registers removed attribute GC pairs, and creates a reverse style operation. |
| `TreeEditOperation` | `packages/sdk/src/document/operation/tree_edit_operation.ts` | Operation that edits tree content. Rust currently covers split-free element insert/delete, text-node split insert/delete, multi-level element split, visible-boundary merge, operation info, insert/delete/split/merge reverse operations, and tree-node GC registration. |
| `TreeStyleOperation` | `packages/sdk/src/document/operation/tree_style_operation.ts` | Operation that applies or removes tree element attributes, splits text at style boundaries, registers removed attribute GC pairs, and creates a reverse tree style operation. |
| `JsonObject` | `packages/sdk/src/document/json/object.ts` | Public JSON-like object API must map to JS object behavior. |
| `JsonObject::set` | `ObjectProxy.setInternal()` | Reject object keys containing `.` and store the new member value. |
| `JsonObject::remove` | `ObjectProxy.deleteInternal()` | Rust method for deleting an object member. Missing keys must be a no-op. |
| `JsonArray` | `packages/sdk/src/document/json/array.ts` | Public array API maps index, ID-based, and splice-like array edits to add, array-set, remove, and move operations during `Document::update`. |
| `JsonArrayElement` | JS `WrappedElement` read-only lookup shape | Lightweight Rust lookup value that exposes an array element ID and borrowed JSON value. It is not a mutable proxy. |
| `JsonCounter` | `packages/sdk/src/document/json/counter.ts` | Public counter facade for regular increase operations and dedup actor-add operations during `Document::update`. |
| `JsonValue` | JS JSON element/proxy values | Temporary Rust wrapper for porting primitives, counters, objects, and arrays. |
| `Client` | `packages/sdk/src/client/client.ts` | Currently scaffolded only. Lifecycle must follow JS. |
| `yorkie-protocol` | `packages/sdk/src/api/converter.ts`, proto files | Proto-shaped resource types and field-level converters should track JS/proto names before generated protobuf binary encoding is added. |

Update this table whenever a new Rust type becomes part of the porting surface.

## First Porting Targets

Start with the JS SDK behaviors that define the document surface before network
sync:

1. Document key handling.
2. Primitive set on the root object.
3. Object set and nested object updates.
4. Array set and array push.
5. Object key removal.
6. Local change and operation capture.

Only after local change capture is aligned should the Rust SDK move to
`ChangePack`, protobuf conversion, and `Client.attach/sync`.

## Agent Checklist

Before adding or changing a Rust SDK concept:

- Did you check [Current Porting Gaps](current-porting-gaps.md) for known
  differences in the area you are touching?
- Which JS SDK file is the source of truth?
- Which JS SDK test or behavior is being ported?
- Which Go SDK/client file was cross-checked, if the feature has a comparable
  implementation?
- Does the Rust name map clearly to the JS name?
- Is this a public API or an internal implementation detail?
- If this is a new internal abstraction, which JS concept does it support?
- Does it preserve the dependency direction in
  [Dependency Boundaries](dependency-boundaries.md)?
- Did `cargo fmt` and `cargo test` pass?
