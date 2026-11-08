#![forbid(unsafe_code)]
//! Network client layer for Yorkie.

mod attachment;
mod auth;
mod channel;
mod client;
mod connect;
mod error;
mod options;
/// Transport DTO to protobuf conversion used by `ConnectTransport`. Not part
/// of the supported API; implement `ClientTransport` with the re-exported
/// request and response types instead.
#[doc(hidden)]
pub mod protocol;
mod sync_loop;
mod transport;
mod watch;

pub use attachment::SharedDocument;
pub use channel::{
    BroadcastErrorHandler, BroadcastOptions, Channel, ChannelEvent, ChannelStatus,
    ChannelSubscriptionId, SharedChannel,
};
pub use client::{Client, ClientCondition, ClientStatus};
pub use connect::ConnectTransport;
pub use error::{ClientError, ClientResult};
pub use options::{
    AttachChannelOptions, AttachOptions, AuthTokenInjector, ClientOptions, DeactivateOptions,
    DetachOptions, SyncMode, DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS, DEFAULT_POLLING_INTERVAL_MS,
    DEFAULT_RECONNECT_STREAM_DELAY_MS, DEFAULT_RETRY_SYNC_LOOP_DELAY_MS, DEFAULT_RPC_ADDR,
    DEFAULT_SYNC_LOOP_DURATION_MS, DEFAULT_WATCH_HEARTBEAT_INTERVAL_MS,
};
pub use transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    BroadcastRequest, BroadcastResponse, ChannelWatchStream, ClientTransport,
    DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
    DetachDocumentResponse, PeekChannelRequest, PeekChannelResponse, PushPullChangesRequest,
    PushPullChangesResponse, RefreshChannelRequest, RefreshChannelResponse, RemoveDocumentRequest,
    RemoveDocumentResponse, WatchChannelEvent, WatchChannelEventType, WatchChannelRequest,
    WatchChannelResponse, WatchDocEvent, WatchDocEventType, WatchDocumentRequest,
    WatchDocumentResponse, WatchStream,
};
pub use yorkie_core::{
    ActorId, DocStatus, Document, ErrorCode, JsonArray, JsonObject, JsonValue, Presence, Result,
    SchemaRule, TreeNodeRule, YorkieError,
};
