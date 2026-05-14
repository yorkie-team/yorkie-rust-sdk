# Tree Parity

Last reviewed: 2026-05-14

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
  `crates/yorkie-core/src/crdt/root.rs`

## Scope

Tree node model, element tree, indexes, edit operation, style operation,
attributes, removed node GC, path/index conversion, public tree facade,
concurrency, and protocol conversion.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Tree node ID and floor lookup | covered | Rust `TreeNodeId` orders by creation ticket then offset and `CrdtTree::find_floor_node` only returns nodes with the same creation ticket, matching JS/Go split-node lookup. |
| Node JSON/XML/data size | partial | Rust covers root/element/text serialization, UTF-16 text size, active attribute size, and hidden removed children. Edit-time split/merge size deltas are not implemented yet. |
| CRDT tree element | partial | `CrdtElement::Tree` delegates metadata, JSON, data size, removal, and deep copy. Tree edit/style behavior is still absent. |
| Tree edit/style operations | missing | No Rust tree operations yet. |
| Attribute RHT reuse | partial | Tree nodes now reuse `Rht`; visible attributes serialize deterministically, and removed attribute nodes become root GC pairs. |
| Tree GC | partial | Removed tree nodes and removed tree attributes are registered and purged through root GC when rebuilding from an existing root object. Operation-time registration is pending tree edit/style operations. |
| Path/index conversion | partial | Rust now ports the JS/Go index/path/position conversion rules, including element padding and text-child paths. Edit-time index tree maintenance is still pending tree operations. |
| Upstream skipped history/unit cases | blocked | JS and Go carry skipped Tree cases around history redo, overlapping undo reconciliation, L2 split undo, mixed-level merge, and generated concurrency failures. Rust must keep these skipped or ignored until upstream unskips them. See `upstream-skipped-tests.md`. |
| Public tree facade | missing | Depends on context-backed editing model. |
| Wire conversion | missing | Depends on tree operations and protocol conversion. |

## Next Checks

- Extend tree index/path coverage with removed-node and mixed element/text
  scenarios before public tree editing.
- Port tree edit/style operation tests before public facade tests.
- Keep split/merge metadata (`insPrevID`, `insNextID`, `mergedFrom`,
  `mergedAt`, `mergedInto`) aligned with JS/Go when adding edit operations.
