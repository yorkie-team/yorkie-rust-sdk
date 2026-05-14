# Root and Garbage Collection Parity

Last reviewed: 2026-05-14

## References

- JS: `packages/sdk/src/document/crdt/root.ts`,
  `packages/sdk/src/document/crdt/gc.ts`
- JS tests: `packages/sdk/test/unit/document/crdt/root_test.ts`,
  `packages/sdk/test/unit/document/gc_test.ts`,
  `packages/sdk/test/integration/gc_test.ts`
- Go: `pkg/document/crdt/root.go`,
  `pkg/document/crdt/gc.go`,
  `pkg/document/crdt/root_test.go`,
  `pkg/document/gc_test.go`
- Rust: `crates/yorkie-core/src/crdt/root.rs`

## Scope

Root object ownership, element lookup by created time, parent relationship,
path creation, removed element registration, internal GC pair registration,
document size accounting, garbage collection with version vectors, and root
deep copy/rebuild.

## Status

| Area | Status | Notes |
| --- | --- | --- |
| Root object and element lookup | covered | Tests cover basic object members and nested descendants. |
| Path creation | covered | Tests cover object, array, and text paths for current variants, including root operation matrix cases that refresh descendant parent snapshots. |
| Removed element GC | covered | Root tracks removed elements and deregisters them on GC. |
| Array dead position GC | covered | Dead RGA position nodes are registered, copied, and purged. |
| Text node/attribute GC | covered | Root rebuilds and purges text internal GC pairs. |
| Document size accounting | partial | Current tests cover narrow slices, including array move/remove size consistency after root rebuild. Broad JS/Go size tests remain. |
| Deep copy index rebuild | partial | Covered for current variants, including array moved/dead position lookup; tree/counter missing. |
| Sync lifecycle GC | blocked | Requires client/sync lifecycle and server version vectors. |
| Snapshot GC rebuild | partial | Array moved/dead positions and text internal pairs rebuild through copied root objects; snapshot application is missing. |

## Next Checks

- Add doc size tests for array dead position GC and mixed element/internal GC.
- Add root-level tests whenever a new CRDT element variant is introduced.
- Defer sync lifecycle GC tests until client and protocol layers exist.
