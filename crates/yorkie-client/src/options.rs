use crate::error::ClientResult;
use futures_util::future::BoxFuture;
use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;
use std::time::Duration;
use yorkie_core::{JsonObject, Presence};

pub const DEFAULT_RPC_ADDR: &str = "https://api.yorkie.dev";
pub const DEFAULT_SYNC_LOOP_DURATION_MS: u64 = 50;
pub const DEFAULT_RETRY_SYNC_LOOP_DELAY_MS: u64 = 1000;
pub const DEFAULT_RECONNECT_STREAM_DELAY_MS: u64 = 1000;
/// How often a channel session is refreshed (JS `DefaultChannelHeartbeatMs`),
/// a third of the server's 15s channel session TTL.
pub const DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS: u64 = 5000;
pub const DEFAULT_POLLING_INTERVAL_MS: u64 = 3000;
/// Heartbeat interval a client asks the server for on its watch streams
/// (JS `DefaultWatchHeartbeatMs`).
pub const DEFAULT_WATCH_HEARTBEAT_INTERVAL_MS: u64 = 20000;

/// Synchronization mode used by document and channel attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    Manual,
    Realtime,
    RealtimePushOnly,
    RealtimeSyncOff,
    Polling,
}

/// Returns a token for the auth webhook (JS `authTokenInjector`).
///
/// The client calls it with `None` when it activates, and with the reason
/// the server gave (the auth webhook's reason, such as `"token expired"`)
/// after a request is refused with `ErrUnauthenticated`. The token is sent in
/// the `authorization` header of every later request.
///
/// It may fail, as the JS promise may reject. `activate` then fails with its
/// error. After a refused request, a one-shot call (`attach`, `detach`,
/// `remove`, `sync`, `sync_channel`, `deactivate`, and the first try of a
/// broadcast) fails with its error, and otherwise with the refusal for the
/// caller to retry; the background loops keep the previous token, retry, and
/// call it again only after a backoff of 1s, doubling up to 30s, jittered by
/// 20%. A loop that got a refusal waits for that backoff, but at most 5s.
///
/// It may run while the client holds its task queue, so it must not call a
/// one-shot `Client` method (`attach`, `sync`, `deactivate`, and so on):
/// that waits for the same queue and deadlocks.
pub type AuthTokenInjector =
    Arc<dyn Fn(Option<String>) -> BoxFuture<'static, ClientResult<String>> + Send + Sync>;

/// User-settable options for a Yorkie client.
///
/// Two option sets are equal when their fields are, with the same
/// [`AuthTokenInjector`] callback, if any.
#[derive(Clone)]
pub struct ClientOptions {
    pub rpc_addr: String,
    pub key: Option<String>,
    pub api_key: String,
    pub metadata: BTreeMap<String, String>,
    pub sync_loop_duration: Duration,
    pub retry_sync_loop_delay: Duration,
    pub reconnect_stream_delay: Duration,
    pub channel_heartbeat_interval: Duration,
    /// How often the server should send a heartbeat on an idle watch stream
    /// (JS `watchHeartbeatInterval`). The server clamps it and reports what it
    /// uses; a stream silent for three times that long is reconnected. Zero
    /// asks for no heartbeats and disables the idle timeout.
    pub watch_heartbeat_interval: Duration,
    pub user_agent: Option<String>,
    /// Provides the auth webhook token (JS `authTokenInjector`); see
    /// [`AuthTokenInjector`]. `None` sends no `authorization` header.
    pub auth_token_injector: Option<AuthTokenInjector>,
}

impl Debug for ClientOptions {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self {
            rpc_addr,
            key,
            api_key,
            metadata,
            sync_loop_duration,
            retry_sync_loop_delay,
            reconnect_stream_delay,
            channel_heartbeat_interval,
            watch_heartbeat_interval,
            user_agent,
            auth_token_injector,
        } = self;
        f.debug_struct("ClientOptions")
            .field("rpc_addr", rpc_addr)
            .field("key", key)
            .field("api_key", api_key)
            .field("metadata", metadata)
            .field("sync_loop_duration", sync_loop_duration)
            .field("retry_sync_loop_delay", retry_sync_loop_delay)
            .field("reconnect_stream_delay", reconnect_stream_delay)
            .field("channel_heartbeat_interval", channel_heartbeat_interval)
            .field("watch_heartbeat_interval", watch_heartbeat_interval)
            .field("user_agent", user_agent)
            .field(
                "auth_token_injector",
                &auth_token_injector.as_ref().map(|_| "<fn>"),
            )
            .finish()
    }
}

impl PartialEq for ClientOptions {
    fn eq(&self, other: &Self) -> bool {
        // Destructured so that a new field cannot be left out.
        let Self {
            rpc_addr,
            key,
            api_key,
            metadata,
            sync_loop_duration,
            retry_sync_loop_delay,
            reconnect_stream_delay,
            channel_heartbeat_interval,
            watch_heartbeat_interval,
            user_agent,
            auth_token_injector,
        } = self;
        let same_injector = match (auth_token_injector, &other.auth_token_injector) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        *rpc_addr == other.rpc_addr
            && *key == other.key
            && *api_key == other.api_key
            && *metadata == other.metadata
            && *sync_loop_duration == other.sync_loop_duration
            && *retry_sync_loop_delay == other.retry_sync_loop_delay
            && *reconnect_stream_delay == other.reconnect_stream_delay
            && *channel_heartbeat_interval == other.channel_heartbeat_interval
            && *watch_heartbeat_interval == other.watch_heartbeat_interval
            && *user_agent == other.user_agent
            && same_injector
    }
}

impl Eq for ClientOptions {}

/// User-settable options for deactivating clients (JS `deactivate`
/// options).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeactivateOptions {
    /// Accepted for parity with JS and ignored: in JS it keeps a browser
    /// `fetch` alive while the page unloads, which has no Rust counterpart.
    pub keepalive: bool,
    pub synchronous: bool,
}

/// User-settable options for attaching documents (JS `AttachOptions`).
///
/// `sync_mode` defaults to [`SyncMode::Realtime`] and `initial_presence` to an
/// empty presence, as in JS `attach`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachOptions {
    pub initial_root: Option<JsonObject>,
    pub initial_presence: Presence,
    pub sync_mode: SyncMode,
    pub document_poll_interval: Option<Duration>,
    pub schema: Option<String>,
}

/// User-settable options for attaching channels (JS `AttachChannelOptions`).
///
/// `sync_mode` defaults to [`SyncMode::Realtime`]; channels take `Manual`,
/// `Realtime`, and `Polling` only. `channel_heartbeat_interval` overrides the
/// client's [`ClientOptions::channel_heartbeat_interval`] for this channel
/// and must be greater than zero.
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
            watch_heartbeat_interval: Duration::from_millis(DEFAULT_WATCH_HEARTBEAT_INTERVAL_MS),
            user_agent: None,
            auth_token_injector: None,
        }
    }
}

impl Default for AttachOptions {
    fn default() -> Self {
        Self {
            initial_root: None,
            initial_presence: Presence::new(),
            sync_mode: SyncMode::Realtime,
            document_poll_interval: None,
            schema: None,
        }
    }
}
