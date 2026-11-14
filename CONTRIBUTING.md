# Contributing

Yorkie is Apache 2.0 licensed and accepts contributions via GitHub pull
requests.

If you have questions along the way, ask on the
[Yorkie Discord](https://discord.com/invite/MVEAwz9sBy).

## Contribution flow

- Create a topic branch from `main`.
- Make commits of logical units, with messages in the format below.
- Run `make verify`.
- Push the branch and open a pull request against `main`, following
  `.github/PULL_REQUEST_TEMPLATE.md`.
- A maintainer reviews the pull request and squash-merges it.

The repository refuses force pushes on every branch, so answer a review with
new commits rather than by rewriting the branch.

This SDK is a port. A change to behavior starts from the JS SDK: read
[docs/porting-from-js-sdk.md](docs/porting-from-js-sdk.md), and say in the pull
request which JS SDK change or test it ports.

## Building and testing

The Rust version is pinned in `rust-toolchain.toml`; `rustup` installs it on
first use.

```sh
make verify   # format check, clippy, unit tests, rustdoc: what CI checks without a server
```

`make verify` runs `make fmt-check`, `make clippy`, `make test` and `make doc`.
`cargo fmt --all` fixes formatting.

The integration tests need a Yorkie server. Without one they return early and
pass, so `make verify` does not exercise them:

```sh
yorkie server --rpc-port 18080 --profiling-port 18081 &
YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie
```

The protobuf code under `crates/yorkie-protocol/src/generated/` is generated.
After changing a `.proto` file (their source of truth is
[yorkie-team/yorkie](https://github.com/yorkie-team/yorkie/tree/main/api)),
regenerate it:

```sh
cargo run -p protogen
```

## Commit messages

```
Re-issue pre-attach tickets on attach

A document edited before its first attach mints its tickets under the
initial actor, which every such document shares. Two clients that edit
the same document key before attaching then push elements with the same
created_at, and replicas can disagree on the winner.
```

- The subject starts with a verb, says what changed, and is 70 characters or
  less. No type prefix.
- A blank line separates it from the body.
- The body is wrapped at 80 columns and says why.

## Documents

- [docs/README.md](docs/README.md): the development contract (porting rules,
  dependency boundaries, parity checklists, known gaps).
- [docs/design/](docs/design/README.md): design documents for changes to the
  architecture.
- [docs/tasks/](docs/tasks/README.md): the plan and the lessons of each task.
