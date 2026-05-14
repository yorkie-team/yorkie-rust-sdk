# Counter Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/counter.ts`
- JS tests: `packages/sdk/test/integration/counter_test.ts`,
  `packages/sdk/test/unit/document/crdt/counter_test.ts`
- Go: `pkg/document/crdt/counter.go`,
  `pkg/document/crdt/counter_test.go`,
  `pkg/document/operations/increase.go`
- Rust: `crates/yorkie-core/src/crdt/counter.rs`,
  `crates/yorkie-core/src/operation/increase_operation.rs`

## Scope

Counter element representation, integer/long value handling, increase
operation, concurrent increments, JSON output, data size, and protocol mapping.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| CRDT counter element | partial | Rust has regular integer, long, and integer-dedup counters, element dispatch, JSON output, bytes, deep copy, HLL restore, and data size. Public/wire construction is still missing. |
| HLL dedup core | covered | Rust uses precision 14, xxhash64 seed 0, register max-merge, 16KB register serialization, and restore behavior matching JS/Go. |
| Increase operation | partial | Rust has operation-level increase for primitive numeric operands, op info, reverse op generation, root index refresh, and actor-based dedup increases. Public/history integration is still missing. |
| Public JSON counter facade | missing | Depends on public editing model. |
| Concurrent increment tests | missing | Port after change-level/public counter paths exist. |
| Wire conversion | missing | Depends on operation conversion. |

## Parity Notes

- Int counters normalize constructor values and operands to signed 32-bit
  wrapping values. Long counters normalize to signed 64-bit wrapping values.
- Numeric primitive operands are limited to Rust's current primitive model:
  integer, long, and double. Double operands are truncated before wrapping.
- Reverse operations preserve the same parent and use the negative numeric
  operand. Rust cannot represent a primitive integer outside `i32`, so the
  `i32::MIN` reverse operand is represented as a long with the same JSON value
  that JS emits.
- Dedup counters require an actor and only accept unit increments. Rust accepts
  integer, long, or double `1` as a unit increment, following JS behavior; Go's
  typed helper currently rejects float operands for dedup increments.
- HLL-backed dedup counters include serialized register bytes in data size,
  matching JS/Go.

## Next Checks

- Add public `Counter` construction and `increase` support once the JSON facade
  can express CRDT leaf elements directly.
- Port concurrent counter tests through `Change` and document history when
  undo/redo is available.
- Add wire conversion tests for counter element and increase operation.
- Add protocol conversion for HLL register payloads and dedup increase actors.
