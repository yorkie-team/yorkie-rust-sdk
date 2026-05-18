use std::collections::BTreeMap;
use std::time::Duration;
use yorkie_core::JsonObject;

pub const DEFAULT_RPC_ADDR: &str = "https://api.yorkie.dev";
pub const DEFAULT_SYNC_LOOP_DURATION_MS: u64 = 50;
pub const DEFAULT_RETRY_SYNC_LOOP_DELAY_MS: u64 = 1000;
pub const DEFAULT_RECONNECT_STREAM_DELAY_MS: u64 = 1000;
pub const DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS: u64 = 30000;
pub const DEFAULT_POLLING_INTERVAL_MS: u64 = 3000;

/// Synchronization mode used by document and channel attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    Manual,
    Realtime,
    RealtimePushOnly,
    RealtimeSyncOff,
    Polling,
}

/// User-settable options for a Yorkie client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOptions {
    pub rpc_addr: String,
    pub key: Option<String>,
    pub api_key: String,
    pub metadata: BTreeMap<String, String>,
    pub sync_loop_duration: Duration,
    pub retry_sync_loop_delay: Duration,
    pub reconnect_stream_delay: Duration,
    pub channel_heartbeat_interval: Duration,
    pub user_agent: Option<String>,
}

/// User-settable options for deactivating clients.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeactivateOptions {
    pub keepalive: bool,
    pub synchronous: bool,
}

/// User-settable options for attaching documents.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttachOptions {
    pub initial_root: Option<JsonObject>,
    pub sync_mode: Option<SyncMode>,
    pub document_poll_interval: Option<Duration>,
    pub schema: Option<String>,
}

/// User-settable options for attaching channels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttachChannelOptions {
    pub sync_mode: Option<SyncMode>,
    pub channel_heartbeat_interval: Option<Duration>,
}

/// User-settable options for detaching documents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetachOptions;

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            rpc_addr: DEFAULT_RPC_ADDR.to_owned(),
            key: None,
            api_key: String::new(),
            metadata: BTreeMap::new(),
            sync_loop_duration: Duration::from_millis(DEFAULT_SYNC_LOOP_DURATION_MS),
            retry_sync_loop_delay: Duration::from_millis(DEFAULT_RETRY_SYNC_LOOP_DELAY_MS),
            reconnect_stream_delay: Duration::from_millis(DEFAULT_RECONNECT_STREAM_DELAY_MS),
            channel_heartbeat_interval: Duration::from_millis(
                DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS,
            ),
            user_agent: None,
        }
    }
}
