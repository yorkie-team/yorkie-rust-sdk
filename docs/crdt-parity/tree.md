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
- Rust: not implemented yet

## Scope

Tree node model, element tree, indexes, edit operation, style operation,
attributes, removed node GC, path/index conversion, public tree facade,
concurrency, and protocol conversion.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| CRDT tree element | missing | No Rust tree element variant yet. |
| Tree edit/style operations | missing | No Rust tree operations yet. |
| Attribute RHT reuse | blocked | `Rht` exists and should be reused once tree nodes are ported. |
| Tree GC | missing | Depends on tree node implementation. |
| Public tree facade | missing | Depends on context-backed editing model. |
| Wire conversion | missing | Depends on tree operations and protocol conversion. |

## Next Checks

- Port the core tree node model after Array/Text operation behavior is stable.
- Reuse `Rht` for attributes and add GC pair tests early.
- Port tree edit/style operation tests before public facade tests.
