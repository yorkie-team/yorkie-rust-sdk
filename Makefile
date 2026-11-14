# The local gate. `make verify` runs exactly what the `check` job of
# .github/workflows/ci.yml runs, because that job calls these targets. None of
# them needs a Yorkie server: the integration tests under crates/yorkie/tests
# return early (and pass) unless YORKIE_TEST_RPC_ADDR is set, and CI's
# `integration` job is where they run against one.

.PHONY: verify fmt-check clippy test doc

verify: fmt-check clippy test doc

fmt-check:
	cargo fmt --all --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc -p yorkie --no-deps
