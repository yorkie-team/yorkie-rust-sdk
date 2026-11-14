# .github/agent-harness

This repository's half of the agent harness: everything the `@claude` command
surface needs to know about yorkie-rust-sdk. The other half — `scripts/agent/**`
and every `.github/workflows/agent-*.yml` — is the same bytes in every
repository that runs the harness. It is documented in
[`scripts/agent/README.md`](../../scripts/agent/README.md), which is also where
every configuration key, the prompt frames, the cargo test rules, the glob
dialect and the toolchain action's contract are specified.

## Source

The shared half is copied from
[yorkie-team/yorkie](https://github.com/yorkie-team/yorkie) by that
repository's `scripts/agent/sync.mjs`. [`upstream.json`](upstream.json) records
the commit and the manifest digest; nothing under `scripts/agent/` or in an
`agent-*.yml` is edited here. Something this repository needs that the shared
half cannot express goes upstream first.

Comments in the shared files that cite `docs/design/agent-command-verbs.md` or
other design documents by path mean the server repository's documents; they
are not copied here. Where a message links to that one, it is by URL
(`docs.commandVerbsRef`).

## What is this repository's

| File | What it is |
|---|---|
| `config.json` | Every repository-specific value: the CI-defining paths, the file-class rules, the lens source globs, the generated file left out of a reviewed diff, the sentences the fixer prompts are completed with, the test rules (`cargo`, and `vitest` for the harness's own tests), the fix guard's replicated-rule paths and its two counterpart repositories |
| `coverage-note.md` | What `ci.yml`'s two lanes prove and do not, told to every lens |
| `repo.test.mjs` | Grades the two files above, and the toolchain action, against this repository (`npm run test:repo` in `scripts/agent`). The reason each configured entry exists is written there, beside the assertion that pins it |
| `repo.snapshot.json` | Every value the workflows read and every prompt as rendered here, recorded. A change to a prompt or a gate shows its before and after in this file's diff |
| `upstream.json` | Written by `sync.mjs`: where the shared half came from |
| `../actions/agent-toolchain/action.yml` | Rust at the version `rust-toolchain.toml` pins, with rustfmt and clippy, installed by rustup from outside the workspace. It never invokes cargo, uses no other action and saves no cache, and it exports `RUSTUP_TOOLCHAIN` so the workspace's toolchain file does not choose the compiler for the rest of the job |

Where upstream's text assumes something this repository does not have:

- There is no `.github/CODEOWNERS` here. `scripts/agent/README.md` says
  workflow files are "CODEOWNER-ed, so every edit to one still needs a human
  reviewer". What holds here is the other half: a pull request that touches a
  CI-defining path is never promoted automatically, so a person reviews it.
- There are no git hooks and no `scripts/setup.sh`. `fixer.commitNote` states
  the commit-message convention without saying anything checks it.
- The design contract is not only `docs/design/**`. The porting rules, the
  dependency boundaries, the parity checklists, the known-gaps list and the
  milestone plans are listed under `review.classRules.design-spec`, so the
  design-fit lens reads them.
- `make verify` passes the integration tests without running them: each
  returns early unless `YORKIE_TEST_RPC_ADDR` is set, and no agent job starts
  a server. The prompts and the coverage note both say so. Only CI's
  `integration tests` lane exercises them.

## Compiling is executing

`make verify` compiles, and compiling runs the build script and the procedural
macros of every crate `Cargo.lock` names. The toolchain action therefore
installs the compiler and nothing else, and every agent job builds from
nothing, with no cache. The manifests, the lockfile, the toolchain file and
any `.cargo/**`, `build.rs`, rustfmt or clippy configuration are CI-defining:
a pull request that touches one is never promoted automatically.

## Syncing from yorkie

1. Fetch yorkie and check out its `main` (a copy is synced from the default
   branch only).
2. From this repository's root:

   ```sh
   node scripts/agent/sync.mjs --from ../yorkie
   ```

   It overwrites the shared half, deletes what upstream no longer has, writes
   `upstream.json`, and prints each configuration key the new schema requires
   or no longer knows.
3. Edit `config.json` to match, then
   `git add scripts/agent .github/workflows .github/agent-harness`.
4. In `scripts/agent`: `npm ci --ignore-scripts`, `npm test`,
   `npm run test:repo` and `npm run check:upstream`. A failure in
   `repo.snapshot.json` is a prompt or a gate that changed: read the
   difference, then record it with `AGENT_HARNESS_RECORD=1 npm run test:repo`
   and run the suite once more without the variable.
5. actionlint from the repository root:

   ```sh
   docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:1.7.12 -shellcheck= -pyflakes=
   ```

## When this repository changes

- The Rust version: `rust-toolchain.toml`, both `toolchain:` inputs in
  `ci.yml`, `PINNED_TOOLCHAIN` in the toolchain action and the version in
  `coverage-note.md`, in one change. `repo.test.mjs` fails until all agree.
- A new lane in `ci.yml`, or one removed: `coverage-note.md`, and the
  assertions about it in `repo.test.mjs`. A command added to the `check` job
  goes into the Makefile as a target that `verify` runs.
- The server image in `ci.yml`: the tag in `coverage-note.md`.
- A new file that decides what CI does (a `.cargo/config.toml`, a `build.rs`,
  a `rustfmt.toml` or `clippy.toml`): it is already CI-defining. Take it off
  `RESERVED` in `repo.test.mjs`, and say in `coverage-note.md` and in the
  toolchain action's header what changed.
- A new crate: nothing, while it lives under `crates/` or `tools/`.
- A test module the scope check cannot follow (one reached through `#[path]`,
  a directory that is itself a test module, a test-only module whose name does
  not end in `_tests`): `tests.cargo.testOnlyFileGlobs`. `repo.test.mjs`
  derives the list from the `mod` declarations and fails until it matches.
- A new directory whose rules the JS SDK and the server must apply
  identically: `fixGuard.replicatedRuleGlobs`.
- A workflow that publishes or reads a secret: it must refuse the agent App as
  the author of the release or push that starts it. `repo.test.mjs` fails on
  the first such workflow to say so.

`agent-scripts.yml` runs both suites on a change to the harness, this
directory, `.github/actions/` or a workflow, and weekly. So a change to one of
the other files above that makes this directory wrong (the toolchain bumped in
`rust-toolchain.toml` alone, a Makefile recipe edited, a test module added in
a layout the scope check cannot follow) is found within a week, not on the
pull request that makes it.
