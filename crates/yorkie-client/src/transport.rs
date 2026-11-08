use crate::error::ClientResult;
use async_trait::async_trait;
use futures_util::Stream;
use std::collections::BTreeMap;
use std::pin::Pin;
use yorkie_core::{ActorId, ChangePack, SchemaRule};

/// Request data for activating a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateClientRequest {
    pub client_key: String,
    pub metadata: BTreeMap<String, String>,
    pub shard_key: String,
}

/// Response data for activating a client.
///
/// `client_id` is the per-session ID sent back in later requests. `actor_id`
/// is the stable actor stamped into document changes; it is `None` when the
/// server does not return one (JS `res.actorId || res.clientId`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateClientResponse {
    pub client_id: ActorId,
    pub actor_id: Option<ActorId>,
}

/// Request data for deactivating a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeactivateClientRequest {
    pub client_id: ActorId,
    pub synchronous: bool,
    pub shard_key: String,
}

/// Response data for deactivating a client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeactivateClientResponse;

/// Request data for attaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachDocumentRequest {
    pub client_id: ActorId,
    pub change_pack: ChangePack,
    pub schema_key: Option<String>,
    pub shard_key: String,
}

/// Response data for attaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachDocumentResponse {
    pub document_id: String,
    pub change_pack: ChangePack,
    pub max_size_per_document: usize,
    pub schema_rules: Vec<SchemaRule>,
}

/// Request data for detaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachDocumentRequest {
    pub client_id: ActorId,
    pub document_id: String,
    pub change_pack: ChangePack,
    pub remove_if_not_attached: bool,
    pub shard_key: String,
}

/// Response data for detaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachDocumentResponse {
    pub change_pack: ChangePack,
}

/// Request data for removing a document.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveDocumentRequest {
    pub client_id: ActorId,
    pub document_id: String,
    pub change_pack: ChangePack,
    pub shard_key: String,
}

/// Response data for removing a document.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveDocumentResponse {
    pub change_pack: ChangePack,
}

/// Request data for pushing and pulling document changes.
#[derive(Debug, Clone, PartialEq)]
pub struct PushPullChangesRequest {
    pub client_id: ActorId,
    pub document_id: String,
    pub change_pack: ChangePack,
    pub push_only: bool,
    pub shard_key: String,
}

/// Response data for pushing and pulling document changes.
#[derive(Debug, Clone, PartialEq)]
pub struct PushPullChangesResponse {
    pub change_pack: ChangePack,
}

/// Request data for refreshing a channel session (JS `refreshChannel` in
/// `syncInternal`).
///
/// The first refresh of a channel has no `session_id`: it sends the client
/// key and metadata, and the server activates the client if `client_id` is
/// `None`, attaches it to the channel, and refreshes the session in one round
/// trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshChannelRequest {
    pub client_id: Option<ActorId>,
    pub channel_key: String,
    pub session_id: Option<String>,
    /// Sent on the first refresh only; empty afterwards.
    pub client_key: String,
    /// Sent on the first refresh only; empty afterwards.
    pub metadata: BTreeMap<String, String>,
    pub shard_key: String,
}

/// Response data for refreshing a channel session. `client_id` and
/// `session_id` are set only in reply to a first refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshChannelResponse {
    pub session_count: u64,
    pub client_id: Option<ActorId>,
    pub session_id: Option<String>,
}

/// Request data for reading a channel's session count without joining it
/// (JS `peekChannel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekChannelRequest {
    pub channel_key: String,
    pub shard_key: String,
}

/// Response data for peeking at a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeekChannelResponse {
    pub session_count: u64,
}

/// Request data for broadcasting to a channel (JS `rpcClient.broadcast`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastRequest {
    pub client_id: ActorId,
    pub channel_key: String,
    pub topic: String,
    /// The JSON encoding of the payload (JS `JSON.stringify`).
    pub payload: Vec<u8>,
    pub shard_key: String,
}

/// Response data for broadcasting to a channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BroadcastResponse;

/// Request data for watching a channel (JS `watch` with a channel resource
/// descriptor, built in `createChannelWatchStream`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchChannelRequest {
    pub client_id: ActorId,
    pub channel_key: String,
    /// How often the server should send a heartbeat on an idle stream; 0
    /// asks for none.
    pub heartbeat_interval_ms: u64,
    pub shard_key: String,
}

/// One message of a channel watch stream (JS `WatchResponse` with a channel
/// body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchChannelResponse {
    /// The session count when the stream opened, its sequence number, and the
    /// heartbeat interval the server settled on (0 when it sends none).
    Initialization {
        session_count: u64,
        seq: u64,
        heartbeat_interval_ms: u64,
    },
    /// The session count changed, or a client broadcast a message.
    Event(WatchChannelEvent),
    /// Keeps an idle stream alive; carries nothing.
    Heartbeat,
}

