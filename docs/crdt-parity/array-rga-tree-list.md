# Array and RGATreeList Parity

Last reviewed: 2026-05-15

## References

- JS: `packages/sdk/src/document/crdt/array.ts`,
  `packages/sdk/src/document/crdt/rga_tree_list.ts`,
  `packages/sdk/src/document/json/array.ts`
- JS tests: `packages/sdk/test/integration/array_test.ts`,
  `packages/sdk/test/integration/history_array_test.ts`,
  `packages/sdk/test/unit/document/document_test.ts`
- Go: `pkg/document/crdt/array.go`,
  `pkg/document/crdt/rga_tree_list.go`,
  `pkg/document/crdt/array_test.go`,
  `pkg/document/crdt/rga_tree_list_test.go`
- Rust: `crates/yorkie-core/src/crdt/array.rs`,
  `crates/yorkie-core/src/crdt/rga_tree_list.rs`,
  `crates/yorkie-core/src/crdt/splay.rs`,
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
| Operation position anchors | covered | Rust ports Go position-confusion regressions for move-front/move-last followed by push or insert. |
| Upstream skipped history cases | blocked | JS skips array history undo matrix cases where a move is followed by a set. Rust must keep these skipped or ignored until upstream unskips them. See `upstream-skipped-tests.md`. |
| Public `JsonArray` facade | partial | Plain value APIs now cover index get/set/insert/remove and nested object/array access. `Document::update` can infer Add, Remove, ArraySet, and nested object/array changes for existing arrays, but it is still not a context-backed editing facade and cannot preserve all user intent. |
| Splay/index optimization | partial | `RgaTreeList` now keeps JS/Go-shaped position and element maps and uses weighted splay lookup for visible indexes and paths. Structural mutations still rebuild the indexes around the Rust `Vec` backing store instead of maintaining linked node handles incrementally. |
| Snapshot restoration | partial | Root rebuild tests cover moved positions, dead positions, path lookup, and GC after copy; protocol snapshot conversion is still missing. |
| Wire conversion | missing | No operation/protocol conversion yet. |

## Next Checks

- Build the context-backed public `JsonArray` facade before porting JS public
  ID-based move/insert tests.
- Replace the rebuild-on-mutation indexing strategy with stable node handles to
  match the JS/Go write-side implementation more closely.
- Keep porting JS/Go array replay and snapshot restoration scenarios around
  duplicate position IDs, moved positions, and dead position GC.
