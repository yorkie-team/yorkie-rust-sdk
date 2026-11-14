# Yorkie Rust SDK

A Rust port of the [Yorkie JS SDK](https://github.com/yorkie-team/yorkie-js-sdk).
The JS SDK is the source of truth for behavior, names and tests; the Go server
([yorkie-team/yorkie](https://github.com/yorkie-team/yorkie)) is what the wire
format and the CRDT rules must converge with. Read
[docs/porting-from-js-sdk.md](docs/porting-from-js-sdk.md) before porting
anything.

## Layout

Cargo workspace, Rust pinned in `rust-toolchain.toml`.

- `crates/yorkie-core`: document model, CRDTs, changes, operations, logical
  clocks. No network dependencies.
- `crates/yorkie-protocol`: generated protobuf types and converters.
- `crates/yorkie-client`: async client, Connect transport, sync, watch,
  channels.
- `crates/yorkie`: the public facade (re-exports only), the examples and the
  integration tests.
- `tools/protogen`: regenerates the protobuf code. `tools/interop`: test
  sources copied into the JS SDK and the Go server to produce the fixtures
  under `crates/yorkie-protocol/tests/fixtures/`; nothing here runs them.

Dependencies flow one way, `yorkie` → `yorkie-client` → `yorkie-protocol` →
`yorkie-core`: see [docs/dependency-boundaries.md](docs/dependency-boundaries.md).

## Commands

```sh
make verify   # everything CI's `check` job runs; needs no server
make fmt-check
make clippy   # cargo clippy --workspace --all-targets -- -D warnings
make test     # cargo test --workspace
make doc      # rustdoc for the `yorkie` crate, warnings denied

cargo fmt --all                              # fix formatting
cargo test -p yorkie-core <name>             # one test
cargo run -p protogen                        # regenerate the protobuf code
```

Run `make verify` before every push.

The integration tests (`crates/yorkie/tests/integration_*.rs`) return early,
and pass, unless `YORKIE_TEST_RPC_ADDR` is set. To run them, start a server
with an in-memory database and point the tests at it:

```sh
yorkie server --rpc-port 18080 --profiling-port 18081 &
YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie
```

CI runs them against the `yorkieteam/yorkie` image pinned in
`.github/workflows/ci.yml`.

## Task workflow

1. **Plan**: write `docs/tasks/active/YYYYMMDD-<slug>-todo.md` and its
   `-lessons.md` pair before code ([docs/tasks/README.md](docs/tasks/README.md)).
   If the change alters architecture, add or update a document under
   `docs/design/` from [TEMPLATE.md](docs/design/TEMPLATE.md) and list it in
   [docs/design/README.md](docs/design/README.md).
2. **Branch and commit**: a topic branch from `main`; never commit on `main`.
3. **Verify**: `make verify`.
4. **Open a PR**: the body follows `.github/PULL_REQUEST_TEMPLATE.md`. A change
   that ports JS SDK behavior links the JS SDK change or test it ports.
5. **Before merge**: move the finished task pair to
   `docs/tasks/archive/YYYY/MM/`.

## Commit messages

- The subject starts with a verb and says what changed, in 70 characters or
  less. No type prefix (`feat:`, `docs:`).
- A blank line, then a body wrapped at 80 columns that says why.
- Pull requests are squash-merged, so `main` carries one commit per PR, with
  the PR number at the end of the subject.

## Gotchas

- Never hand-edit `crates/yorkie-protocol/src/generated/yorkie.v1.rs`.
  Regenerate it with `cargo run -p protogen`; the test
  `protogen_output_is_up_to_date` fails when it is stale.
- The `.proto` files under `crates/yorkie-protocol/proto/` are copies. Their
  source of truth is the server repository.
- A test that the JS SDK or the Go server skips stays `#[ignore]`d here with
  the upstream reason:
  [docs/crdt-parity/upstream-skipped-tests.md](docs/crdt-parity/upstream-skipped-tests.md).
- Unit tests live beside the code: in a `#[cfg(test)] mod tests` at the bottom
  of the file, or in a `*_tests.rs` file its parent declares with
  `#[cfg(test)] mod <name>;`.
- Never hold a document lock across an `.await`.
- Known differences from the JS SDK are listed in
  [docs/current-porting-gaps.md](docs/current-porting-gaps.md). Update it when
  a port closes or opens one.
- The repository refuses force pushes on every branch. Add commits to a
  pushed branch; do not rebase or amend it.
