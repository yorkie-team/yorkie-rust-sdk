# Array and RGATreeList Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/array.ts`,
  `packages/sdk/src/document/crdt/rga_tree_list.ts`,
  `packages/sdk/src/document/json/array.ts`
- JS tests: `packages/sdk/test/integration/array_test.ts`,
  `packages/sdk/test/unit/document/document_test.ts`
- Go: `pkg/document/crdt/array.go`,
  `pkg/document/crdt/rga_tree_list.go`,
  `pkg/document/crdt/array_test.go`,
  `pkg/document/crdt/rga_tree_list_test.go`
- Rust: `crates/yorkie-core/src/crdt/array.rs`,
  `crates/yorkie-core/src/crdt/rga_tree_list.rs`,
  `crates/yorkie-core/src/operation/add_operation.rs`,
  `crates/yorkie-core/src/operation/move_operation.rs`,
  `crates/yorkie-core/src/operation/array_set_operation.rs`,
  `crates/yorkie-core/src/operation/remove_operation.rs`

## Scope

Array element storage, position-node identity, insert ordering, delete, set,
move, LWW position register, dead move positions, GC, path lookup, operation
application, public array mutation APIs, and sync-level convergence.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Basic array serialization | covered | `CrdtArray` tests cover visible element output. |
| Nested descendant lookup | covered | `CrdtArray` tests cover nested object descendants. |
| RGA insert/delete basics | covered | `RgaTreeList` tests cover inserts, deletes, and paths. |
| RGA move semantics | covered | Tests cover position nodes, LWW losing moves, and dead positions. |
| RGA convergence matrix | covered | Rust covers same/different moves, chained moves, independent moves, and insert/move/set/remove combinations at RGA level. |
| Dead position GC | covered | Root can register and physically purge dead RGA position nodes. |
| Operation basics | covered | Add, move, set, remove have focused operation tests. |
| Operation-level matrix | covered | Add/move/array-set/remove pairs are applied through `CrdtRoot` in both orders and checked for JSON, path, root stats, and GC convergence. |
| Public `JsonArray` facade | blocked | Current public array is not context-backed, so operation intent is not preserved. |
| Splay/index optimization | missing | Rust uses linear `Vec` scans. |
| Snapshot restoration | partial | Internal `add_dead_position` and `add_moved_element` exist but snapshot tests are missing. |
| Wire conversion | missing | No operation/protocol conversion yet. |

## Next Checks

- Port Go array position-confusion cases at root/operation level:
  move-front/move-last followed by push or insert.
- Build the context-backed public `JsonArray` facade before porting JS public
  array tests.
- Add explicit position and element indexes before attempting splay
  optimization.
