# Tree Parity

Last reviewed: 2026-11-09

## References

- JS: `packages/sdk/src/document/crdt/tree.ts`,
  `packages/sdk/src/document/json/tree.ts`
- JS tests: `packages/sdk/test/integration/tree_test.ts`,
  `packages/sdk/test/integration/tree_concurrency_test.ts`,
  `packages/sdk/test/integration/tree_split_opinfo_test.ts`,
  `packages/sdk/test/integration/tree_concurrent_split_merge_test.ts`,
  `packages/sdk/test/integration/tree_merge_anchor_test.ts`,
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
  `crates/yorkie-core/src/operation/tree_style_operation.rs`,
  `crates/yorkie-core/src/wire.rs`,
  `crates/yorkie-protocol/src/converter.rs`

## Scope

Tree node model, element tree, indexes, edit operation, style operation,
attributes, removed node GC, path/index conversion, public tree facade,
concurrency, and protocol conversion.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Tree node ID and floor lookup | covered | Rust `TreeNodeId` orders by creation ticket then offset and `CrdtTree::find_floor_node` only returns nodes with the same creation ticket, matching JS/Go split-node lookup. |
| Node JSON/XML/data size | partial | Rust covers root/element/text serialization, UTF-16 text size, active attribute size, hidden removed children, text-node split size deltas during tree edit/style operations, and element split size deltas across split levels. Merge-specific size checks still need broader parity tests. |
| CRDT tree element | covered | `CrdtElement::Tree` delegates metadata, JSON, data size, removal, and deep copy. `CrdtTree::edit` ports JS `CRDTTree.edit` steps 01 to 05 by node ID: position resolution, unknown split sibling advance and range narrowing, deletion with split sibling cascade, merge with chained-merge flattening and delete propagation, element split (Fix 8, §7.3, §7.4, §7.5, §7.7, same-boundary ordering, next-sibling `insPrevID` link), and the §9.4 stamp on inserts declared in a merged-away parent. |
| Tree style operation | partial | Rust has `TreeStyleOperation` for element attribute set/remove, text-boundary split, reverse operation creation, op info, removed attribute GC registration, and initial unknown split sibling style propagation. Full concurrent split/style matrices still need parity tests. |
| Tree edit operation | partial | `TreeEditOperation` executes every edit `CrdtTree::edit` supports, carries `split_tickets` (JS `splitTickets`) with the JS fallback when they are empty, reports op infos in the JS `TreeEditOpInfo` shape (one empty change for a split, no `splitLevel`), and registers tree-node GC pairs. Undo and redo are ported: the reverse is an identity edit by `TreeRestoreSpan` where the captured spans describe the edit, else a split, a boundary deletion, or a copy-reinsert, as JS chooses; `restore_spans`, `retombstone_spans`, and `restore_mode` travel on the wire, checked against bytes JS wrote (`crates/yorkie-protocol/tests/tree_undo_interop.rs`). The JS history suites (`history_tree_test.ts`, `history_tree_concurrent_test.ts`, `history_tree_opinfo_test.ts`, `history_tree_split_test.ts`) are ported. |
| Attribute RHT reuse | partial | Tree nodes now reuse `Rht`; visible attributes serialize deterministically, and removed attribute nodes become root GC pairs through tree style operations. |
| Tree GC | partial | Removed tree nodes and removed tree attributes are registered and purged through root GC. Edits register removed nodes, merge-propagated and stranded deletions, content born removed, and pieces split off removed nodes; purging a node links its split siblings to each other as JS does. Attribute tombstones a split product or a restore's recreated node copies are not registered (see the gaps doc). A node an undo revives unregisters its pair; one GC purged is recreated from its span. |
| Path/index conversion | partial | Rust now ports the JS/Go index/path/position conversion rules, including element padding, text-child paths, floor lookup for split text positions, parent/left path resolution, and token traversal for edit/style collection. It still recomputes over the current tree instead of maintaining the same stable index-tree structure as JS/Go. |
| Upstream skipped history/unit cases | blocked | JS and Go skip Tree cases around concurrent undo segmentation, chained L2 split undo, mixed-level merge, and generated concurrency failures. Rust ports them ignored, with the upstream reason, until upstream unskips them. See `upstream-skipped-tests.md`. |
| Public tree facade | partial | `JsonTree` ports JS `Tree` insert, delete, split, merge, style, and remove-style editing, by index and by path, with `splitByPath` and `mergeByPath`. Tests: the single-document and split/merge cases of `tree_test.ts`, `tree_split_opinfo_test.ts`, `tree_concurrent_split_merge_test.ts`, `tree_merge_anchor_test.ts`, and every matrix of `tree_concurrency_test.ts`, whose results match a JS SDK run. `styleByPath`, `removeStyleByPath`, and the range conversions are not ported. |
| Wire conversion | partial | Full tree `JSONElement` payloads and tree edit/style operation bodies convert to/from protobuf-shaped wire values, including tree node IDs, positions, attributes, split links, and merge metadata. Set, Add and ArraySet element payloads drop their trees' split links on decode and on undo (JS `dropSplitLinksInElement`). Rust now ports the direct tree-node conversion, standalone tree bytes, edit/style root bytes, merge-state root bytes, and protocol-level tree payload converter scenarios through stable internal fixtures. Public tree facade, presence/sync fixtures, and external binary fixtures are still missing. |

## Next Checks

- Extend tree index/path coverage with removed-node and mixed element/text
  scenarios before public tree editing.
- Extend tree style operation tests from the current direct split-sibling
  propagation toward version-vector-aware concurrent style matrices.
- Keep split/merge metadata (`insPrevID`, `insNextID`, `mergedFrom`,
  `mergedAt`, `mergedInto`) aligned with JS/Go when adding edit operations.
- Port the JS style guards for merged-away anchors (§9.4) if a style case
  diverges.
- Add protocol-level tree edit/style replay fixtures after the remaining
  in-repo converter tests are ported.