/// A channel event from the watch stream (JS `ChannelEvent` in the API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchChannelEvent {
    pub event_type: WatchChannelEventType,
    pub publisher: ActorId,
    pub session_count: u64,
    pub seq: u64,
    pub topic: String,
    /// The JSON encoding of a broadcast payload.
    pub payload: Vec<u8>,
}

/// The kind of a watch stream channel event (JS `PbChannelEventType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchChannelEventType {
    /// The session count changed. JS treats every type but `BROADCAST` this
    /// way, and so does the conversion.
    Presence,
    Broadcast,
}

/// A stream of channel watch messages, which ends like a [`WatchStream`].
pub type ChannelWatchStream =
    Pin<Box<dyn Stream<Item = ClientResult<WatchChannelResponse>> + Send>>;

/// Request data for watching a document (JS `watch` with a document
/// resource descriptor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchDocumentRequest {
    pub client_id: ActorId,
    /// The stable actor the client stamps into changes, so the server keys
    /// watched/unwatched events by the same ID as presences.
    pub actor_id: ActorId,
    pub document_id: String,
    /// How often the server should send a heartbeat on an idle stream; 0
    /// asks for none.
    pub heartbeat_interval_ms: u64,
    pub shard_key: String,
}

/// One message of a document watch stream (JS `WatchResponse` with a
/// document body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchDocumentResponse {
    /// The clients watching the document when the stream opened, and the
    /// heartbeat interval the server settled on (0 when it sends none).
    Initialization {
        client_ids: Vec<ActorId>,
        heartbeat_interval_ms: u64,
    },
    /// Something happened to the document on the server.
    Event(WatchDocEvent),
    /// Keeps an idle stream alive; carries nothing.
    Heartbeat,
}

/// A document event from the watch stream (JS `DocEvent` in the API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchDocEvent {
    pub event_type: WatchDocEventType,
    pub publisher: ActorId,
}

/// The kind of a watch stream document event (JS `PbDocEventType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchDocEventType {
    DocumentChanged,
    DocumentWatched,
    DocumentUnwatched,
    DocumentBroadcast,
}

/// A stream of watch messages. It ends with `None` when the server closes the
/// stream normally, and yields an error when it fails.
pub type WatchStream = Pin<Box<dyn Stream<Item = ClientResult<WatchDocumentResponse>> + Send>>;

/// Transport boundary used by the client lifecycle and document attachment.
#[async_trait]
pub trait ClientTransport: Send + Sync {
    async fn activate_client(
        &self,
        request: ActivateClientRequest,
    ) -> ClientResult<ActivateClientResponse>;

    async fn deactivate_client(
        &self,
        request: DeactivateClientRequest,
    ) -> ClientResult<DeactivateClientResponse>;

    async fn attach_document(
        &self,
        request: AttachDocumentRequest,
    ) -> ClientResult<AttachDocumentResponse>;

    async fn detach_document(
        &self,
        request: DetachDocumentRequest,
    ) -> ClientResult<DetachDocumentResponse>;

    async fn remove_document(
        &self,
        request: RemoveDocumentRequest,
    ) -> ClientResult<RemoveDocumentResponse>;

    async fn push_pull_changes(
        &self,
        request: PushPullChangesRequest,
    ) -> ClientResult<PushPullChangesResponse>;

    /// Opens a server stream of events for one document (JS `watch`).
    async fn watch_document(&self, request: WatchDocumentRequest) -> ClientResult<WatchStream>;

    /// Creates or refreshes a channel session (JS `refreshChannel`).
    async fn refresh_channel(
        &self,
        request: RefreshChannelRequest,
    ) -> ClientResult<RefreshChannelResponse>;

    /// Reads a channel's session count (JS `peekChannel`).
    async fn peek_channel(&self, request: PeekChannelRequest) -> ClientResult<PeekChannelResponse>;

    /// Opens a server stream of events for one channel (JS `watch`).
    async fn watch_channel(&self, request: WatchChannelRequest)
        -> ClientResult<ChannelWatchStream>;

    /// Sends a message to the clients watching a channel (JS `broadcast`).
    async fn broadcast(&self, request: BroadcastRequest) -> ClientResult<BroadcastResponse>;

    /// Sets the token sent in the `authorization` header of every later
    /// request (JS `setToken` of `createAuthInterceptor`). The client calls it
    /// with each token its auth token injector returns. A transport that does
    /// not authenticate can ignore it, which the default does.
    fn set_auth_token(&self, _token: String) {}
}
