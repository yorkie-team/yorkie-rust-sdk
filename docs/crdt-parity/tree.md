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
| Node JSON/XML/data size | partial | Rust covers root/element/text serialization, UTF-16 text size, active attribute size, hidden removed children, and text-node split size deltas during tree edit/style operations. Element split/merge size deltas are not implemented yet. |
| CRDT tree element | partial | `CrdtElement::Tree` delegates metadata, JSON, data size, removal, deep copy, split-free element insert/delete edit behavior, text-node split insert/delete edit behavior, and text-boundary split style/remove-style behavior. |
| Tree style operation | partial | Rust has `TreeStyleOperation` for element attribute set/remove, text-boundary split, reverse operation creation, op info, and removed attribute GC registration. Advancing unknown split siblings and propagating style/remove-style across split siblings are still missing. |
| Tree edit operation | partial | Rust has `TreeEditOperation` for split-free element insert/delete and text-node split insert/delete, reverse operation creation, op info, and tree-node GC registration. Element split, merge, full `insPrevID`/`insNextID` maintenance, and unknown split sibling handling are still missing. |
| Attribute RHT reuse | partial | Tree nodes now reuse `Rht`; visible attributes serialize deterministically, and removed attribute nodes become root GC pairs through tree style operations. |
| Tree GC | partial | Removed tree nodes and removed tree attributes are registered and purged through root GC when rebuilding from an existing root object. Tree style operations register removed attribute GC pairs; tree edit operations register removed tree-node GC pairs for split-free element deletion and text-node split deletion. |
| Path/index conversion | partial | Rust now ports the JS/Go index/path/position conversion rules, including element padding and text-child paths. Edit-time index tree maintenance is still pending tree operations. |
| Upstream skipped history/unit cases | blocked | JS and Go carry skipped Tree cases around history redo, overlapping undo reconciliation, L2 split undo, mixed-level merge, and generated concurrency failures. Rust must keep these skipped or ignored until upstream unskips them. See `upstream-skipped-tests.md`. |
| Public tree facade | missing | Depends on context-backed editing model. |
| Wire conversion | missing | Depends on tree operations and protocol conversion. |

## Next Checks

- Extend tree index/path coverage with removed-node and mixed element/text
  scenarios before public tree editing.
- Extend tree style operation tests from text-boundary split toward
  version-vector-aware styling, unknown split siblings, and split sibling
  propagation.
- Extend tree edit operation tests from text-node split insert/delete toward
  element split, merge, and concurrent split sibling cases before public facade
  tests.
- Keep split/merge metadata (`insPrevID`, `insNextID`, `mergedFrom`,
  `mergedAt`, `mergedInto`) aligned with JS/Go when adding edit operations.
