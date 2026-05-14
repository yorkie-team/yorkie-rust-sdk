# RHT Attributes Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/rht.ts`
- JS tests: `packages/sdk/test/unit/document/crdt/rht_test.ts`
- Go: `pkg/document/crdt/rht.go`,
  `pkg/document/crdt/rht_test.go`
- Rust: `crates/yorkie-core/src/crdt/rht.rs`

## Scope

Attribute map storage, LWW set/remove behavior, tombstones, removed-node GC
candidates, data size, JSON conversion, deterministic ordering, and text/tree
integration.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Set/get/has behavior | covered | Rust tests cover the basic RHT flow. |
| Remove and tombstones | covered | Tests cover removal, repeated removal, and missing-key tombstones. |
| LWW conflict behavior | covered | Tests cover late sets/removes and newer tombstones. |
| Purge behavior | covered | Tests cover current tombstone purge behavior. |
| Text integration | covered | Text values use `Rht`; style operations register removed attribute GC pairs. |
| Tree integration | partial | Tree nodes use `Rht`; visible attributes serialize to JSON/XML and removed attributes become root GC pairs when rebuilding from CRDT state. Operation-time tree style registration is pending tree operations. |
| JSON ordering | partial | Rust uses deterministic key ordering; direct JS `Map` ordering differs, while text output is aligned. |
| Wire conversion | missing | No attribute protocol conversion yet. |

## Next Checks

- Register removed tree attributes during tree style operations once those
  operations are ported.
- Keep text output parity tests focused on `CRDTTextValue` serialization rather
  than raw `RHT.toJSON` insertion order.
- Add protocol conversion once text/tree operations are converted.
