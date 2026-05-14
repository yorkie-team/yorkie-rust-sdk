# Counter Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/counter.ts`
- JS tests: `packages/sdk/test/integration/counter_test.ts`,
  `packages/sdk/test/unit/document/crdt/counter_test.ts`
- Go: `pkg/document/crdt/counter.go`,
  `pkg/document/crdt/counter_test.go`,
  `pkg/document/operations/increase.go`
- Rust: not implemented yet

## Scope

Counter element representation, integer/long value handling, increase
operation, concurrent increments, JSON output, data size, and protocol mapping.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| CRDT counter element | missing | No Rust counter element variant yet. |
| Increase operation | missing | No Rust increase operation yet. |
| Public JSON counter facade | missing | Depends on public editing model. |
| Concurrent increment tests | missing | Port after the CRDT element and operation exist. |
| Wire conversion | missing | Depends on operation conversion. |

## Next Checks

- Port CRDT counter value and tests before public facade work.
- Add `IncreaseOperation` and operation-level tests.
- Cross-check integer widening and overflow behavior against JS/Go.
