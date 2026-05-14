# Splay and Index Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/util/splay_tree.ts`,
  `packages/sdk/src/util/index_tree.ts`
- Go: `pkg/splay/splay.go`, `pkg/splay/splay_test.go`,
  `pkg/index/tree.go`
- Rust: `crates/yorkie-core/src/crdt/splay.rs`

## Scope

Weighted splay tree node lookup, index lookup for text and array-like values,
node deletion, range reweighting after tombstones, and later integration with
RGA text/list and Tree path/index conversion.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Weighted splay core | covered | Rust has an arena-backed splay tree with weighted nodes, insert, insert-after, splay, index lookup, delete, range reweighting, weight checks, and JS/Go-shaped tests. |
| Text cursor lookup | covered | `find_for_text` follows the JS/Go boundary behavior where cursor positions can land at node edges. |
| Array index lookup | covered | `find_for_array` skips tombstoned nodes by weight and rejects out-of-range lookups. |
| RGA list integration | partial | `RgaTreeList` keeps position/element maps and uses weighted splay lookup for visible indexes and paths. Structural mutations rebuild the index around the current `Vec` storage instead of using stable linked-node handles. |
| RGA text integration | partial | `RgaTreeSplit` keeps `tree_by_index` and `tree_by_id` equivalents and uses weighted splay lookup for text indexes. Structural mutations rebuild indexes around the current `Vec` storage instead of using stable linked-node handles. |
| Tree `IndexTree` integration | missing | Tree path/index conversion still needs the JS `IndexTree` / Go `pkg/index` model. |

## Next Checks

- Move `RgaTreeSplit` from rebuild-on-mutation indexing to stable node handles
  when aligning the write-side implementation with JS/Go.
- Move `RgaTreeList` from rebuild-on-mutation indexing to stable node handles
  when aligning the write-side implementation with JS/Go.
- Port Tree path/index conversion separately because its index model includes
  element padding and hierarchical paths, not only flat weighted lookup.
