# Documentation

This directory captures the development contract for the Rust SDK.

- [Dependency Boundaries](dependency-boundaries.md): crate and module dependency
  direction rules.
- [Current Porting Gaps](current-porting-gaps.md): known differences between
  the current Rust implementation and the JS/Go implementations, including the
  out-of-scope list with JS source paths.
- [CRDT Parity Checklists](crdt-parity/README.md): per-element CRDT parity
  status, missing tests, and next checks.
- [Upstream Skipped Tests](crdt-parity/upstream-skipped-tests.md):
  CRDT-related skipped JS/Go tests that must stay skipped or ignored in Rust
  until upstream behavior changes.
- [JS SDK Parity Plan](plans/2026-11-07-js-sdk-parity.md): milestone plan for
  closing the gap to the JS SDK (completed 2026-11-08).
- [Porting from the JS SDK](porting-from-js-sdk.md): source-of-truth rules,
  Go SDK cross-check policy, naming policy, and the test-driven porting
  workflow.
