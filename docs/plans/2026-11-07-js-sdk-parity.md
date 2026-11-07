# JS SDK Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Rust SDK from "CRDT core + transport boundary" to a usable
SDK comparable to the JS SDK: real network sync against a Yorkie server,
realtime watch, presence, document events, public Text/Tree, undo/redo,
schema/size validation, broadcast, and channels.

**Architecture:** `yorkie-core` stays runtime-free and gains presence, events,
public Text/Tree facades, history, and validation. `yorkie-client` gains a
Connect-protocol transport (reqwest), a `Client` that owns its transport and
shares attached documents as `Arc<std::sync::Mutex<Document>>`, and tokio tasks
for the sync loop and watch streams. Every behavior is ported from a named JS
SDK test or source section, cross-checked against Go where CRDT internals are
involved, following `docs/porting-from-js-sdk.md`.

**Tech Stack:** Rust 1.99 (edition 2021), prost 0.14, tokio 1, reqwest 0.12
(rustls, no default features), async-trait, serde_json, a local `yorkie server`
(memdb) for integration tests.

**Spec:** this document plus `docs/current-porting-gaps.md` (the gap list it
closes) and `docs/porting-from-js-sdk.md` (the porting contract). JS SDK at
`../yorkie-js-sdk` commit `8629993e`, Yorkie server/protos at `../yorkie`
commit `54b96762`.

## Global Constraints

- JS SDK is the behavioral source of truth; Go is a typed cross-check. Every
  feature starts from a JS test or JS source section, recorded in the test
  module doc or `docs/`.
- `yorkie-core` must not depend on tokio, reqwest, prost, or any network crate.
  Allowed new core dependency: `serde_json` (presence values and attribute
  values only).
- Crate direction stays `core <- protocol <- client <- yorkie` (see
  `docs/dependency-boundaries.md`).
- Rust names map visibly to JS names (`to_sorted_json` ↔ `toSortedJSON`).
  Every new public type gets a row in the concept map in
  `docs/porting-from-js-sdk.md` and its gap entry updated in
  `docs/current-porting-gaps.md`.
- `#![forbid(unsafe_code)]` stays on all crates.
- Every task ends with `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` passing.
- Integration tests live in `crates/yorkie/tests/integration_*.rs`, read the
  server address from `YORKIE_TEST_RPC_ADDR`, and return early (pass with an
  `eprintln!` skip notice) when it is unset. Run them with
  `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`. Start the
  server with `yorkie server --rpc-port 18080 --profiling-port 18081`.
- Each integration test uses unique document keys
  (`<test-name>-<unix-millis>`) so tests can run in parallel on one server.
- Commits are English, subject ≤70 chars starting with an imperative verb, no
  type prefix; body lines ≤80 chars and explain why.
- Out of scope for this plan (documented as gaps, not implemented): offline
  persistence (doc-store, session-lock), devtools, revisions API, epoch
  reanchor, identity-preserving undo restore spans, disable-GC attach option,
  Tree undo/redo.

## Review Focus

1. A server restart or dropped watch stream mid-session: the client must
   reconnect after `reconnect_stream_delay` and resume sync without losing
   local changes (Task 6 test `watch_reconnects_after_stream_error`).
2. Concurrent edits from two realtime clients to the same Text/Tree range must
   converge to identical `to_sorted_json` (Tasks 8 and 9 integration tests).
3. Non-ASCII text (Korean, emoji surrogate pairs) in Text edits and Tree text
   nodes: indexes are UTF-16 code units like JS
   (Task 8 test `text_edit_uses_utf16_indexes`).
4. An update that fails validation (schema or max size) must leave the
   document and its pending changes untouched
   (Task 12 test `rejected_update_rolls_back`).
5. Deactivating a client with attached documents must move every document to
   `Detached` and stop all background tasks
   (Task 5 test `deactivate_detaches_documents_and_stops_tasks`).

---

## Milestone 1: Network foundation

### Task 1: Refresh vendored protos and add a regeneration tool

**Files:**
- Modify: `crates/yorkie-protocol/proto/yorkie/v1/{resources,yorkie}.proto`
  (copy from `../yorkie/api/yorkie/v1/`)
