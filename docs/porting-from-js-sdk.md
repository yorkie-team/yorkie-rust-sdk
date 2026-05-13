# Porting from the JS SDK

The Rust SDK is a port of the Yorkie JS SDK. The JS SDK is the source of truth
for SDK behavior, terminology, state transitions, and test expectations.

## Source of Truth

Use the following order when deciding how a concept should work:

1. JS SDK tests for observable behavior.
2. JS SDK implementation for field names, state flow, and event semantics.
3. Yorkie protobuf files for wire format.
4. Go server and Go client code for algorithmic background only.

In the local development workspace, the main references are:

- JS SDK public facade: `../yorkie-js-sdk/packages/sdk/src/yorkie.ts`
- JS SDK source: `../yorkie-js-sdk/packages/sdk/src/`
- JS SDK tests: `../yorkie-js-sdk/packages/sdk/test/`
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
- Public Rust APIs may be idiomatic Rust, but their semantics must match the JS
  SDK. For example, a Rust method may be named `to_sorted_json`, but it maps to
  JS `toSortedJSON()`.
- Every ported feature must start from a JS SDK test or a JS SDK source section.
  If no JS test exists, add the Rust test from the JS source behavior and record
  the source path in the test or nearby module docs.

## Porting Workflow

1. Pick one JS SDK test file or one small behavior inside a test file.
2. Add a Rust test that states the same observable behavior.
3. Keep source references in docs and planning notes, not in public code
   comments unless they are necessary for maintainability.
4. Implement the smallest Rust code needed to pass the test.
5. Keep the Rust names mapped to JS names. Add a mapping note when the Rust name
   differs for idiomatic reasons.
6. Run `cargo fmt` and `cargo test`.

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
| `CrdtElementMeta` | `packages/sdk/src/document/crdt/element.ts` | Rust metadata holder for CRDT element timestamps. |
| `CrdtElement` | `packages/sdk/src/document/crdt/element.ts` | Internal enum that exposes common element behavior across CRDT value types. |
| `CrdtPrimitive` | `packages/sdk/src/document/crdt/primitive.ts` | Internal CRDT element for leaf primitive values. |
| `ElementRht` | `packages/sdk/src/document/crdt/element_rht.ts` | Internal replicated hash table for object members, keyed by visible key and element creation time. |
| `CrdtObject` | `packages/sdk/src/document/crdt/object.ts` | Internal CRDT container for object members stored in `ElementRht`. |
| `JsonObject` | `packages/sdk/src/document/json/object.ts` | Public JSON-like object API must map to JS object behavior. |
| `JsonObject::set` | `ObjectProxy.setInternal()` | Reject object keys containing `.` and store the new member value. |
| `JsonObject::remove` | `ObjectProxy.deleteInternal()` | Rust method for deleting an object member. Missing keys must be a no-op. |
| `JsonArray` | `packages/sdk/src/document/json/array.ts` | Public array API must map to JS array behavior. |
| `JsonValue` | JS JSON element/proxy values | Temporary Rust wrapper for porting primitives, objects, and arrays. |
| `Client` | `packages/sdk/src/client/client.ts` | Currently scaffolded only. Lifecycle must follow JS. |
| `yorkie-protocol` | `packages/sdk/src/api/converter.ts`, proto files | Converter names and wire fields should track JS/proto names. |

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

- Which JS SDK file is the source of truth?
- Which JS SDK test or behavior is being ported?
- Does the Rust name map clearly to the JS name?
- Is this a public API or an internal implementation detail?
- If this is a new internal abstraction, which JS concept does it support?
- Does it preserve the dependency direction in
  [Dependency Boundaries](dependency-boundaries.md)?
- Did `cargo fmt` and `cargo test` pass?
