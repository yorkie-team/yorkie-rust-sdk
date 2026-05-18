# Counter Parity

Last reviewed: 2026-05-18

## References

- JS: `packages/sdk/src/document/crdt/counter.ts`
- JS tests: `packages/sdk/test/integration/counter_test.ts`,
  `packages/sdk/test/unit/document/crdt/counter_test.ts`
- Go: `pkg/document/crdt/counter.go`,
  `pkg/document/crdt/counter_test.go`,
  `pkg/document/operations/increase.go`
- Rust: `crates/yorkie-core/src/crdt/counter.rs`,
  `crates/yorkie-core/src/operation/increase_operation.rs`,
  `crates/yorkie-core/src/json.rs`,
  `crates/yorkie-core/src/document.rs`,
  `crates/yorkie-core/src/wire.rs`,
  `crates/yorkie-protocol/src/converter.rs`

## Scope

Counter element representation, integer/long value handling, increase
operation, concurrent increments, JSON output, data size, and protocol mapping.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| CRDT counter element | partial | Rust has regular integer, long, and integer-dedup counters, element dispatch, JSON output, bytes, deep copy, HLL restore, data size, and public JSON construction paths. Wire construction is still missing. |
| HLL dedup core | covered | Rust uses precision 14, xxhash64 seed 0, register max-merge, 16KB register serialization, and restore behavior matching JS/Go. |
| Increase operation | partial | Rust has operation-level increase for primitive numeric operands, op info, reverse op generation, root index refresh, actor-based dedup increases, and public recorder integration. History and sync integration are still missing. |
| Public JSON counter facade | partial | `JsonCounter` supports regular and dedup counters through object/array helpers, same-update creation and increase, existing-counter increase, long overflow, and dedup actor-add tests. The shape is Rust-specific rather than JS constructor/proxy syntax. |
| Concurrent increment tests | missing | Port after client sync/history paths exist. |
| Wire conversion | partial | Counter set and increase operations convert to proto-shaped Rust payloads, including counter value types, primitive increase operands, and dedup actors. From-protocol conversion, protobuf binary encoding, and full `JSONElement.Counter` HLL register conversion are still missing. |

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
- Public `JsonCounter::add` is restricted to dedup counters, matching the
  split public shape where regular counters expose increase and dedup counters
  expose actor-add semantics.
- `yorkie_protocol::converter::to_change_pack` follows JS/Go converter shape:
  counter element creation uses counter value types, while increase operands
  remain primitive numeric element-simple values.

## Next Checks

- Port concurrent counter tests through client sync and document history when
  those layers are available.
- Add from-protocol conversion tests for counter element and increase
  operation.
- Add protocol conversion for full counter `JSONElement` payloads with HLL
  register bytes.
- Revisit public constructor ergonomics once the top-level Rust SDK facade is
  shaped around typed document editing.
