# Text and RGATreeSplit Parity

Last reviewed: 2026-11-09

## References

- JS: `packages/sdk/src/document/crdt/text.ts`,
  `packages/sdk/src/document/crdt/rga_tree_split.ts`,
  `packages/sdk/src/document/json/text.ts`
- JS tests: `packages/sdk/test/integration/text_test.ts`,
  `packages/sdk/test/integration/history_text_test.ts`
- Go: `pkg/document/crdt/text.go`,
  `pkg/document/crdt/rga_tree_split.go`,
  `pkg/document/crdt/text_test.go`,
  `pkg/document/crdt/rga_tree_split_test.go`
- Rust: `crates/yorkie-core/src/crdt/text.rs`,
  `crates/yorkie-core/src/crdt/rga_tree_split.rs`,
  `crates/yorkie-core/src/crdt/splay.rs`,
  `crates/yorkie-core/src/operation/edit_operation.rs`,
  `crates/yorkie-core/src/operation/style_operation.rs`,
  `crates/yorkie-core/src/wire.rs`,
  `crates/yorkie-protocol/src/converter.rs`

## Scope

Text blocks, split positions, UTF-16 indexing, edit/delete, styling, attribute
RHT integration, removed node GC, operation execution, version-vector-aware
behavior, public text facade, and operation info output.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Text block storage | covered | `CrdtText` and `RgaTreeSplit` exist for current text values. |
| UTF-16 code unit indexing | partial | Covered for valid Rust strings; invalid standalone surrogate parity needs a design choice. |
| Edit/delete behavior | covered | Tests cover split positions, composition replacement, boundary deletion, and last-node deletion. |
| Style/remove-style behavior | covered | Tests cover overlap, conflict LWW, concurrent insert formatting, and style removal. |
| Text GC pairs | covered | Removed text nodes and removed attributes are registered and purged. |
| Operation execution | partial | Edit/style operations execute and register GC pairs, but operation info shape is not final. |
| Removal ledger and identity undo | covered | Ports JS `84068f70`: per-node removal ledger, live-in-view remote deletion, `restore`/`retombstone`, ledger pruning and `held_from_purge` in GC, identity undo and redo, and the wire fields. The JS ledger, restore, and fuzz tests are ported; JS vs Go divergences are listed in `docs/current-porting-gaps.md`. |
| Upstream skipped history cases | divergent | JS still skips the overlapping-delete undo correctness cases with a stale comment; Go runs them live, and Rust runs them live in `integration_history.rs`. See `upstream-skipped-tests.md`. |
| Public Text facade | missing | No public context-backed text API yet. |
| Splay/ID lookup optimization | partial | `RgaTreeSplit` now keeps `tree_by_index` and `tree_by_id` equivalents and uses weighted splay lookup for text indexes. Structural mutations still rebuild indexes around the current `Vec` storage instead of using stable linked-node handles. |
| History and multi-client scenarios | partial | Important integration-style cases still need operation-level replay tests. |
| Wire conversion | partial | Full text `JSONElement` payloads and edit/style operation bodies convert to/from protobuf-shaped wire values. Rust ports the text composition/style portion of JS's root bytes scenario at core wire level and covers a mixed protocol `ChangePack` replay with text edit/style operations. Restore spans and `removal_events` cross the wire, checked against change-pack and snapshot bytes JS and Go wrote (`crates/yorkie-protocol/tests/text_undo_interop.rs`). Public text facade, sync replay fixtures, and external event payload shape are still missing. |

## Next Checks

- Align edit operation info with the value-change list from the reference
  implementation before exposing events.
- Add operation-level replay tests for multi-change text scenarios.
- Extend protocol-level text edit/style replay from the current mixed
  `ChangePack` fixture toward multi-change and concurrent cases.
- Decide how Rust should represent invalid UTF-16 surrogate edges.
- Replace rebuild-on-mutation indexing with stable node handles to align the
  write-side implementation more closely with JS/Go.
