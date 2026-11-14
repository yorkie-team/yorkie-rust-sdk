**Created**: 2026-11-14

# Lessons: onboarding onto the shared agent harness

## Compiling is executing, so the toolchain action stops at the compiler

The Go and pnpm adopters' toolchain actions can install dependencies without
running branch code (`go install` of a pinned linter; `pnpm install
--ignore-scripts`). Cargo has no such mode: every build runs build scripts and
procedural macros. The action here installs rustup's toolchain and never
invokes cargo; the agent's own `make verify` does the first build. The cost is
a cold build in every agent job.

## A toolchain file in the workspace beats `rustup default`

`rustup default 1.99.1` is not enough: a `rust-toolchain.toml` in the working
directory wins over the default, and it may name a `path` to binaries the
branch ships. `RUSTUP_TOOLCHAIN` wins over the file. Checked in a container:
with a hostile toolchain file, `rustc --version` reported the branch's binary
without the variable and 1.99.1 with it.

## The `rust` Docker image is not the toolchain file

`rust:1.99.1` installs the minimal profile, without rustfmt or clippy. Run
inside the workspace, rustup then installs them because
`rust-toolchain.toml` lists them. That download is not part of the build time
measured.

## `make verify` is green without running the integration tests

Each `integration_*.rs` test returns early when `YORKIE_TEST_RPC_ADDR` is
unset. A fixer's `make verify` therefore says nothing about client and server
behavior. The prompts and the coverage note say so; only CI's
`integration tests` lane runs them.

## A list that puts files in scope has to be derived, not typed

`tests.cargo.testOnlyFileGlobs` puts whole files in a fix round's scope. The
first test for it asked the harness which files its declaration check
follows, but the harness answers "not followed" for any file the list already
names, so a needless entry justified itself. `repo.test.mjs` now walks every
`mod` declaration in the workspace and computes the list: reached only under
`#[cfg(test)]`, and not followed by the check.

## Guards for files that do not exist

A `.cargo/config.toml`, a `build.rs`, a `rustfmt.toml` or a legacy
`rust-toolchain` file would change what CI runs the day one is created. They
are CI-defining now, ahead of existing, and `repo.test.mjs` holds each on a
`RESERVED` list that fails when the file appears.

## Measured

Cold `make verify` in `rust:1.99.1` on linux/arm64, four CPUs, dependencies
downloaded: fmt 0 s, clippy 22 s, tests 113 s, rustdoc 4 s. 1,332 tests
passed, 10 ignored. `target/` was 6.6 GB afterwards.
