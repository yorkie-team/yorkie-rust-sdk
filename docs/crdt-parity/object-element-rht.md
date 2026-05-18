# Object and ElementRHT Parity

Last reviewed: 2026-05-19

## References

- JS: `packages/sdk/src/document/crdt/object.ts`,
  `packages/sdk/src/document/crdt/element_rht.ts`
- JS tests: `packages/sdk/test/integration/object_test.ts`,
  `packages/sdk/test/unit/document/crdt/element_rht_test.ts`
- Go: `pkg/document/crdt/object.go`, `pkg/document/crdt/element_rht.go`,
  `pkg/document/crdt/object_test.go`,
  `pkg/document/crdt/element_rht_test.go`
- Rust: `crates/yorkie-core/src/crdt/object.rs`,
  `crates/yorkie-core/src/crdt/element_rht.rs`,
  `crates/yorkie-core/src/wire.rs`,
  `crates/yorkie-protocol/src/converter.rs`

## Scope

Object member storage, LWW set behavior, tombstones, key lookup, created-time
lookup, deletion, purge, descendant traversal, path lookup, sorted JSON output,
and root index integration.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| ElementRHT set/delete basics | covered | Current Rust tests cover visible key and created-time lookup. |
| LWW conflict handling | covered | Tests cover late concurrent sets and multiple late sets. |
| Tombstone and purge behavior | covered | Tests cover delete and purge for current ElementRHT behavior. |
| Object nested members | partial | Nested object behavior is tested, but broad JS object scenarios remain. |
| Root integration | partial | Root tests cover nested descendants and paths for objects. |
| Public object facade | partial | Public object changes still use the temporary diff bridge. |
| Wire conversion | partial | Full object `JSONElement` payloads and object-valued `JSONElementSimple` bytes now convert to/from wire values. Rust ports the root object bytes shape for nested object members and the JS/Go object-GC bytes scenario at core wire level. External binary fixtures are still missing. |

## Next Checks

- Port more JS object integration cases after the context-backed public facade
  exists.
- Add operation-level object set/remove replay cases with nested parents.
- Add protocol fixture coverage for object tombstones and same-key LWW members.
- Revisit object GC when tree descendants are added.
