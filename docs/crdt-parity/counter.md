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
| CRDT counter element | partial | Rust has regular integer and long counters, element dispatch, JSON output, bytes, deep copy, and data size. Dedup/HLL is not implemented yet. |
| Increase operation | partial | Rust has operation-level increase for primitive numeric operands, op info, reverse op generation, and root index refresh. Dedup actor handling only suppresses reverse op for now because dedup counters do not exist yet. |
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
- Dedup counters, HLL register serialization, and actor-based dedup increment
  semantics remain future work.

## Next Checks

- Add public `Counter` construction and `increase` support once the JSON facade
  can express CRDT leaf elements directly.
- Port concurrent counter tests through `Change` and document history when
  undo/redo is available.
- Add wire conversion tests for counter element and increase operation.
- Implement dedup/HLL only after regular counter behavior is stable.
