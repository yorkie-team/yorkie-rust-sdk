# Tree Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/tree.ts`,
  `packages/sdk/src/document/json/tree.ts`
- JS tests: `packages/sdk/test/integration/tree_test.ts`,
  `packages/sdk/test/integration/tree_concurrency_test.ts`,
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
| Path/index conversion | missing | Rust does not yet have the tree index structure used by JS `IndexTree` and Go `pkg/index`. |
| Public tree facade | missing | Depends on context-backed editing model. |
| Wire conversion | missing | Depends on tree operations and protocol conversion. |

## Next Checks

- Port tree index/path conversion before public tree editing.
- Port tree edit/style operation tests before public facade tests.
- Keep split/merge metadata (`insPrevID`, `insNextID`, `mergedFrom`,
  `mergedAt`, `mergedInto`) aligned with JS/Go when adding edit operations.
