#![forbid(unsafe_code)]
//! Network client layer for Yorkie.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub use yorkie_core::{Document, JsonArray, JsonObject, JsonValue, Result, YorkieError};

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

/// Client activation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    Deactivated,
    Activated,
}

/// Client background loop condition keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClientCondition {
    SyncLoop,
    WatchLoop,
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

/// Client state holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    key: String,
    status: ClientStatus,
    conditions: BTreeMap<ClientCondition, bool>,
    options: ClientOptions,
}

impl Client {
    pub fn new(options: ClientOptions) -> Self {
        let key = options.key.clone().unwrap_or_else(generate_client_key);
        let conditions = BTreeMap::from([
            (ClientCondition::SyncLoop, false),
            (ClientCondition::WatchLoop, false),
        ]);

        Self {
            key,
            status: ClientStatus::Deactivated,
            conditions,
            options,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn status(&self) -> ClientStatus {
        self.status
    }

    pub fn is_active(&self) -> bool {
        self.status == ClientStatus::Activated
    }

    pub fn condition(&self, condition: ClientCondition) -> bool {
        self.conditions.get(&condition).copied().unwrap_or(false)
    }

    pub fn options(&self) -> &ClientOptions {
        &self.options
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new(ClientOptions::default())
    }
}

fn generate_client_key() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("rust-{timestamp:x}-{counter:x}")
}

#[cfg(test)]
mod tests {
    use super::{
        AttachChannelOptions, AttachOptions, Client, ClientCondition, ClientOptions, ClientStatus,
        DeactivateOptions, DetachOptions, SyncMode, DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS,
        DEFAULT_POLLING_INTERVAL_MS, DEFAULT_RECONNECT_STREAM_DELAY_MS,
        DEFAULT_RETRY_SYNC_LOOP_DELAY_MS, DEFAULT_RPC_ADDR, DEFAULT_SYNC_LOOP_DURATION_MS,
    };
    use std::collections::BTreeMap;
    use std::time::Duration;

    #[test]
    fn creates_client_with_default_options() {
        let client = Client::default();

        assert!(!client.key().is_empty());
        assert_eq!(ClientStatus::Deactivated, client.status());
        assert!(!client.is_active());
        assert!(!client.condition(ClientCondition::SyncLoop));
        assert!(!client.condition(ClientCondition::WatchLoop));

        let options = client.options();
        assert_eq!(DEFAULT_RPC_ADDR, options.rpc_addr);
        assert_eq!(
            Duration::from_millis(DEFAULT_SYNC_LOOP_DURATION_MS),
            options.sync_loop_duration
        );
        assert_eq!(
            Duration::from_millis(DEFAULT_RETRY_SYNC_LOOP_DELAY_MS),
            options.retry_sync_loop_delay
        );
        assert_eq!(
            Duration::from_millis(DEFAULT_RECONNECT_STREAM_DELAY_MS),
            options.reconnect_stream_delay
        );
        assert_eq!(
            Duration::from_millis(DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS),
            options.channel_heartbeat_interval
        );
    }

    #[test]
    fn creates_attachment_option_defaults() {
        let deactivate_options = DeactivateOptions::default();
        let attach_options = AttachOptions::default();
        let attach_channel_options = AttachChannelOptions::default();
        let detach_options = DetachOptions;

        assert!(!deactivate_options.keepalive);
        assert!(!deactivate_options.synchronous);
        assert_eq!(None, attach_options.initial_root);
        assert_eq!(None, attach_options.sync_mode);
        assert_eq!(None, attach_options.document_poll_interval);
        assert_eq!(None, attach_options.schema);
        assert_eq!(None, attach_channel_options.sync_mode);
        assert_eq!(None, attach_channel_options.channel_heartbeat_interval);
        assert_eq!(DetachOptions, detach_options);
        assert_eq!(
            Duration::from_millis(3000),
            Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)
        );
    }

    #[test]
    fn carries_sync_mode_values() {
        let attach_options = AttachOptions {
            sync_mode: Some(SyncMode::Polling),
            document_poll_interval: Some(Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)),
            schema: Some("schema".to_owned()),
            ..AttachOptions::default()
        };
        let channel_options = AttachChannelOptions {
            sync_mode: Some(SyncMode::Realtime),
            channel_heartbeat_interval: Some(Duration::from_millis(
                DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS,
            )),
        };

        assert_eq!(Some(SyncMode::Polling), attach_options.sync_mode);
        assert_eq!(
            Some(Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)),
            attach_options.document_poll_interval
        );
        assert_eq!(Some(SyncMode::Realtime), channel_options.sync_mode);
        assert_eq!(SyncMode::Manual, SyncMode::Manual);
        assert_eq!(SyncMode::RealtimePushOnly, SyncMode::RealtimePushOnly);
        assert_eq!(SyncMode::RealtimeSyncOff, SyncMode::RealtimeSyncOff);
    }

    #[test]
    fn uses_explicit_client_options() {
        let options = ClientOptions {
            rpc_addr: "http://localhost:8080".to_owned(),
            key: Some("client-key".to_owned()),
            api_key: "api-key".to_owned(),
            metadata: BTreeMap::from([("region".to_owned(), "local".to_owned())]),
            sync_loop_duration: Duration::from_millis(25),
            retry_sync_loop_delay: Duration::from_millis(200),
            reconnect_stream_delay: Duration::from_millis(300),
            channel_heartbeat_interval: Duration::from_secs(10),
            user_agent: Some("test-agent".to_owned()),
        };

        let client = Client::new(options.clone());

        assert_eq!("client-key", client.key());
        assert_eq!(&options, client.options());
        assert_eq!(
            Some("local"),
            client.options().metadata.get("region").map(String::as_str)
        );
    }
}
