# yorkie-rust-sdk

Rust SDK for [Yorkie](https://github.com/yorkie-team/yorkie).

## Project layout

- `crates/yorkie-core`: local document model and CRDT implementation.
- `crates/yorkie-protocol`: generated protocol types and converters.
- `crates/yorkie-client`: async client, sync, watch, and channel support.
- `crates/yorkie`: public facade crate for SDK users.
