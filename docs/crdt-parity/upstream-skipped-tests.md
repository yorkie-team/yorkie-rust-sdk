# Upstream Skipped Tests

Last reviewed: 2026-05-14

This document tracks CRDT-related tests that are skipped or dynamically skipped
in the JS SDK or Go implementation. These tests are not Rust pass targets while
they remain skipped upstream.

## Policy

- Active JS SDK tests are Rust parity targets.
- Active Go tests are secondary parity targets when they clarify typed CRDT
  internals.
- Skipped upstream tests are known unresolved or underspecified behavior. Do not
  silently solve them in Rust during parity work.
- If a skipped upstream case is represented in Rust, keep it as an ignored test
  with a note that points to the upstream file, line, and skip condition.
- Do not remove the Rust ignore marker until the JS SDK unskips the case or a
  dedicated design decision records a deliberate Rust divergence.
- If Rust must intentionally handle one of these cases earlier than upstream,
  document the difference in this file and in `docs/current-porting-gaps.md`
  before changing the behavior.

## JS SDK Inventory

| Area | Source | Skipped case | Upstream reason | Rust handling |
| --- | --- | --- | --- | --- |
| Array history | `packages/sdk/test/integration/history_array_test.ts:101` | Undo matrix cases where `op1 === "move"` and `op3 === "set"` in the `add/remove/move/set` triple matrix. | The set operation restores at the moved element's original dead position; upstream notes that fixing this requires a proto-level change. | Keep skipped or ignored until upstream unskips the cases. |
| Text history | `packages/sdk/test/integration/history_text_test.ts:705` | Case 3 correctness: both undo overlapping deletes should restore the original text. | Both clients converge to the same wrong content because deep-copy re-insert duplicates the overlapping removed range. | Keep skipped or ignored; do not optimize Rust undo to pass this case without an upstream decision. |
| Text history | `packages/sdk/test/integration/history_text_test.ts:742` | Case 5 correctness: both undo partially overlapping deletes should restore the original text. | Same deep-copy re-insert limitation as Case 3. | Keep skipped or ignored; track with history undo/redo parity. |
| Tree history | `packages/sdk/test/integration/history_tree_test.ts:414` | Redo matrix case where `op1 === "insert-text"` and `op2 === "delete-text"`. | Upstream marks adjacent or overlapping redo ranges as Phase 2 because some combinations diverge. | Keep skipped or ignored when Tree history tests are ported. |
| Tree history | `packages/sdk/test/integration/history_tree_test.ts:574` | Case 3 `contained_by`: undo range contained by remote should collapse. | Overlapping reconciliation needs symmetric index computation. | Keep skipped or ignored. |
| Tree history | `packages/sdk/test/integration/history_tree_test.ts:619` | Case 4 `contains`: remote range contained by undo should adjust. | Overlapping reconciliation needs symmetric index computation. | Keep skipped or ignored. |
| Tree history | `packages/sdk/test/integration/history_tree_test.ts:667` | Case 5 `overlap_start`: remote overlaps start of undo range. | Overlapping reconciliation needs symmetric index computation. | Keep skipped or ignored. |
| Tree history | `packages/sdk/test/integration/history_tree_test.ts:712` | Case 6 `overlap_end`: remote overlaps end of undo range. | Overlapping reconciliation needs symmetric index computation. | Keep skipped or ignored. |
| Tree split history | `packages/sdk/test/integration/history_tree_split_test.ts:799` | Chained undo case where `op1 === "split-l2"` and `op2 === "split-l2"`. | Upstream notes a known undo bug in the boundary-deletion reverse operation when consecutive L2 splits create tombstoned structure. | Keep skipped or ignored. |
| Tree CRDT unit | `packages/sdk/test/unit/document/crdt/tree_test.ts:1444` | `Can merge different levels with edit`. | Upstream TODO says the test and change assertions need to be fixed. | Keep skipped or ignored; do not infer new merge semantics from this test alone. |

Non-CRDT skips and todos found during this pass are excluded from the parity
inventory, such as schema validator todos, benchmark environment skips,
database setup skips, and server push-pull consistency skips.

## Go Inventory

| Area | Source | Skipped case | Upstream reason | Rust handling |
| --- | --- | --- | --- | --- |
| Tree concurrency | `test/complex/tree_concurrency_test.go:250` | Matrix subtests that fail convergence are dynamically skipped with the failure description. | The Go complex Tree concurrency runner treats non-converging generated combinations as skipped cases instead of hard failures. | Do not convert dynamically skipped Go matrix cases into Rust pass requirements. If ported, record the concrete generated case and keep the Rust test ignored. |

## Current Rust Status

Rust does not currently carry ignored tests for the skipped upstream cases
listed above. That is intentional for now because several required layers are
not yet present, especially full history undo/redo and Tree edit/style
operations.

When those layers are added, port active upstream tests first. Then add ignored
Rust tests for the skipped upstream cases only when they are useful as visible
tracking markers.
