# Porting from the JS SDK

The Rust SDK is a port of the Yorkie JS SDK. The JS SDK is the source of truth
for SDK behavior, terminology, state transitions, and test expectations.

## Source of Truth

Use the following order when deciding how a concept should work:

1. JS SDK tests for observable behavior.
2. JS SDK implementation for field names, state flow, and event semantics.
3. Go SDK/client implementation for cross-checking typed SDK flow, CRDT
   internals, and edge cases.
4. Yorkie protobuf files for wire format.
5. Go server internals for algorithmic background only.

In the local development workspace, the main references are:

- JS SDK public facade: `../yorkie-js-sdk/packages/sdk/src/yorkie.ts`
- JS SDK source: `../yorkie-js-sdk/packages/sdk/src/`
- JS SDK tests: `../yorkie-js-sdk/packages/sdk/test/`
- Go SDK/client source: `../yorkie/client/`
- Go SDK document source: `../yorkie/pkg/document/`
- Go SDK document tests: `../yorkie/pkg/document/**/*_test.go`
- Yorkie proto files: `../yorkie/api/yorkie/v1/`
- Yorkie design docs: `../yorkie/docs/design/`

## Non-Negotiable Porting Rules

- Do not invent Rust-only domain concepts unless the concept is explicitly
  documented as an implementation detail and mapped to the JS SDK concept it
  supports.
- Do not rename domain fields casually. If Rust uses idiomatic snake_case, the
  corresponding JS name must still be obvious in docs, comments, tests, or type
  names.
- Do not change the lifecycle flow. `Client`, `Document`, `Change`,
  `ChangePack`, `Operation`, `TimeTicket`, `VersionVector`, presence, sync
  modes, watch streams, and document statuses must follow the JS SDK model.
- Do not use the Go client as the behavioral source of truth when it differs
  from the JS SDK. Go is useful for CRDT internals and server-side context, but
  JS defines the SDK behavior we are porting.
- Cross-check the Go SDK/client when the JS implementation relies on
  JavaScript-specific runtime behavior, proxies, reference sharing, or dynamic
  typing. Use Go to clarify how the same concept works in a typed SDK, then
  keep the Rust behavior aligned with JS.
- Public Rust APIs may be idiomatic Rust, but their semantics must match the JS
  SDK. For example, a Rust method may be named `to_sorted_json`, but it maps to
  JS `toSortedJSON()`.
- Every ported feature must start from a JS SDK test or a JS SDK source section.
  If no JS test exists, add the Rust test from the JS source behavior and record
  the source path in the test or nearby module docs.
- Do not treat skipped JS/Go CRDT tests as pass targets. If they are represented
  in Rust, keep them ignored and reference
  `docs/crdt-parity/upstream-skipped-tests.md`.

## Porting Workflow

1. Pick one JS SDK test file or one small behavior inside a test file.
2. Add a Rust test that states the same observable behavior.
3. If the source test is skipped upstream, document it and keep the Rust test
   ignored instead of implementing ahead of upstream behavior.
4. Keep source references in docs and planning notes, not in public code
   comments unless they are necessary for maintainability.
5. Cross-check the corresponding Go SDK/client implementation when the feature
   touches CRDT internals, operation application, typed API flow, ownership-like
   structure, or error handling.
6. Implement the smallest Rust code needed to pass the test.
7. Keep the Rust names mapped to JS names. Add a mapping note when the Rust name
   differs for idiomatic reasons.
8. Run `cargo fmt` and `cargo test`.

Prefer vertical slices over broad scaffolding. A small feature is done only when
it has:

- a JS SDK source/test reference,
- a Rust test,
- matching observable behavior,
- no undocumented Rust-only concepts.

## Current Concept Map

