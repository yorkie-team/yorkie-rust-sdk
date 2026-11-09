# Upstream Skipped Tests

Last reviewed: 2026-11-09

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
| Array history | `packages/sdk/test/integration/history_array_test.ts:101` | Undo matrix cases where `op1 === "move"` and `op3 === "set"` in the `add/remove/move/set` triple matrix. | The set operation restores at the moved element's original dead position; upstream notes that fixing this requires a proto-level change. | Skipped in the matrix loop of `returns_to_each_array_state` (`crates/yorkie-core/tests/history.rs`) until upstream unskips the cases. |
| Text history | `packages/sdk/test/integration/history_text_test.ts:705` | Case 3 correctness: both undo overlapping deletes should restore the original text. | Both clients converge to the same wrong content because deep-copy re-insert duplicates the overlapping removed range. | Runs live as `restores_the_original_after_both_undo_contained_deletions` in `crates/yorkie/tests/integration_history.rs`. A recorded divergence: the JS skip comment predates identity-preserving restore, which fixes the case, and Go runs it live (`test/integration/history_text_reconcile_test.go` `TestReconcileOverlappingUndoDuplicatesContent`). Rust ports that restore (`docs/plans/2026-11-09-text-removal-ledger.md`); see Undo and Redo in `docs/current-porting-gaps.md`. |
| Text history | `packages/sdk/test/integration/history_text_test.ts:742` | Case 5 correctness: both undo partially overlapping deletes should restore the original text. | Same deep-copy re-insert limitation as Case 3. | Runs live as `restores_the_original_after_both_undo_overlapping_deletions` in `crates/yorkie/tests/integration_history.rs`, for the same reason as Case 3. |
| Tree history | `packages/sdk/test/integration/history_tree_concurrent_test.ts:159` | "KNOWN: delete a whole <p> vs edit text inside it, both undo". | After GC the internal text-node segmentation diverges (the XML agrees); a fix needs undo-stack-aware GC. Go skips it too. | Ported as the ignored `converges_after_undo_of_a_paragraph_delete_vs_an_edit_inside_it` (`crates/yorkie/tests/integration_history_tree.rs`). |
| Tree history | `packages/sdk/test/integration/history_tree_concurrent_test.ts:202` | "KNOWN: delete two <p> vs edit inside first, both undo (segmentation)". | Same segmentation limit, with two elements deleted. | Ported as the ignored `converges_after_undo_of_a_two_paragraph_delete_vs_an_edit_inside`. |
| Tree split history | `packages/sdk/test/integration/history_tree_split_test.ts:855` | Chained undo case where `op1 === "split-l2"` and `op2 === "split-l2"`. | TODO(#1235): the boundary-deletion reverse does not restore the state when two consecutive L2 splits leave tombstoned structure. Go skips it too (`history_tree_split_test.go` 737). | Ported as the ignored `single_client_undoes_two_chained_level_two_splits` (`crates/yorkie-core/src/document/tree_undo_tests.rs`). |
| Tree CRDT unit | `packages/sdk/test/unit/document/crdt/tree_test.ts:1700` | `Can merge different levels with edit`. | Upstream TODO says the test and change assertions need to be fixed. | Ported as the ignored `crdt::tree::tests::merges_different_levels_with_edit`; do not infer new merge semantics from this test alone. |

Non-CRDT skips and todos found during this pass are excluded from the parity
inventory, such as schema validator todos, benchmark environment skips,
database setup skips, and server push-pull consistency skips.

## Go Inventory

| Area | Source | Skipped case | Upstream reason | Rust handling |
| --- | --- | --- | --- | --- |
| Tree concurrency | `test/complex/tree_concurrency_test.go:250` | Matrix subtests that fail convergence are dynamically skipped with the failure description. | The Go complex Tree concurrency runner treats non-converging generated combinations as skipped cases instead of hard failures. | Do not convert dynamically skipped Go matrix cases into Rust pass requirements. If ported, record the concrete generated case and keep the Rust test ignored. |

## Current Rust Status

Rust carries an ignored test for each skipped upstream Tree case listed
above: `crdt::tree::tests::merges_different_levels_with_edit` for the JS
Tree CRDT unit case, and the three Tree history cases. The two JS Text
history correctness cases run live, as Go runs them (see the inventory). At
JS `8629993e`, `history_tree_test.ts` skips nothing: its redo matrix and
reconcile Cases 3-6, skipped before, run in JS and in Rust
(`integration_history_tree.rs`). The JS `tree_concurrency_test.ts` matrices
have no skipped combinations, and every Rust combination converges.

When those layers are added, port active upstream tests first. Then add ignored
Rust tests for the skipped upstream cases only when they are useful as visible
tracking markers.
