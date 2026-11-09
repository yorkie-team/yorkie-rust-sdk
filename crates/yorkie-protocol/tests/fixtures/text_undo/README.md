# Text undo interop fixtures

Checked by `crates/yorkie-protocol/tests/text_undo_interop.rs`.

- `restore_retombstone_pack.hex`, `ledger_snapshot_pack.hex`: protobuf
  `ChangePack` bytes written by the JS SDK at `84068f70` (checkout
  `8629993e`, which adds only upgrade notes) with Node 22 and pnpm 9. The
  `.json` file next to each is the sorted JSON of the document JS built.
  Command, from a scratch copy of yorkie-js-sdk:
  `cp tools/interop/text_undo_fixtures_test.ts <js-sdk>/packages/sdk/test/unit/api/ && cd <js-sdk> && TEXT_UNDO_FIXTURE_DIR=<this directory> pnpm sdk exec vitest run test/unit/api/text_undo_fixtures_test.ts`
- `go_ledger_snapshot_pack.hex`: the ledger snapshot written by Go yorkie at
  `54b96762`. Go writes `restore_retombstone_pack.hex` byte for byte, so it
  has no Go copy. Command, from a scratch copy of yorkie:
  `cp tools/interop/text_undo_fixtures_test.go <yorkie>/pkg/document/ && cd <yorkie> && TEXT_UNDO_FIXTURE_DIR=<this directory> go test ./pkg/document -run TestTextUndoInteropFixtures -count=1`
