#![forbid(unsafe_code)]
//! Network client layer for Yorkie.

mod attachment;
mod client;
mod error;
mod options;
mod transport;

pub use client::{Client, ClientCondition, ClientStatus};
pub use error::{ClientError, ClientResult};
pub use options::{
    AttachChannelOptions, AttachOptions, ClientOptions, DeactivateOptions, DetachOptions, SyncMode,
    SyncOptions, DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS, DEFAULT_POLLING_INTERVAL_MS,
    DEFAULT_RECONNECT_STREAM_DELAY_MS, DEFAULT_RETRY_SYNC_LOOP_DELAY_MS, DEFAULT_RPC_ADDR,
    DEFAULT_SYNC_LOOP_DURATION_MS,
};
pub use transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    ClientTransport, DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
    DetachDocumentResponse, PushPullChangesRequest, PushPullChangesResponse,
};
pub use yorkie_core::{
    ActorId, DocStatus, Document, JsonArray, JsonObject, JsonValue, Result, SchemaRule,
    TreeNodeRule, YorkieError,
};
