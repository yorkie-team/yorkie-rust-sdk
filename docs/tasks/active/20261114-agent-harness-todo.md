**Created**: 2026-11-14

# Onboard onto the shared agent harness

## Why

yorkie and yorkie-js-sdk run the `@claude` command surface (review panel, fix
loops, issue-to-PR) from one shared core written in yorkie. This repository is
the third adopter and the first Rust one. The core gained a `cargo` rule set
for it (yorkie#2352 to #2356); this task copies the core here and writes the
half that is this repository's.

## Plan

Repository prerequisites:

- [x] `Makefile` with `verify` = the four commands of `ci.yml`'s `check` job;
      the job calls the targets
- [x] `ci.yml` named exactly `CI` (the harness triggers on that name)
- [x] `CLAUDE.md`, `CONTRIBUTING.md`, `docs/tasks/`, `docs/design/` with a
      template and an index, `.github/PULL_REQUEST_TEMPLATE.md`: the six paths
      the issue-to-PR prompt names

The harness:

- [x] Copy the core with yorkie's `sync.mjs` (`scripts/agent/**`,
      `.github/workflows/agent-*.yml`, `upstream.json`); `.gitattributes`,
      `.gitignore`
- [x] `.github/agent-harness/config.json`: cargo test rules, CI-defining
      paths with guards for files that do not exist yet, design-spec extras,
      both counterpart repositories, the fixer sentences
- [x] `.github/agent-harness/coverage-note.md`
- [x] `.github/actions/agent-toolchain/action.yml`: pinned Rust through
      rustup, no cargo, no other action, no cache
- [x] `.github/agent-harness/repo.test.mjs`, mutation-checked, and
      `repo.snapshot.json`
- [x] `.github/agent-harness/README.md`

Verification:

- [x] `npm test`, `npm run test:repo`, `manifest.mjs verify` in `scripts/agent`
- [x] actionlint 1.7.12 over every workflow
- [x] `make verify` cold, in the `rust:1.99.1` image
- [x] The toolchain action's script in a clean `ubuntu:24.04` container
      against a hostile workspace
- [x] The scope hook on hand-made fix rounds

## Before merge

- [ ] yorkie#2352 to #2356 are merged, and the core is synced again from
      yorkie `main`. Until then `npm run check:upstream` fails by design: the
      recorded commit is a pull request's head.
- [ ] Move this pair to `docs/tasks/archive/2026/11/`.

## Open (for a maintainer, in GitHub settings)

Merging is inert: every agent job is gated on the repository variable
`AGENT_PIPELINE_ENABLED`. To switch it on: install the agent App, add its
secrets and the model token pool, create the `agent` environment and the
`agent:*` labels, then set the variable. The issue-to-PR verb also needs a
required approving review on `main`. The pull request body has the checklist.

## Not verified

- No workflow has run on a runner, and no fix round has run.
- `make verify` was timed on linux/arm64 in Docker, not on a GitHub runner.