- Create: `tools/protogen/` (workspace member, `publish = false`) using
  `prost-build` + `protoc-bin-vendored`, writing
  `crates/yorkie-protocol/src/generated/yorkie.v1.rs`
- Modify: `crates/yorkie-protocol/src/generated/yorkie.v1.rs` (regenerated)
- Modify: `crates/yorkie-protocol/src/converter.rs`, `crates/yorkie-client/src/protocol.rs`
  (compile fixes for new fields only; new fields stay default)

**Interfaces:**
- Produces: generated types including `PeekChannel*`, `WatchHeartbeat`,
  `ActivateClientResponse.actor_id`, `ChangePack.epoch`.

- [ ] Copy protos, run `cargo run -p protogen`, confirm `git diff --stat`
  only touches proto, generated, and compile-fix files.
- [ ] Add test `protogen_output_is_up_to_date` (in `tools/protogen/tests/`)
  that regenerates into a temp dir and asserts byte equality with the
  checked-in file.
- [ ] Verification gate (fmt/clippy/test). Commit
  "Refresh vendored protos to Yorkie 54b96762".

### Task 2: Connect transport and integration harness

**Files:**
- Create: `crates/yorkie-client/src/connect.rs` — `ConnectTransport`
- Modify: `crates/yorkie-client/src/transport.rs` — trait methods take `&self`,
  trait becomes `#[async_trait] pub trait ClientTransport: Send + Sync`
- Modify: `crates/yorkie-client/src/client.rs` (call sites), `error.rs`
- Create: `crates/yorkie/tests/integration_support/mod.rs` —
  `fn rpc_addr() -> Option<String>`, `fn unique_key(prefix: &str) -> String`
- Create: `crates/yorkie/tests/integration_client.rs`

**Interfaces:**
- Produces:
  - `pub struct ConnectTransport` with
    `ConnectTransport::new(options: &ClientOptions) -> ClientResult<Self>`.
  - Unary calls POST `{rpc_addr}/yorkie.v1.YorkieService/{Method}` with
    `content-type: application/proto`, `connect-protocol-version: 1`, headers
    `x-api-key` (when `api_key` set), `x-shard-key` (from the request DTO),
    `x-yorkie-user-agent: yorkie-rust-sdk/{CARGO_PKG_VERSION}`.
  - Non-200 replies decode the Connect JSON error body into
    `ClientError::Rpc { code: String, message: String }` (code is the Connect
    code string, e.g. `"not_found"`, `"failed_precondition"`).
  - `ClientTransport::watch_document(&self, req) -> ClientResult<WatchStream>`
    is added in Task 7, not here.

- [ ] Test `connect_error_body_maps_to_rpc_error` (unit, no network): feed
  `{"code":"not_found","message":"document not found"}` to the decoder and
  assert `ClientError::Rpc { code: "not_found", .. }`.
- [ ] Integration test `activate_attach_sync_detach_roundtrip`: client A
  activates, attaches doc K, sets `k1 = "v1"`, syncs, detaches; client B
  attaches K and asserts `to_sorted_json() == r#"{"k1":"v1"}"#`; both
  deactivate.
- [ ] Integration test `attach_twice_fails`: second attach of the same key on
  one client returns the existing `ClientError` for already-attached.
- [ ] Implement `ConnectTransport` with reqwest
  (`default-features = false, features = ["rustls-tls", "http2"]`).
- [ ] Verification gate plus integration run against the local server.
  Commit "Add Connect transport for unary RPCs".

## Milestone 2: Realtime client

### Task 3: Presence model and presence changes

JS refs: `src/document/presence/{presence,change}.ts`, `document.ts`
(`getMyPresence`, `getPresence`, `getOthersPresences`, `getPresences`),
`api/converter.ts` (`toPresenceChange`, `fromPresence`). Go cross-check:
`pkg/document/presence/`.

**Files:**
- Create: `crates/yorkie-core/src/presence.rs`
- Modify: `crates/yorkie-core/src/{document.rs,change/change.rs,change/pack.rs,lib.rs,wire.rs}`
- Modify: `crates/yorkie-protocol/src/converter.rs` (presence change, snapshot
  presences)

