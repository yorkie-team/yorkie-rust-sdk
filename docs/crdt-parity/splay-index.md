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
| RGA integration | missing | `RgaTreeList` and `RgaTreeSplit` still use linear vectors. |
| Tree `IndexTree` integration | missing | Tree path/index conversion still needs the JS `IndexTree` / Go `pkg/index` model. |

## Next Checks

- Add explicit node handles to `RgaTreeSplit` before replacing linear index
  lookup with `SplayTree`.
- Add explicit position and element ID indexes to `RgaTreeList`, then attach
  visible array index lookup to `SplayTree`.
- Port Tree path/index conversion separately because its index model includes
  element padding and hierarchical paths, not only flat weighted lookup.
