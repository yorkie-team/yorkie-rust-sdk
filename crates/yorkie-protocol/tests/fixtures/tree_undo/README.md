# Tree undo interop fixtures

Checked by `crates/yorkie-protocol/tests/tree_undo_interop.rs`.

- `restore_retombstone_pack.hex`: a Tree deletion, its undo (`restore`) and
  redo (`retombstone`), then a replace of a styled paragraph, whose undo and
  redo carry both span lists.
- `merge_undo_pack.hex`: a merge of two paragraphs, its undo (a split with
  `split_tickets`) and its redo.

Each `.hex` file is protobuf `ChangePack` bytes written by the JS SDK at
`8629993e` with Node 22 and pnpm 9. The `.json` file next to it is the XML
JS read after each change. Command, from a scratch copy of yorkie-js-sdk:
`cp tools/interop/tree_undo_fixtures_test.ts <js-sdk>/packages/sdk/test/unit/api/ && cd <js-sdk> && TREE_UNDO_FIXTURE_DIR=<this directory> pnpm sdk exec vitest run test/unit/api/tree_undo_fixtures_test.ts`

There are no Go copies: Go fixtures are optional for this port, and the JS
ones already pin the bytes Rust reads and writes.