**Interfaces:**
- Produces:
  - `pub type Presence = serde_json::Map<String, serde_json::Value>;`
  - `pub enum PresenceChange { Put(Presence), Clear }` on `Change`
    (`Change::presence_change() -> Option<&PresenceChange>`).
  - `pub struct PresenceProxy` with `set(&mut self, Presence)` (merges keys,
    JS `presence.set`) and `get(&self, key) -> Option<&Value>`.
  - `Document::update_with_presence<F>(&mut self, f: F) -> Result<()>` where
    `F: FnOnce(&mut JsonObject, &mut PresenceProxy) -> Result<()>`;
    existing `update` delegates to it.
  - `Document::{my_presence, presence(&ActorId), others_presences,
    presences}` returning owned/borrowed `Presence`; only online actors (plus
    self) are returned, as in JS.
  - `Document::{set_online_clients, add_online_client, remove_online_client,
    has_presence}` (`pub(crate)`-style visibility is not possible across
    crates; mark `#[doc(hidden)] pub`).
  - Wire: proto `Presence.data` is `map<string,string>`, each value the JSON
    encoding of the presence value (JS `JSON.stringify`).

- [ ] Port `test/unit/document/presence_test.ts`-style cases: set presence in
  update, `my_presence` reflects it, `to_sorted_json` unaffected.
- [ ] Test `presence_change_roundtrips_through_change_pack`: encode → decode
  `ChangePack` keeps `PresenceChange::Put` with string `{"name":"\"a\""}` on
  the wire.
- [ ] Test `snapshot_restores_presences`.
- [ ] Verification gate. Commit "Add document presence".

### Task 4: Document events and subscriptions

JS refs: `document.ts` `DocEventType`, `subscribe` overloads, `publish`;
`test/unit/document/document_test.ts` subscribe cases.

**Files:**
- Create: `crates/yorkie-core/src/event.rs`
- Modify: `crates/yorkie-core/src/{document.rs,operation/mod.rs,lib.rs}`

**Interfaces:**
- Produces:
  - `pub enum DocEvent { StatusChanged(DocStatus), ConnectionChanged(StreamConnectionStatus),
    SyncStatusChanged(DocSyncStatus), Snapshot, LocalChange(ChangeInfo),
    RemoteChange(ChangeInfo), Initialized(Vec<(ActorId, Presence)>),
    Watched(ActorId, Presence), Unwatched(ActorId, Presence),
    PresenceChanged(ActorId, Presence), Broadcast { actor: ActorId, topic: String, payload: serde_json::Value } }`
  - `pub struct ChangeInfo { pub actor: ActorId, pub message: Option<String>, pub operations: Vec<OperationInfo> }`
  - `pub enum OperationInfo` — public JS-shaped op infos with `path: String`:
    `Set{key}`, `Remove{key: Option<String>, index: Option<usize>}` (JS has one
    `remove` with key or index — collapse Rust `ArrayRemove` into it), `Add{index}`,
    `Move{index, previous_index}`, `Increase{value}`, `Edit{from,to,value}`,
    `Style{from,to,value}`, `TreeEdit{..}`, `TreeStyle{..}`. Internal `OpInfo`
    converts into it.
  - `Document::subscribe(&mut self, f: impl FnMut(&DocEvent) + Send + 'static) -> SubscriptionId`
  - `Document::subscribe_path(&mut self, path: &str, f) -> SubscriptionId` —
    delivers only Local/RemoteChange whose op paths equal or descend from
    `path` (JS `subscribe('$.todos', cb)`), filtering `operations`.
  - `Document::unsubscribe(&mut self, id: SubscriptionId)`
  - `Document::publish(&mut self, event: DocEvent)` (`#[doc(hidden)]`, used by
    the client crate).
- Emission points in core: `update` → `LocalChange`; `apply_change_pack`
  remote changes → `RemoteChange`, snapshot → `Snapshot`; `apply_status` →
  `StatusChanged`; presence-only change → `PresenceChanged` for remote actors.

