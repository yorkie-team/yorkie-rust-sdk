# Element and Metadata Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/element.ts`
- Go: `pkg/document/crdt/element.go`
- Rust: `crates/yorkie-core/src/crdt/element.rs`

## Scope

Common element metadata and dispatch behavior: creation time, moved time,
removed time, positioning, logical removal, deep copy, JSON conversion, data
size, and container dispatch.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Creation and ID access | covered | Rust exposes element identity through `created_at` and ID strings. |
| Removed timestamp LWW behavior | covered | Element tests cover later remove time behavior. |
| Moved timestamp LWW behavior | covered | Element tests cover later move time behavior. |
| Dispatch to primitive/object/array/text | partial | Primitive and text delegation are covered; tree/counter variants are missing. |
| Deep copy | partial | Implemented for current variants; ownership model may change later. |
| Data size | partial | Covered for implemented variants only. |
| Wire conversion | missing | No CRDT element protocol conversion yet. |

## Next Checks

- Add tree and counter variants before treating element dispatch as complete.
- Add wire conversion tests once operation/change protobuf conversion begins.
- Keep root index refresh tests close to any new element variant.
