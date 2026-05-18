# Primitive Parity

Last reviewed: 2026-05-18

## References

- JS: `packages/sdk/src/document/crdt/primitive.ts`
- JS tests: `packages/sdk/test/integration/primitive_test.ts`,
  `packages/sdk/test/unit/document/crdt/primitive_test.ts`
- Go: `pkg/document/crdt/primitive.go`,
  `pkg/document/crdt/primitive_test.go`
- Rust: `crates/yorkie-core/src/crdt/primitive.rs`,
  `crates/yorkie-core/src/wire.rs`,
  `crates/yorkie-protocol/src/converter.rs`

## Scope

Primitive value representation, JSON serialization, value typing, bytes
encoding, metadata, removal, movement, and deep copy.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Basic primitive values | covered | Rust supports current primitive value enum and JSON output. |
| Numeric type identification | covered | Unit tests cover numeric type helpers. |
| Byte encoding | covered | Unit tests cover expected byte output and round trips. |
| Metadata and deep copy | covered | Creation/removal metadata and deep copy are tested. |
| Date/bytes edge cases | partial | Needs broader JS/Go parity once public JSON facade matures. |
| Wire conversion | partial | Primitive simple elements, full primitive `JSONElement` payloads, and primitive operation operands convert to/from protobuf-shaped wire values. Cross-language binary fixtures are still missing. |

## Next Checks

- Compare all JS primitive integration cases with Rust values.
- Add date and binary edge cases if the public Rust value layer supports them.
- Add JS/Go-produced protobuf fixtures with exact primitive type mapping.