- [ ] Tests `local_change_event_carries_set_and_remove_ops`,
  `remote_change_event_after_apply_change_pack`,
  `subscribe_path_filters_operations` (`$.todos` sees `$.todos` and
  `$.todos.0` paths, not `$.other`), `unsubscribe_stops_delivery`,
  `status_changed_on_apply_status`.
- [ ] Verification gate. Commit "Add document event subscriptions".

### Task 5: Client owns transport and shares documents

JS refs: `client.ts` `activate`, `attach`, `detach`, `sync`, `deactivate`;
`test/integration/client_test.ts`.

**Files:**
- Modify: `crates/yorkie-client/src/{client.rs,attachment.rs,options.rs,lib.rs}`,
  `crates/yorkie/src/lib.rs`
- Modify: `crates/yorkie/tests/integration_client.rs`

**Interfaces:**
- Consumes: `ConnectTransport` (Task 2), presence (Task 3), events (Task 4).
- Produces:
  - `pub type SharedDocument = Arc<std::sync::Mutex<Document>>;`
  - `#[derive(Clone)] pub struct Client` (internally `Arc<ClientInner>`);
    `Client::new(options) -> ClientResult<Client>` builds a
    `ConnectTransport`; `Client::with_transport(options, Arc<dyn ClientTransport>) -> Client`
    for tests.
  - `&self` async methods: `activate()`, `deactivate(DeactivateOptions)`,
    `attach(doc: SharedDocument, AttachOptions) -> ClientResult<SharedDocument>`,
    `detach(&SharedDocument, DetachOptions)`, `sync(Option<&SharedDocument>)`
    (None syncs all attached, JS `sync()`), `remove(&SharedDocument)`,
    `change_sync_mode(&SharedDocument, SyncMode)`.
  - `AttachOptions { initial_presence: Presence, sync_mode: SyncMode, schema: Option<String> }`.
  - The document mutex is never held across an `.await`.
  - Actor: use `ActivateClientResponse.actor_id` when non-empty, else
    `client_id` (JS `this.actorID = res.actorId || res.clientId`).
- Existing mock-transport unit tests are kept by moving them to
  `with_transport` with a `MockTransport` implementing the new trait.

- [ ] Integration test `two_clients_converge_with_manual_sync` (A sets,
  sync; B syncs; equal JSON; then B edits, both sync, equal JSON).
- [ ] Integration test `attach_with_initial_presence` (B sees A's presence
  after both sync in manual mode).
- [ ] Integration test `deactivate_detaches_documents_and_stops_tasks`
  (Review Focus 5): after deactivate, every attached doc status is
  `Detached` and `client.is_active() == false`.
- [ ] Verification gate. Commit "Let Client own its transport and documents".

### Task 6: Watch stream and realtime sync loop

JS refs: `client.ts` `runSyncLoop`, `runWatchLoop`, `handleWatchStream`,
`handleWatchDocumentResponse`; `attachment.ts`; `watch.ts`;
`test/integration/client_test.ts` realtime cases.

**Files:**
- Modify: `crates/yorkie-client/src/connect.rs` — server-streaming call:
  POST with `content-type: application/connect+proto`, request and response
  bodies use Connect envelopes (1 flag byte, 4-byte big-endian length);
  flag `0x02` marks the end-stream JSON message carrying an optional error.
- Modify: `crates/yorkie-client/src/{transport.rs,client.rs,attachment.rs}`
- Create: `crates/yorkie-client/src/watch.rs`, `crates/yorkie-client/src/sync_loop.rs`

