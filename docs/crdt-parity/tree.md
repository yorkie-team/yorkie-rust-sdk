# Tree Parity

Last reviewed: 2026-05-15

## References

- JS: `packages/sdk/src/document/crdt/tree.ts`,
  `packages/sdk/src/document/json/tree.ts`
- JS tests: `packages/sdk/test/integration/tree_test.ts`,
  `packages/sdk/test/integration/tree_concurrency_test.ts`,
  `packages/sdk/test/integration/history_tree_test.ts`,
  `packages/sdk/test/integration/history_tree_split_test.ts`,
  `packages/sdk/test/unit/document/crdt/tree_test.ts`
- Go: `pkg/document/crdt/tree.go`,
  `pkg/document/crdt/tree_test.go`,
  `pkg/document/operations/tree_edit.go`,
  `pkg/document/operations/tree_style.go`
- Rust: `crates/yorkie-core/src/crdt/tree.rs`,
  `crates/yorkie-core/src/crdt/element.rs`,
  `crates/yorkie-core/src/crdt/root.rs`,
  `crates/yorkie-core/src/operation/tree_edit_operation.rs`,
  `crates/yorkie-core/src/operation/tree_style_operation.rs`

## Scope

Tree node model, element tree, indexes, edit operation, style operation,
attributes, removed node GC, path/index conversion, public tree facade,
concurrency, and protocol conversion.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Tree node ID and floor lookup | covered | Rust `TreeNodeId` orders by creation ticket then offset and `CrdtTree::find_floor_node` only returns nodes with the same creation ticket, matching JS/Go split-node lookup. |
| Node JSON/XML/data size | partial | Rust covers root/element/text serialization, UTF-16 text size, active attribute size, hidden removed children, text-node split size deltas during tree edit/style operations, and element split size deltas across split levels. Merge-specific size checks still need broader parity tests. |
| CRDT tree element | partial | `CrdtElement::Tree` delegates metadata, JSON, data size, removal, deep copy, split-free element insert/delete edit behavior, text-node split insert/delete edit behavior, text-boundary split style/remove-style behavior, multi-level element split behavior, and visible-boundary merge behavior. |
| Tree style operation | partial | Rust has `TreeStyleOperation` for element attribute set/remove, text-boundary split, reverse operation creation, op info, removed attribute GC registration, and initial unknown split sibling style propagation. Full concurrent split/style matrices still need parity tests. |
| Tree edit operation | partial | Rust has `TreeEditOperation` for split-free element insert/delete, text-node split insert/delete, multi-level element split, visible-boundary merge, parent/left position resolution, unknown split sibling advancement, range narrowing, representative concurrent split/insert/delete cases, reverse operation creation for insert/delete/split/merge, op info, and tree-node GC registration. Redo tagging and the remaining concurrent edit matrix still need coverage. |
| Attribute RHT reuse | partial | Tree nodes now reuse `Rht`; visible attributes serialize deterministically, and removed attribute nodes become root GC pairs through tree style operations. |
| Tree GC | partial | Removed tree nodes and removed tree attributes are registered and purged through root GC when rebuilding from an existing root object. Tree style operations register removed attribute GC pairs; tree edit operations register removed tree-node GC pairs for split-free element deletion and text-node split deletion. |
| Path/index conversion | partial | Rust now ports the JS/Go index/path/position conversion rules, including element padding, text-child paths, floor lookup for split text positions, parent/left path resolution, and token traversal for edit/style collection. It still recomputes over the current tree instead of maintaining the same stable index-tree structure as JS/Go. |
| Upstream skipped history/unit cases | blocked | JS and Go carry skipped Tree cases around history redo, overlapping undo reconciliation, L2 split undo, mixed-level merge, and generated concurrency failures. Rust must keep these skipped or ignored until upstream unskips them. See `upstream-skipped-tests.md`. |
| Public tree facade | missing | Depends on context-backed editing model. |
| Wire conversion | missing | Depends on tree operations and protocol conversion. |

## Next Checks

- Extend tree index/path coverage with removed-node and mixed element/text
  scenarios before public tree editing.
- Extend tree style operation tests from the current direct split-sibling
  propagation toward version-vector-aware concurrent style matrices.
- Extend tree edit operation tests from the current multi-level split, merge,
  representative concurrent split/insert/delete, and direct split/merge reverse
  scenarios toward redo tagging and the broader concurrent edit matrix.
- Keep split/merge metadata (`insPrevID`, `insNextID`, `mergedFrom`,
  `mergedAt`, `mergedInto`) aligned with JS/Go when adding edit operations.
