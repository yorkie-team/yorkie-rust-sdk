## What the mechanical lanes cover on this branch

These run on this PR and must pass before it can be promoted. A finding whose
whole content is "this would fail CI" costs a review round and tells the author
nothing they were not about to be told anyway.

ENFORCED — you may rely on these:
- `cargo fmt --all --check` on Rust 1.99.1: the formatting of every crate in
  the workspace, with rustfmt's defaults (there is no rustfmt.toml).
- `cargo clippy --workspace --all-targets -- -D warnings`: clippy's default
  set and every rustc warning, as errors, over library, test and example code.
  Unused code, unused results and needless clones are covered here. Every
  crate root declares `#![forbid(unsafe_code)]`, so `unsafe` does not compile.
- `cargo test --workspace`: every unit test under `src`, every integration
  target under `crates/*/tests` and `tools/*/tests`, and every doc-test. That
  includes `protogen_output_is_up_to_date`, which regenerates the protobuf
  code and fails when `crates/yorkie-protocol/src/generated/yorkie.v1.rs`
  differs, so a hand edit or a stale file reds the lane, and the interop tests
  that decode fixtures written by the JS SDK and the Go server.
- `cargo doc -p yorkie --no-deps` with `RUSTDOCFLAGS=-D warnings`: broken
  intra-doc links and malformed doc comments in the facade crate.
- The `integration tests` lane: `cargo test -p yorkie` with
  `YORKIE_TEST_RPC_ADDR` set, against `yorkieteam/yorkie:0.7.26` with its
  in-memory database, then a run of the `quickstart` example against the same
  server.
- No path filter: all of the above runs on every PR, documentation-only ones
  included.

NOT ENFORCED BY ANYTHING — a real finding here is worth MORE than one the lanes
above would have caught, because nothing else in the pipeline will catch it:
- That a test ran at all. Each test in `integration_*.rs` that needs a server
  returns early, and passes, when `YORKIE_TEST_RPC_ADDR` is unset, and the
  `cargo test --workspace` lane runs them that way. A test that returns before
  its assertions is green in both lanes.
- Any test marked `#[ignore]`. No lane passes `--ignored`, so an ignored
  reproducer is never executed.
- Anything that needs a MongoDB-backed server, or any server but
  `yorkieteam/yorkie:0.7.26`.
- A lint silenced in the diff: `#[allow(...)]` passes `-D warnings`.
- Rustdoc for any crate but `yorkie`.
- Any Rust but 1.99.1, any platform but Linux x86_64, and any feature set but
  the default.
- Dependency audit, licence headers, unused dependencies, semver checks, Miri,
  sanitizers and coverage: none is configured.
- Parity with the JS SDK and the Go server beyond the checked-in fixtures.
  `docs/crdt-parity/**` and `docs/current-porting-gaps.md` are maintained by
  hand, and `tools/interop/` is run by hand in the other repositories.
- Whether a passing test asserts anything. The lanes prove the suite is GREEN,
  never that it is ADEQUATE — a test that asserts nothing passes just as loudly.