**Interfaces:**
- Produces:
  - `ClientTransport::watch_document(&self, WatchDocumentRequest) -> ClientResult<WatchStream>`
    where `pub type WatchStream = Pin<Box<dyn Stream<Item = ClientResult<WatchDocumentResponse>> + Send>>`.
  - Sync loop: one tokio task per active client, ticking every
    `sync_loop_duration_ms`, syncing attachments that are realtime and either
    have local changes or a pending remote-change notice; on error waits
    `retry_sync_loop_delay_ms`.
  - Watch loop: one task per realtime attachment; on stream error publishes
    `ConnectionChanged(Disconnected)` and reconnects after
    `reconnect_stream_delay_ms`; sends `heartbeat_interval_ms` from options.
  - Handles: `WatchInitialization` → `set_online_clients` + `Initialized`;
    `DocEvent DOCUMENT_CHANGED` → mark sync needed; `DOCUMENT_WATCHED` /
    `UNWATCHED` → online-client bookkeeping + `Watched`/`Unwatched`;
    `DOCUMENT_BROADCAST` handled in Task 7.
  - `SyncMode::{Realtime, RealtimePushOnly, RealtimeSyncOff, Manual}` follow
    JS semantics; `SyncStatusChanged(Synced | SyncFailed)` published per sync.

- [ ] Unit test `connect_envelope_roundtrip` (encode/decode, end-stream
  error JSON → `ClientError::Rpc`).
- [ ] Integration test `realtime_clients_converge_without_manual_sync`
  (poll up to 5 s for equal JSON).