| Rust concept | JS SDK source of truth | Notes |
| --- | --- | --- |
| `yorkie` facade crate | `packages/sdk/src/yorkie.ts` | Public entrypoint for SDK users. |
| `Document` | `packages/sdk/src/document/document.ts` | Rust `Document::new` maps to JS `new Document(key)` and now carries document status and actor assignment. |
| `DocStatus` | `packages/sdk/src/document/document.ts` | Document lifecycle status: detached, attached, and removed. |
| `Document::to_sorted_json` | `Document.toSortedJSON()` | Rust naming is idiomatic; behavior must match JS. |
| `ActorId` | `packages/sdk/src/document/time/actor_id.ts` | Rust newtype for the actor identifier string. |
| `TimeTicket` | `packages/sdk/src/document/time/ticket.ts` | Logical timestamp ordered by lamport, actor, then delimiter. |
| `VersionVector` | `packages/sdk/src/document/time/version_vector.ts` | Actor-to-lamport vector used for causal visibility checks. |
| `ChangeId` | `packages/sdk/src/document/change/change_id.ts` | Change identifier with client sequence, server sequence, actor, lamport, and version vector. |
| `Checkpoint` | `packages/sdk/src/document/change/checkpoint.ts` | Tracks sent and received client/server sequences for sync. |
| `ChangeContext` | `packages/sdk/src/document/change/context.ts` | Records operations during a local edit and issues time tickets from the next change ID. |
| `Change` | `packages/sdk/src/document/change/change.ts` | Unit of document modification that executes operations and collects operation info and reverse operations. |
| `Presence`, `PresenceChange`, `PresenceProxy` | `packages/sdk/src/document/presence/{presence,change}.ts` | `Presence` is a JSON object map; `PresenceChange` is `Put`/`Clear` plus receive-only `Patch`; `PresenceProxy` is the `presence` handle of `Document::update_with_presence` (`set` merges, `set_with_history` is JS `set(p, { addToHistory: true })`, `get`, `clear`). `Document::{my_presence, presence, others_presences, presences}` mirror `getMyPresence/getPresence/getOthersPresences/getPresences`. |
| `DocEvent`, `Document::{subscribe, subscribe_path, unsubscribe, publish}`, `SubscriptionId` | `packages/sdk/src/document/document.ts` (`DocEventType`, `DocEvent`, `subscribe`, `publish`, `reconcilePresence`) | `subscribe` is JS `subscribe('all', cb)`; `subscribe_path` is JS `subscribe(targetPath, cb)` (segment-prefix match, operations filtered, empty changes dropped); `SubscriptionId` stands in for the JS `Unsubscribe` function. Callbacks run synchronously while the caller holds `&mut Document`. A cloned `Document` has no subscribers. JS `AuthError`, `EpochMismatch`, `WriteRejected`, `LocalChangesDropped`, and `PersistDisabled` events are not ported. |
| `Document::{history, history_mut}`, `HistoryRef`, `HistoryMut` | `doc.history` (`canUndo`, `canRedo`, `undo`, `redo`) in `packages/sdk/src/document/document.ts` | JS `doc.history.canUndo()` is `doc.history().can_undo()`; `doc.history.undo()` is `doc.history_mut().undo()`, which needs `&mut Document`. `HistoryRef` borrows the document for `can_undo`/`can_redo`; `HistoryMut` adds `undo`/`redo`, which return `Result<()>` and do nothing on an empty stack, as JS does. An undo or redo is a new local change marked as undo/redo (JS `markAsUndoRedo`). JS refuses an undo inside an `update` callback; Rust cannot express the call, as the callback has no access to the document. |
| `History`, `HistoryOperation` (internal) | `packages/sdk/src/document/history.ts` | Undo and redo stacks, 50 entries each (`MAX_UNDO_REDO_STACK_DEPTH`, JS `MaxUndoRedoStackDepth`), with `reconcile_created_at` and `reconcile_text_edit`. An entry holds reverse operations and the presence values to restore (JS `{ type: 'presence', value }`). The JS `maxUndoDepth` document option is not ported. |
| `ChangeInfo` | `ChangeInfo` in `packages/sdk/src/document/document.ts` | `actor`, `message`, `operations`; JS `clientSeq`/`serverSeq` and `rawChange` are not carried. |
| `OperationInfo`, `EditOpValue`, `StyleOpValue`, `TreeStyleOpValue` | `OpInfo` types in `packages/sdk/src/document/operation/operation.ts` | Public JS-shaped op infos converted from the internal `OpInfo`; internal `ArrayRemove` becomes `Remove { index }`, attribute values are decoded like JS `parseAttrValue`, and a removed text attribute is reported as `null`. `TreeEdit.value` carries the inserted nodes as `TreeNodeInput` values, like JS `TreeNode`. `TreeEdit.split_level` is always `None`, as JS never sets `splitLevel`: a split is reported as an empty change where it opened. |
| `StreamConnectionStatus`, `DocSyncStatus` | `packages/sdk/src/document/document.ts` | Payloads of `DocEvent::ConnectionChanged` and `DocEvent::SyncStatusChanged`, published by the client crate. |
| `ChangePack` | `packages/sdk/src/document/change/change_pack.ts` | Bundle of document key, checkpoint, changes, version vector, removal flag, and optional snapshot for sync. |
| `yorkie_core::wire` | `packages/sdk/src/api/converter.ts`, `../yorkie/api/converter/*` | Protocol-facing projection for changes, operations, simple elements, full JSON elements, RGA/RHT nodes, text nodes, tree nodes, and positions without exposing CRDT internals through the top-level SDK facade. |
| `CrdtElementMeta` | `packages/sdk/src/document/crdt/element.ts` | Rust metadata holder for CRDT element timestamps. |
| `CrdtElement` | `packages/sdk/src/document/crdt/element.ts` | Internal enum that exposes common element behavior across primitive, object, array, text, counter, and tree values. |
| `CrdtPrimitive` | `packages/sdk/src/document/crdt/primitive.ts` | Internal CRDT element for leaf primitive values. |
| `Hll` | `packages/sdk/src/document/crdt/hll.ts` | Internal HyperLogLog register set for dedup counters, using precision 14 and xxhash64 seed 0. |
| `CrdtCounter` | `packages/sdk/src/document/crdt/counter.ts` | Internal CRDT counter element for integer, long, and integer-dedup counters. |
| `ElementRht` | `packages/sdk/src/document/crdt/element_rht.ts` | Internal replicated hash table for object members, keyed by visible key and element creation time. |
| `CrdtObject` | `packages/sdk/src/document/crdt/object.ts` | Internal CRDT container for object members stored in `ElementRht`. |
| `RhtNode` | `packages/sdk/src/document/crdt/rht.ts` | Internal node for string attributes used by text and tree values. |
| `Rht` | `packages/sdk/src/document/crdt/rht.ts` | Internal replicated hash table for string attributes. Cross-check with Go `pkg/document/crdt/rht.go`. |
| `RgaTreeSplitNodeId` | `packages/sdk/src/document/crdt/rga_tree_split.ts` | Internal block identifier for text nodes, ordered by creation ticket and UTF-16 offset. |
| `RgaTreeSplit` | `packages/sdk/src/document/crdt/rga_tree_split.ts` | Internal block-based RGA for text. Rust currently keeps the same node semantics with a linear backing structure. |
| `TextValue` | `packages/sdk/src/document/crdt/text.ts` | Internal text block content plus `Rht` attributes. |
| `CrdtText` | `packages/sdk/src/document/crdt/text.ts` | Internal CRDT text element backed by `RgaTreeSplit<TextValue>` and indexed by `CrdtRoot` as a CRDT element. |
| `TreeNodeId` | `packages/sdk/src/document/crdt/tree.ts` | Internal tree node identifier ordered by creation ticket and UTF-16 offset. |
| `TreeNode` | `packages/sdk/src/document/crdt/tree.ts` | Internal tree node for element/text content, attributes, tombstones, split links, and merge metadata. |
| `CrdtTree` | `packages/sdk/src/document/crdt/tree.ts` | Internal CRDT tree element with node lookup, serialization, data-size accounting, deep copy, GC-pair discovery, split-free element insert/delete edit behavior, text-node split insert/delete edit behavior, multi-level element split behavior, visible-boundary merge behavior, and text-boundary split style/remove-style behavior. |
| `RgaTreeList` | `packages/sdk/src/document/crdt/rga_tree_list.ts` | Internal replicated growable array list. Rust currently keeps the same node semantics with a linear backing structure. |
| `CrdtArray` | `packages/sdk/src/document/crdt/array.ts` | Internal CRDT container for array elements stored in `RgaTreeList`. |
| `CrdtRoot` | `packages/sdk/src/document/crdt/root.ts` | Internal root that indexes CRDT elements by creation time and creates element paths. |
| `SetOperation` | `packages/sdk/src/document/operation/set_operation.ts` | Operation that sets an object member and updates the root index and GC candidates. |
| `RemoveOperation` | `packages/sdk/src/document/operation/remove_operation.ts` | Operation that removes an object member or array element by creation time and updates GC candidates. |
| `AddOperation` | `packages/sdk/src/document/operation/add_operation.ts` | Operation that inserts a CRDT element into an array after a position identity. |
| `MoveOperation` | `packages/sdk/src/document/operation/move_operation.ts` | Operation that moves an existing array element using position-node identity. |
| `ArraySetOperation` | `packages/sdk/src/document/operation/array_set_operation.ts` | Operation that replaces an array element by inserting a new element and tombstoning the previous one. |
| `IncreaseOperation` | `packages/sdk/src/document/operation/increase_operation.ts` | Operation that increases regular counters with numeric operands and dedup counters with actor-scoped unit increments. |
| `EditOperation` | `packages/sdk/src/document/operation/edit_operation.ts` | Operation that edits a text range, registers removed text-node GC pairs, and creates a reverse edit. |
| `StyleOperation` | `packages/sdk/src/document/operation/style_operation.ts` | Operation that applies or removes text attributes, registers removed attribute GC pairs, and creates a reverse style operation. |
| `TreeEditOperation` | `packages/sdk/src/document/operation/tree_edit_operation.ts` | Operation that edits tree content. Rust currently covers split-free element insert/delete, text-node split insert/delete, multi-level element split, visible-boundary merge, operation info, insert/delete/split/merge reverse operations, and tree-node GC registration. |
| `TreeStyleOperation` | `packages/sdk/src/document/operation/tree_style_operation.ts` | Operation that applies or removes tree element attributes, splits text at style boundaries, registers removed attribute GC pairs, and creates a reverse tree style operation. |
| `JsonObject` | `packages/sdk/src/document/json/object.ts` | Public JSON-like object API must map to JS object behavior. |
| `JsonObject::set` | `ObjectProxy.setInternal()` | Reject object keys containing `.` and store the new member value. |
| `JsonObject::remove` | `ObjectProxy.deleteInternal()` | Rust method for deleting an object member. Missing keys must be a no-op. |
| `JsonArray` | `packages/sdk/src/document/json/array.ts` | Public array API maps index, ID-based, and splice-like array edits to add, array-set, remove, and move operations during `Document::update`. |
| `JsonArrayElement` | JS `WrappedElement` read-only lookup shape | Lightweight Rust lookup value that exposes an array element ID and borrowed JSON value. It is not a mutable proxy. |
| `JsonCounter` | `packages/sdk/src/document/json/counter.ts` | Public counter facade for regular increase operations and dedup actor-add operations during `Document::update`. |
| `JsonText` | `packages/sdk/src/document/json/text.ts` | Public text facade. `edit`, `delete`, `empty`, and `set_style` take UTF-16 indexes, apply locally, and record `EditOperation`/`StyleOperation` during `Document::update`; attribute values are `serde_json::Value`s stringified like JS `stringifyObjectValues`. Created with `JsonObject::set_text` or `JsonArray::push_text`; editing outside an update fails with `YorkieError::NotInitialized` like JS `ErrNotInitialized`. |
| `TextChunk` | `TextValueType` in `packages/sdk/src/document/crdt/text.ts` | Entry of `JsonText::values` (JS `Text.values()`): content plus decoded attributes, `None` when the run has none. |
| `JsonTree` | `packages/sdk/src/document/json/tree.ts` | Public tree facade. `edit` (JS `edit`/`editBulk`), `edit_by_path` (JS `editByPath`/`editBulkByPath`), `split_by_path` (JS `splitByPath`), `merge_by_path` (JS `mergeByPath`), `style`, and `remove_style` apply locally and record `TreeEditOperation`/`TreeStyleOperation` during `Document::update`; `to_xml`, `to_json`, `size` (JS `getSize`), `index_to_path`, and `path_to_index` read it. Created with `JsonObject::set_tree` or `JsonTree::new` (JS `new Tree(root)`). A `split_level` above zero splits that many elements, and the recorded operation carries the tickets the split issued (JS `splitTickets`); an uninitialized tree fails with `YorkieError::NotInitialized`. |
| `TreeNodeInput` | `TreeNode`, `ElementNode`, `TextNode` in `packages/sdk/src/document/crdt/tree.ts` | Tree node given to `JsonTree` and reported in `OperationInfo::TreeEdit`. Element attributes are `serde_json::Value`s stringified like JS `stringifyObjectValues`; invalid input fails with `YorkieError::InvalidArgument` (JS `ErrInvalidArgument`). |
| `JsonValue` | JS JSON element/proxy values | Temporary Rust wrapper for porting primitives, counters, texts, trees, objects, and arrays. |
| `SchemaRule`, `TreeNodeRule` | `packages/sdk/src/api/converter.ts`, `../yorkie/api/converter/from_pb.go` | Rust document metadata for schema rules received during attach or set with `Document::set_schema_rules`. Field names track the protobuf/JS/Go shape, with Rust `rule_type` replacing reserved `type`. |
| `schema::validate_yorkie_ruleset` (internal), `schema::tree_validator`, `schema::content_expression` | `packages/sdk/src/document/schema/{ruleset_validator,tree-validator,content-expression}.ts` | Checks a CRDT root against schema rules: primitive, object, array, and Yorkie types by path, and tree node rules (node types, ProseMirror-style content expressions with groups, allowed marks). A rule that cannot be applied (bad path, unknown type, unparsable content expression) is `YorkieError::InvalidArgument`, where JS throws a plain `Error`. |
| `Document::doc_size`, `DocSize`, `DataSize` | `getDocSize`, `DocSize`, `DataSize`, `totalDocSize`, `totalDataSize` in `packages/sdk/src/document/document.ts` and `src/util/resource.ts` | Live and garbage sizes in bytes; `DocSize::total` and `DataSize::total` are JS `totalDocSize` and `totalDataSize`. |
| `YorkieError::{SchemaValidationFailed, DocumentSizeExceedsLimit}` | `ErrDocumentSchemaValidationFailed`, `ErrDocumentSizeExceedsLimit` in `packages/sdk/src/util/error.ts`; the validation block of `Document.update` | A local update that breaks the schema rules or makes the document larger than `max_size_per_document` fails with these, with the JS messages (`schema validation failed: ...`, `document size exceeded: {size} > {limit}`), and leaves the document untouched. Remote changes and undo/redo are not checked, as in JS. |
| `Client` | `packages/sdk/src/client/client.ts` | Cloneable handle (`Arc<ClientInner>`) that owns an `Arc<dyn ClientTransport>` (`Client::new` builds a `ConnectTransport`, `Client::with_transport` takes any transport) and the attached documents. `&self` async `activate`/`deactivate`/`attach`/`detach`/`sync`/`remove`/`change_sync_mode` follow JS and run one at a time on an async task queue (JS `enqueueTask`): the actor is `actor_id` or else `client_id` (JS `res.actorId \|\| res.clientId`) while requests carry `client_id`; attach stamps the actor and sets `initial_presence` first, rejects an attached or attaching key (`AlreadyAttached`), re-checks the session after its RPC, and in a realtime mode waits for the watch stream like JS awaits `runWatchLoop`; detach clears presence first; `sync(None)` syncs every attachment in its mode; every sync publishes `SyncStatusChanged`; deactivate stops the loops and sets every attached document and channel to `Detached`. Activating starts the sync loop, so a client must run inside a tokio runtime. `running_task_count` (hidden) reports the live loops for tests. `has` covers documents and channels, which share one key space as in the JS `attachmentMap`. Channel methods are in the `Client` channel row. |
| `Client::{attach_channel, detach_channel, sync_channel, change_channel_sync_mode, peek_channel, broadcast}` | `attachChannel`, `detachChannel`, `sync(channel)`, `changeChannelSyncMode`, `peekChannel`, `broadcast`, and the channel branch of `syncInternal`/`runSyncLoop` in `client.ts` | `attach_channel` only registers the channel (default `Realtime`; `Manual`/`Realtime`/`Polling` only; heartbeat interval > 0) and starts the sync loop if needed, on an active client or not. The loop's first `RefreshChannel` carries the client key and metadata, adopts the server-issued client ID if the client has none (which activates it), and marks the channel attached with its session ID; later refreshes run every heartbeat interval and publish `PresenceChanged` on a count change. `ErrSessionNotFound` clears the session so the next refresh starts a new one; other failures publish `SyncError`. A realtime channel then watches (not awaited). `detach_channel` is local (the server expires the session by TTL) and, unlike JS, also clears the session ID so a re-attach starts a new session. `sync_channel` refreshes now (`Manual` channels refresh only this way). `peek_channel` needs no activation. `broadcast(key, topic, payload, options)` sends the JSON payload with retries and waits; see `BroadcastOptions`. Channel shard keys use the first key path. |
| `Channel`, `ChannelStatus`, `ChannelEvent`, `ChannelSubscriptionId` | `Channel`, `ChannelStatus`, `ChannelEventType`/`ChannelEvent` in `packages/sdk/src/channel/channel.ts` | `Channel::new` validates the key like JS `validateChannelKey` (`ClientError::InvalidArgument`). Getters mirror `getKey`/`getFirstKeyPath`/`getStatus`/`isAttached`/`getActorID`/`getSessionID`/`getSessionCount`; `set_session_id` is hidden. `subscribe` is JS `subscribe('all' \| cb)`, `subscribe_topic` is JS `subscribe(topic, cb)`; the JS type filters (`presence`, `broadcast`, `local-broadcast`, `sync-error`) are a `match` on `ChannelEvent::{PresenceChanged, Initialized, Broadcast, LocalBroadcast, SyncError}`. Callbacks run under the channel lock. `broadcast` requires an attached channel and publishes `LocalBroadcast`, which the attaching client sends in the background. JS `ChannelStatus::Removed` and the `AuthError` event are not ported. |
| `AttachChannelOptions`, `DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS` | `AttachChannelOptions`, `DefaultChannelHeartbeatMs` in `client.ts` | `sync_mode` (default `Realtime`) and `channel_heartbeat_interval` (overrides `ClientOptions::channel_heartbeat_interval`, default 5s, a third of the server's 15s session TTL). |
| `SharedChannel` | `Channel` passed to `client.attach` in `client.ts` | `Arc<std::sync::Mutex<Channel>>`, locked by the client like `SharedDocument`: only for synchronous steps, never across an `.await`, never with the client state lock. |
| `BroadcastOptions`, `BroadcastErrorHandler` | `BroadcastOptions`, `DefaultBroadcastOptions`, and the retry loop of `broadcast` in `client.ts` | `max_retries` (`None` = JS `Infinity`) bounds the retries of retryable failures, which wait 1s, 2s, 4s, ... at most 20s. `error` gets any final failure of a `Channel::broadcast`; JS calls it only when the first try fails with no retry left (`maxRetries: 0`), never after retries or on a non-retryable failure. Unlike JS, every try checks that the channel is still attached, `Client::broadcast` waits for the retries instead of resolving after the first failure, and a non-retryable failure is reported rather than swallowed. JS's unserializable-payload error has no Rust case: a `serde_json::Value` always serializes. |
| `SharedDocument` | `Document` passed to `client.attach` in `packages/sdk/src/client/client.ts` | `Arc<std::sync::Mutex<Document>>` shared by the app and the client. The client locks it only for synchronous steps, never across an `.await`, and never together with its own state lock. Document event callbacks run under this lock, so a callback must not lock the same document again. |
| `AttachOptions` | `AttachOptions` in `packages/sdk/src/client/client.ts` | `initial_root`, `initial_presence` (default empty), `sync_mode` (default `Realtime`), `document_poll_interval`, `schema`. `disableGC` is out of scope. |
| `ClientTransport` | `packages/sdk/src/client/client.ts`, `client/client.go` | Rust async transport boundary for activate/deactivate/document attach/detach/remove, client-side push-pull, and document watch request and response fields, including shard key construction, `ChangePack` handoff, removed `ChangePack` flag, document ID, max-size metadata, schema rules, push-only mode, the activate response's optional `actor_id`, and the channel RPCs (`refresh_channel`, `peek_channel`, `watch_channel`, `broadcast`). Object-safe (`#[async_trait]`, `&self`, `Send + Sync`); `Client` holds it as `Arc<dyn ClientTransport>`. |
| `ConnectTransport` | `packages/sdk/src/client/client.ts` (Connect transport) | Unary RPCs over HTTP with `application/proto` bodies, `x-api-key`/`x-shard-key`/`x-yorkie-user-agent` headers, and Connect JSON error bodies decoded into `ClientError::Rpc`. `watch_document` calls the server-streaming `Watch` RPC with `application/connect+proto` envelopes (flag byte, 4-byte big-endian length); the `0x02` end-stream message's optional error becomes `ClientError::Rpc`. `watch_channel` uses the same `Watch` RPC with a channel descriptor. |
| `RefreshChannelRequest`/`Response`, `PeekChannelRequest`/`Response`, `BroadcastRequest`/`Response`, `WatchChannelRequest`, `WatchChannelResponse`, `WatchChannelEvent`, `WatchChannelEventType`, `ChannelWatchStream` | `RefreshChannel`, `PeekChannel`, `Broadcast`, and the channel `Watch` built and read in `syncInternal`, `peekChannel`, `broadcast`, `createChannelWatchStream`, `handleWatchChannelResponse` (`client.ts`) | Channel transport DTOs. A refresh without `session_id` is the first call and carries the client key and metadata; its response carries `client_id` and `session_id`. A channel watch response is `Initialization` (session count, seq, heartbeat interval), `Event` (`Presence` for every type but `BROADCAST`, as in JS, with publisher, count, seq, topic, JSON payload bytes), or `Heartbeat`. |
| `WatchDocumentRequest`, `WatchDocumentResponse`, `WatchDocEvent`, `WatchDocEventType`, `WatchStream` | `WatchRequest`/`WatchResponse` built and read in `createDocumentWatchStream`/`handleWatchDocumentResponse` (`packages/sdk/src/client/client.ts`) | Transport DTOs for one document's watch stream: the request carries `client_id`, `actor_id`, the document ID, `heartbeat_interval_ms`, and the shard key; a response is `Initialization` (client IDs, heartbeat interval), `Event` (JS `PbDocEventType` and publisher), or `Heartbeat`. `WatchStream` is a boxed `Stream` of `ClientResult<WatchDocumentResponse>`. |
| Sync loop (`sync_loop.rs`), `Attachment::need_realtime_sync`, `ChannelAttachment::need_sync` | `runSyncLoop` in `client.ts`, `needRealtimeSync`/`needSync` in `client/attachment.ts` | One tokio task per client that is active or has channels (a client with neither ends it); each round syncs, in parallel, the documents that need it and refreshes the channels whose heartbeat is due (never `Manual`) and waits `sync_loop_duration`, or `retry_sync_loop_delay` after a retryable error (`ClientError::is_retryable`, JS `handleConnectError`); other errors stop it. |
| Watch loop (`watch.rs`) | `runWatchLoop` in `client.ts` and `client/attachment.ts`, `runWatchStream`/`watchIdleTimeout` in `client/watch.ts`, `createChannelWatchStream`/`handleWatchChannelResponse` in `client.ts` | One tokio task per realtime document or channel attachment, holding `Weak<ClientInner>`. Reconnects after `reconnect_stream_delay` and times out a silent stream at three heartbeat intervals. A document publishes `ConnectionChanged` and applies watch messages; a channel publishes `Initialized`/`PresenceChanged` (dropping stale seqs) and `Broadcast` (skipping non-JSON payloads), and nothing on disconnect, as in JS. |
| `Document::{apply_watch_init, apply_watched, apply_unwatched, reset_online_clients}` | `applyWatchInit`, `applyDocEvent`, `resetOnlineClients` in `packages/sdk/src/document/document.ts` | Hidden helpers the watch loop calls: the init replaces the online set (skipping self) and publishes `Initialized`; watched/unwatched update the online set and publish `Watched`/`Unwatched` through `reconcilePresence`; stale presences are kept so a reconnecting peer reappears. |
| `ClientOptions::watch_heartbeat_interval`, `DEFAULT_WATCH_HEARTBEAT_INTERVAL_MS` | `watchHeartbeatInterval`, `DefaultWatchHeartbeatMs` in `client.ts` | Heartbeat interval requested on watch streams (default 20s), rounded up to whole milliseconds; zero asks for none. |
| `yorkie-client::protocol` | `packages/sdk/src/client/client.ts`, `client/client.go`, protobuf generated request/response types | Converts client transport DTOs to generated protobuf messages and back. Shard key stays outside protobuf because JS/Go pass it as request metadata. |
| `ClientError` | `packages/sdk/src/util/error.ts`, `packages/sdk/src/client/client.ts` | Rust keeps client lifecycle errors separate from core document errors until the shared public error surface is finalized. `AlreadyAttached` maps JS `ErrAlreadyAttached`. `Rpc::yorkie_code` is the Yorkie code from the `google.rpc.ErrorInfo` detail of a Connect error (JS `errorCodeOf`); the client acts only on `ErrSessionNotFound` so far. `is_retryable` (crate-private) mirrors the Connect-code part of JS `handleConnectError`: `Transport` errors and `canceled`/`unknown`/`resource_exhausted`/`unavailable`/`unauthenticated` are retried by the loops. |
| `yorkie-protocol` | `packages/sdk/src/api/converter.ts`, `../yorkie/api/converter/*`, proto files | Checked-in generated protobuf types are produced from vendored Yorkie proto files; converters encode/decode `ChangePack` values and schema rules and should track JS/proto field names with Go wire behavior as a typed cross-check. |

Update this table whenever a new Rust type becomes part of the porting surface.

## First Porting Targets

Start with the JS SDK behaviors that define the document surface before network
sync:

1. Document key handling.
2. Primitive set on the root object.
3. Object set and nested object updates.
4. Array set and array push.
5. Object key removal.
6. Local change and operation capture.

Only after local change capture is aligned should the Rust SDK move to
`ChangePack`, protobuf conversion, and `Client.attach/sync`.

## Agent Checklist

Before adding or changing a Rust SDK concept:

- Did you check [Current Porting Gaps](current-porting-gaps.md) for known
  differences in the area you are touching?
- Which JS SDK file is the source of truth?
- Which JS SDK test or behavior is being ported?
- Which Go SDK/client file was cross-checked, if the feature has a comparable
  implementation?
- Does the Rust name map clearly to the JS name?
- Is this a public API or an internal implementation detail?
- If this is a new internal abstraction, which JS concept does it support?
- Does it preserve the dependency direction in
  [Dependency Boundaries](dependency-boundaries.md)?
- Did `cargo fmt` and `cargo test` pass?
