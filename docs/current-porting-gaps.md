# Current Porting Gaps

Last reviewed: 2026-05-14

This document records known differences between the current Rust SDK and the
JS/Go implementations. It is intentionally practical: each section explains
what is different today, why it matters, and what should be aligned later.

The JS SDK remains the behavioral source of truth. The Go implementation should
be used as a typed cross-check, especially for CRDT internals, operation
application, and ownership-like structure.

## Reference Areas

- JS document facade and update flow:
  `packages/sdk/src/document/document.ts`
- JS JSON object and array proxies:
  `packages/sdk/src/document/json/object.ts`,
  `packages/sdk/src/document/json/array.ts`
- JS CRDT containers:
  `packages/sdk/src/document/crdt/object.ts`,
  `packages/sdk/src/document/crdt/array.ts`,
  `packages/sdk/src/document/crdt/rga_tree_list.ts`
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
  are refreshed manually.
- Parent pointers in the index are also stored as copied elements.

JS/Go behavior:

- Root indexes point to actual CRDT elements or typed object references.
- Parent/child relationships are reference-like and naturally stay connected to
  the live tree.

Gap:

- Rust's copied index can become stale if a future mutation path forgets to
  refresh the affected element or its ancestors.
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
- The backing structure is a `Vec` with linear scans.
- Dead position nodes from array moves are registered in the root GC pair map
  and included in garbage length/stat counters.

JS/Go behavior:

- `RGATreeList` uses linked nodes plus a splay tree for index lookup.
- It maintains maps for position-node IDs and element IDs.
- Dead position nodes are GC children.

Gap:

- Rust does not yet have the splay tree index. Index lookup and path creation
  are O(n).
- Rust does not yet keep explicit maps for position IDs and element IDs. Linear
  lookup preserves simple behavior, but duplicate replay/idempotency cases are
  not fully covered.
- Root tracks dead RGA position GC pairs, but physical garbage collection and
  purge of those internal nodes is not implemented yet.
- Concurrent move/insert/set combinations need more parity tests, especially
  inserts after dead positions and late moves that still create dead positions.
- Snapshot restoration behavior for moved elements and dead positions is not
  implemented, so `addDeadPosition`/`addMovedElement` parity is only modeled
  internally.

Expected direction:

- Add focused tests from JS/Go array and RGA behavior before optimizing the
  data structure.
- Implement physical garbage collection for dead RGA position nodes.
- Add explicit position and element indexes when replay/idempotency tests need
  them, or when performance becomes a real concern.

## Array Operations

Current Rust behavior:

- Internal `AddOperation`, `MoveOperation`, and `ArraySetOperation` exist.
- `RemoveOperation` can remove either an object member or an array element.
- Tests cover basic add, move, array set, and array remove behavior.
- `MoveOperation` registers dead position nodes as root GC pairs.

JS/Go behavior:

- These operations are created by public array mutation APIs.
- Move operations register dead position nodes for GC.
- Operation execution receives the broader context needed for version-vector
  visibility and sync behavior.

Gap:

- Rust array operations are not yet created by public `JsonArray` methods.
- Rust operation execution does not yet receive or use version vectors.
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

## Object and ElementRHT

Current Rust behavior:

- `CrdtObject` stores members in `ElementRht`.
- `ElementRht` handles basic LWW set behavior, tombstones overwritten values,
  and keeps lookup by visible key and creation time.
- Object descendants can now include arrays.

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
