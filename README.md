# yorkie-rust-sdk

Rust SDK for [Yorkie](https://github.com/yorkie-team/yorkie), a document store
for collaborative applications. It is a port of the
[Yorkie JS SDK](https://github.com/yorkie-team/yorkie-js-sdk): the JS SDK is the
source of truth for behavior, names, and tests (see
[docs/porting-from-js-sdk.md](docs/porting-from-js-sdk.md)).

## Install

The SDK is not published to crates.io yet. Use it as a git dependency. The
client needs a tokio runtime.

```toml
[dependencies]
yorkie = { git = "https://github.com/yorkie-team/yorkie-rust-sdk" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quickstart

Two clients attach the same document, one edits, and both converge. Start a
server first with `yorkie server` (default RPC address `http://localhost:8080`).

```rust
use std::sync::{Arc, Mutex};
use yorkie::{AttachOptions, Client, ClientOptions, DeactivateOptions, Document, SyncMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manual = || AttachOptions { sync_mode: SyncMode::Manual, ..AttachOptions::default() };
    let options = || ClientOptions {
        rpc_addr: "http://localhost:8080".to_owned(),
        ..ClientOptions::default()
    };

    let alice = Client::new(options())?;
    let bob = Client::new(options())?;
    alice.activate().await?;
    bob.activate().await?;

    let doc_a = alice.attach(Arc::new(Mutex::new(Document::new("my-doc"))), manual()).await?;
    let doc_b = bob.attach(Arc::new(Mutex::new(Document::new("my-doc"))), manual()).await?;

    doc_a.lock().unwrap().update(|root| root.set("title", "Hello Yorkie").map(|_| ()))?;
    alice.sync(None).await?;
    bob.sync(None).await?;

    println!("{}", doc_b.lock().unwrap().to_sorted_json());

    alice.deactivate(DeactivateOptions::default()).await?;
    bob.deactivate(DeactivateOptions::default()).await?;
    Ok(())
}
```

Never hold the document lock across an `.await`. Document event callbacks run
while the client holds that lock, so a callback must not lock the same document.

Runnable examples live in `crates/yorkie/examples/`:

```sh
YORKIE_RPC_ADDR=http://localhost:8080 cargo run -p yorkie --example quickstart
YORKIE_RPC_ADDR=http://localhost:8080 cargo run -p yorkie --example realtime_presence
```

## Features

| Feature | Rust API | JS SDK |
| --- | --- | --- |
| Client lifecycle | `Client::{activate, deactivate, attach, detach, sync, remove}` | `Client` |
| Sync modes | `SyncMode::{Realtime, RealtimePushOnly, RealtimeSyncOff, Polling, Manual}`, `Client::change_sync_mode` | `SyncMode` |
| Realtime watch and sync loop | background tasks started by `activate` and `attach` | `runSyncLoop`, `runWatchLoop` |
| Presence | `AttachOptions::initial_presence`, `Document::update_with_presence`, `Document::presences` | `doc.presence`, `doc.getPresences` |
| Document events | `Document::{subscribe, subscribe_path, unsubscribe}`, `DocEvent` | `doc.subscribe` |
| JSON object, array, counter | `JsonObject`, `JsonArray`, `JsonCounter` | `JSONObject`, `JSONArray`, `Counter` |
| Text | `JsonText` (edit, delete, style, UTF-16 indexes) | `Text` |
| Tree | `JsonTree` (edit, style, split, merge, by path) | `Tree` |
| Undo and redo | `Document::{history, history_mut}` | `doc.history` |
| Schema and size limits | `AttachOptions::schema`, `SchemaRule`, `YorkieError::{SchemaValidationFailed, DocumentSizeExceedsLimit}` | `schema`, `maxSizePerDocument` |
| Channels and broadcast | `Client::{attach_channel, broadcast, peek_channel}`, `Channel` | `Channel`, `client.broadcast` |
| Auth token injector | `ClientOptions::auth_token_injector` | `authTokenInjector` |
| Error codes | `ErrorCode`, `YorkieError::code`, `ClientError::code` | `Code` |

## Project layout

- `crates/yorkie-core`: document model and CRDTs, no network dependencies.
- `crates/yorkie-protocol`: generated protobuf types and converters.
- `crates/yorkie-client`: async client, Connect transport, sync, watch, channels.
- `crates/yorkie`: public facade crate (re-exports only), examples, and
  integration tests.

## Tests

Unit tests need nothing else. Integration tests skip themselves (and pass)
unless `YORKIE_TEST_RPC_ADDR` is set.

```sh
cargo test --workspace
```

To run the integration tests, start a local server (in-memory DB) and point the
tests at it:

```sh
yorkie server --rpc-port 18080 --profiling-port 18081 &
YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie
```

CI runs fmt, clippy, the unit tests, and the integration tests against the
`yorkieteam/yorkie` image (see `.github/workflows/ci.yml`).

## Known gaps

Offline persistence, devtools, the revisions API, Tree undo/redo, and a few
other JS features are not ported. See
[docs/current-porting-gaps.md](docs/current-porting-gaps.md) for the full list
with JS source paths.

**Interop limitation: undo in mixed JS and Rust sessions.** The JS SDK undoes
Text edits, and Tree edits, by node identity: the operation it sends carries
`restore_spans`, `retombstone_spans`, and `restore_mode`. The Rust SDK does
not port these. Its decoder drops them and runs only the positional part of
the operation (for a Text undo, an empty edit of the original range), so the
content a JS undo brings back never reappears on a Rust replica, and the
documents diverge. Avoid mixing JS and Rust clients on a document where JS
users undo Text or Tree edits. A Rust undo is sent as a plain positional
edit, which every peer applies alike.

## License

Apache-2.0.