- [ ] Integration test `watched_and_unwatched_events` (A observes B's
  `Watched` with B's initial presence, then `Unwatched` after B detaches).
- [ ] Integration test `presence_changed_event_in_realtime`.
- [ ] Integration test `watch_reconnects_after_stream_error` (Review Focus 1):
  use a test transport wrapper that fails the first watch stream after one
  message; assert a second stream is opened and convergence still happens.
- [ ] Integration test `change_sync_mode_manual_stops_auto_sync`.
- [ ] Verification gate. Commit "Add watch stream and realtime sync loop".

### Task 7: Document broadcast

JS refs: `document.ts` `broadcast`, `client.ts` `broadcast`,
`test/integration/broadcast_test.ts`.

**Files:**
- Modify: `crates/yorkie-client/src/{client.rs,transport.rs,connect.rs,protocol.rs,watch.rs}`
- Modify: `crates/yorkie-core/src/document.rs`

**Interfaces:**
- Produces: `Client::broadcast(&self, doc: &SharedDocument, topic: &str, payload: serde_json::Value) -> ClientResult<()>`
  using the `Broadcast` RPC; incoming `DOCUMENT_BROADCAST` publishes
  `DocEvent::Broadcast`. Payload bytes are the JSON encoding (JS
  `JSON.stringify`).

- [ ] Integration tests `broadcast_reaches_other_client` and
  `broadcast_is_not_echoed_to_sender`; unit test that a non-JSON-encodable
  payload cannot be constructed is unnecessary (serde_json::Value is always
  encodable).
- [ ] Verification gate. Commit "Add document broadcast".

## Milestone 3: Public data types

### Task 8: Public Text facade

JS refs: `src/document/json/text.ts`, `test/unit/document/crdt/text_test.ts`,
`test/integration/text_test.ts`. Go cross-check: `pkg/document/json/text.go`.

**Files:**
- Create: `crates/yorkie-core/src/json/text.rs` (convert `json.rs` into
  `json/mod.rs` only if needed to keep files focused)
- Modify: `crates/yorkie-core/src/{json.rs,document.rs,operation/edit_operation.rs,operation/style_operation.rs}`

**Interfaces:**
- Produces:
  - `pub struct JsonText` (JS `Text`), `JsonValue::Text(JsonText)`.
  - Creation: `JsonObject::set_text(key) -> Result<&mut JsonText>`,
    `JsonArray::push_text() -> Result<&mut JsonText>`;
    lookup `JsonObject::get_text_mut(key)`.
  - Edit: `edit(from, to, content: &str, attrs: Option<Attrs>) -> Result<(usize, usize)>`,
    `delete(from, to)`, `set_style(from, to, attrs: Attrs)`,
    `to_string()`, `len()` (UTF-16 units), `values() -> Vec<TextChunk>`.
    `Attrs = BTreeMap<String, serde_json::Value>`, stringified per JS before
    entering the CRDT.
  - `EditOperation` op info emits the JS value-change list (one entry per
    affected range), mapped to `OperationInfo::Edit`.

- [ ] Port JS text unit cases: edit/delete/insert sequences,
  `set_style` and attribute JSON output, `to_sorted_json`.
- [ ] Test `text_edit_uses_utf16_indexes` (Review Focus 3): insert "안녕😀" then
  edit `(2, 4, "")` removes the emoji (2 UTF-16 units); `len()` == 2.
- [ ] Integration tests from `test/integration/text_test.ts`: concurrent
  insert at same position, concurrent delete-overlap, concurrent style;
  both clients converge.
- [ ] Verification gate. Commit "Add public Text facade".

### Task 9: Public Tree facade (insert, delete, style)

JS refs: `src/document/json/tree.ts`, `test/integration/tree_test.ts`
(non-split sections). Go cross-check: `pkg/document/json/tree.go`.

**Files:**
- Create: `crates/yorkie-core/src/json/tree.rs`
- Modify: `crates/yorkie-core/src/{json.rs,document.rs,crdt/tree.rs,operation/tree_*_operation.rs}`

**Interfaces:**
- Produces:
  - `pub struct JsonTree` (JS `Tree`), `JsonValue::Tree(JsonTree)`.
  - Input nodes: `pub enum TreeNodeInput { Element { node_type: String, attributes: Attrs, children: Vec<TreeNodeInput> }, Text { value: String } }`
    (JS `ElementNode`/`TextNode`).
  - Creation: `JsonObject::set_tree(key, root: TreeNodeInput)`,
    lookup `get_tree_mut(key)`.
  - Methods: `edit(from, to, contents: Vec<TreeNodeInput>, split_level: usize)`
    (split_level > 0 returns an error until Task 10), `edit_by_path`,
    `style(from, to, attrs)`, `remove_style(from, to, keys)`, `to_xml()`,
    `size()`, `index_to_path`, `path_to_index`, `to_json()`.

- [ ] Port JS tree tests for created-tree XML, insert text/element, delete
  ranges, `edit_by_path`, style/remove_style, and their op-info events.
- [ ] Integration tests: concurrent insert at same/different positions and
  concurrent style converge (subset of `tree_concurrency_test.ts`
  without split/merge).
- [ ] Verification gate. Commit "Add public Tree facade".

### Task 10: Tree split and merge through the facade

JS refs: `tree_test.ts` split sections, `tree_concurrent_split_merge_test.ts`,
`tree_merge_anchor_test.ts`, `tree_split_opinfo_test.ts`.

**Files:**
- Modify: `crates/yorkie-core/src/{json/tree.rs,crdt/tree.rs,operation/tree_edit_operation.rs}`,
  `crates/yorkie-protocol/src/converter.rs` (`split_tickets`)

**Interfaces:**
- Produces: `JsonTree::edit` accepts `split_level > 0`; merge via deleting a
  boundary range; tree edit op info for split carries `split_level`.
  `TreeEdit` wire carries `split_tickets` as upstream does.

- [ ] Port JS split/merge unit cases and `tree_split_opinfo_test.ts`.
- [ ] Integration tests from `tree_concurrent_split_merge_test.ts`; known
  upstream-skipped cases stay `#[ignore]` with a reference to
  `docs/crdt-parity/upstream-skipped-tests.md`.
- [ ] Verification gate. Commit "Support Tree split and merge edits".

## Milestone 4: History and validation

### Task 11: Undo and redo

JS refs: `src/document/history.ts`, `document.ts` `history` getters and
`undo`/`redo`, `test/unit/document/history_test.ts`,
`test/integration/history_array_test.ts`, `history_text_test.ts`
(non-restore-span cases).

**Files:**
- Create: `crates/yorkie-core/src/history.rs`
- Modify: `crates/yorkie-core/src/{document.rs,change/change.rs}`

**Interfaces:**
- Produces: `Document::history() -> HistoryRef` with `can_undo()`,
  `can_redo()`, `undo() -> Result<()>`, `redo() -> Result<()>`; max depth 50
  (JS `MaxUndoRedoStackDepth`); remote changes reconcile/clear per JS;
  presence restored on undo like JS `HistoryOperation` presence entries.
  Tree undo is out of scope and returns no reverse ops (documented gap).

- [ ] Port JS history unit tests for object set/remove, array
  add/remove/move, counter increase, text edit/style, presence undo.
- [ ] Integration test from `history_array_test.ts` (two clients, undo
  after remote change converges).
- [ ] Verification gate. Commit "Add document undo and redo".

### Task 12: Schema and max-size validation

JS refs: `document.ts` `update` validation block,
`src/schema/ruleset_validator.ts`, `test/integration/document_schema_test.ts`,
`document_limit_test.ts`.

**Files:**
- Modify: `crates/yorkie-core/src/{document.rs,schema.rs,error.rs}`

**Interfaces:**
- Produces: `YorkieError::SchemaValidationFailed(Vec<String>)` and
  `YorkieError::DocumentSizeExceedsLimit { size: usize, limit: usize }` with
  JS messages; validation runs after the updater against the cloned root and
  before commit.

- [ ] Test `rejected_update_rolls_back` (Review Focus 4): with a rule
  `$.title: string`, setting `title = 1` errors and leaves JSON, pending
  changes, and history unchanged; same for exceeding max size.
- [ ] Integration tests from `document_schema_test.ts` (attach with schema
  key created through the admin API is out of scope; use rules set via
  `set_schema_rules`) and `document_limit_test.ts`.
- [ ] Verification gate. Commit "Validate updates against schema and size".

## Milestone 5: Channels and hardening

### Task 13: Channels

JS refs: `src/channel/channel.ts`, `client.ts` channel attach/detach/refresh/
watch, `test/integration/channel_test.ts`.

**Files:**
- Create: `crates/yorkie-client/src/channel.rs`
- Modify: `crates/yorkie-client/src/{client.rs,transport.rs,connect.rs,protocol.rs}`

**Interfaces:**
- Produces: `pub struct Channel` (key, session count, status, subscribe for
  `PresenceChanged`/`Broadcast`), `Client::attach_channel(Channel, AttachChannelOptions) -> ClientResult<SharedChannel>`,
  `detach_channel`, `Client::broadcast_channel`; heartbeat refresh every
  `channel_heartbeat_interval_ms`.

- [ ] Integration tests from `channel_test.ts`: count increases when two
  clients attach, decreases after detach; broadcast through channel.
- [ ] Verification gate. Commit "Add channels".

### Task 14: Auth token and error codes

JS refs: `client.ts` `authTokenInjector`, `auth_interceptor.ts`,
`util/error.ts` `Code`, `test/integration/webhook_test.ts` (shape only).

**Files:**
- Modify: `crates/yorkie-client/src/{options.rs,connect.rs,error.rs,client.rs}`,
  `crates/yorkie-core/src/error.rs`

**Interfaces:**
- Produces: `ClientOptions::auth_token_injector: Option<Arc<dyn Fn(Option<String>) -> BoxFuture<'static, String> + Send + Sync>>`;
  `authorization` header; on `unauthenticated` with reason `"token expired"`
  re-inject once and retry (JS behavior); `ErrorCode` enum with JS code
  strings and `ClientError::code()`/`YorkieError::code()`; `DocEvent::AuthError`.

- [ ] Unit tests with a mock transport for retry-once-on-expired and
  error-code mapping.
- [ ] Verification gate. Commit "Add auth token injector and error codes".

### Task 15: Facade, examples, docs, CI

**Files:**
- Modify: `crates/yorkie/src/lib.rs`, `README.md`,
  `docs/current-porting-gaps.md`, `docs/porting-from-js-sdk.md`,
  `docs/dependency-boundaries.md`, `docs/README.md`
- Create: `crates/yorkie/examples/{quickstart,realtime_presence}.rs`,
  `.github/workflows/ci.yml` (fmt, clippy, unit tests; integration job
  builds `yorkie` from a pinned release and runs it in the background)

- [ ] `cargo run -p yorkie --example quickstart` against the local server
  prints the converged JSON.
- [ ] Gap doc reflects the new state: every closed gap removed, remaining
  out-of-scope items listed with their JS source paths.
- [ ] Verification gate. Commit "Document the SDK surface and add CI".
