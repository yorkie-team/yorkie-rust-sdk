#![forbid(unsafe_code)]
//! Network client layer for Yorkie.

mod attachment;
mod client;
mod connect;
mod error;
mod options;
pub mod protocol;
mod transport;

pub use attachment::SharedDocument;
pub use client::{Client, ClientCondition, ClientStatus};
pub use connect::ConnectTransport;
pub use error::{ClientError, ClientResult};
pub use options::{
    AttachChannelOptions, AttachOptions, ClientOptions, DeactivateOptions, DetachOptions, SyncMode,
    DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS, DEFAULT_POLLING_INTERVAL_MS,
    DEFAULT_RECONNECT_STREAM_DELAY_MS, DEFAULT_RETRY_SYNC_LOOP_DELAY_MS, DEFAULT_RPC_ADDR,
    DEFAULT_SYNC_LOOP_DURATION_MS,
};
pub use transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    ClientTransport, DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
    DetachDocumentResponse, PushPullChangesRequest, PushPullChangesResponse, RemoveDocumentRequest,
    RemoveDocumentResponse, WatchDocEvent, WatchDocEventType, WatchDocumentRequest,
    WatchDocumentResponse, WatchStream,
};
pub use yorkie_core::{
    ActorId, DocStatus, Document, JsonArray, JsonObject, JsonValue, Presence, Result, SchemaRule,
    TreeNodeRule, YorkieError,
};
