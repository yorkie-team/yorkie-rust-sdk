# Array and RGATreeList Parity

Last reviewed: 2026-05-19

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
  `crates/yorkie-core/src/operation/remove_operation.rs`,
  `crates/yorkie-core/src/wire.rs`,
  `crates/yorkie-protocol/src/converter.rs`

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
| Public `JsonArray` facade | partial | Plain value APIs cover index get/set/insert/remove, nested object/array access, read-only ID/value element lookup, value and ID search, ID-based insert/delete/move, index-based insert, and splice-like remove/insert sequences. During `Document::update`, these APIs record Add, ArraySet, Remove, and Move operations at the mutation site, including same-update parent creation, same-visible-value set calls, and nested edits after splice insertion. Mutable JS-style wrapped element proxies and live CRDT container mutation are still missing. |
| Splay/index optimization | partial | `RgaTreeList` now keeps JS/Go-shaped position and element maps and uses weighted splay lookup for visible indexes and paths. Structural mutations still rebuild the indexes around the Rust `Vec` backing store instead of maintaining linked node handles incrementally. |
| Snapshot restoration | partial | Root rebuild tests cover moved positions, dead positions, path lookup, and GC after copy. Protocol `JSONElement.Array` conversion now preserves live nodes, moved position nodes, and dead position nodes, and decoded snapshot roots can replace a `Document` root. Array-specific snapshot fixtures for moved/dead positions through the protocol path are still missing. |
| Wire conversion | partial | Add/move/remove/array-set operations and full array `JSONElement` payloads convert to/from protobuf-shaped wire values. Rust ports Go's standalone array bytes scenario and the array portion of JS's root bytes scenario at core wire level. External binary fixtures are still missing. |

## Next Checks

- Keep porting JS public array scenarios around mutable wrapped element
  metadata and any read-only behavior that cannot be expressed through Rust
  iterators or slices.
- Replace the rebuild-on-mutation indexing strategy with stable node handles to
  match the JS/Go write-side implementation more closely.
- Add protocol-level replay fixtures around duplicate position IDs, moved
  positions, dead positions, and GC after the JS/Go in-repo converter cases are
  ported more fully.
