# Object and ElementRHT Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/object.ts`,
  `packages/sdk/src/document/crdt/element_rht.ts`
- JS tests: `packages/sdk/test/integration/object_test.ts`,
  `packages/sdk/test/unit/document/crdt/element_rht_test.ts`
- Go: `pkg/document/crdt/object.go`, `pkg/document/crdt/element_rht.go`,
  `pkg/document/crdt/object_test.go`,
  `pkg/document/crdt/element_rht_test.go`
- Rust: `crates/yorkie-core/src/crdt/object.rs`,
  `crates/yorkie-core/src/crdt/element_rht.rs`

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
| Wire conversion | missing | No object/member protocol conversion yet. |

## Next Checks

- Port more JS object integration cases after the context-backed public facade
  exists.
- Add operation-level object set/remove replay cases with nested parents.
- Revisit object GC when tree/counter descendants are added.
