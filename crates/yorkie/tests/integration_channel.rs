//! Channel and broadcast integration tests against a live Yorkie server.
//!
//! JS reference: `packages/sdk/test/integration/channel_test.ts`,
//! `channel_polling_test.ts`, and `broadcast_test.ts`, with
//! `withTwoClientsAndChannels` from `integration_helper.ts`. As in JS, most
//! clients are never activated: the first channel refresh activates them.
//!
//! JS "Should throw error when broadcasting unserializeable payload" is not
//! ported: a `serde_json::Value` payload always serializes.

mod integration_support;

use async_trait::async_trait;
use integration_support::{rpc_addr, unique_key};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yorkie::{
    ActivateClientRequest, ActivateClientResponse, AttachChannelOptions, AttachDocumentRequest,
    AttachDocumentResponse, BroadcastErrorHandler, BroadcastOptions, BroadcastRequest,
    BroadcastResponse, Channel, ChannelEvent, ChannelWatchStream, Client, ClientError,
    ClientOptions, ClientResult, ClientTransport, ConnectTransport, DeactivateClientRequest,
    DeactivateClientResponse, DeactivateOptions, DetachDocumentRequest, DetachDocumentResponse,
    PeekChannelRequest, PeekChannelResponse, PushPullChangesRequest, PushPullChangesResponse,
    RefreshChannelRequest, RefreshChannelResponse, RemoveDocumentRequest, RemoveDocumentResponse,
    SharedChannel, SyncMode, WatchChannelRequest, WatchDocumentRequest, WatchStream,
};

fn client(addr: &str) -> Client {
    Client::new(ClientOptions {
        rpc_addr: addr.to_owned(),
        ..ClientOptions::default()
    })
    .unwrap()
}

async fn active_client(addr: &str) -> Client {
    let client = client(addr);
    client.activate().await.unwrap();
    client
}

fn shared_channel(key: &str) -> SharedChannel {
    Arc::new(Mutex::new(Channel::new(key).unwrap()))
}

fn polling(heartbeat_ms: Option<u64>) -> AttachChannelOptions {
    AttachChannelOptions {
        sync_mode: Some(SyncMode::Polling),
        channel_heartbeat_interval: heartbeat_ms.map(Duration::from_millis),
    }
}

/// Polls `condition` every 50ms until it holds, failing after `timeout` (JS
/// `waitFor`).
async fn wait_for(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn is_attached(channel: &SharedChannel) -> bool {
    let channel = channel.lock().unwrap();
    channel.is_attached() && channel.session_id().is_some()
}

fn session_count(channel: &SharedChannel) -> u64 {
    channel.lock().unwrap().session_count()
}

/// Collects the session counts of `PresenceChanged` events (JS
/// `subscribe('presence', ...)`).
fn collect_counts(channel: &SharedChannel) -> Arc<Mutex<Vec<u64>>> {
    let counts = Arc::new(Mutex::new(Vec::new()));
    channel.lock().unwrap().subscribe({
        let counts = counts.clone();
        move |event| {
            if let ChannelEvent::PresenceChanged { count } = event {
                counts.lock().unwrap().push(*count);
            }
        }
    });
    counts
}

/// Two clients, never activated, each attaching its own channel object for
/// one key (JS `withTwoClientsAndChannels`).
struct TwoChannels {
    c1: Client,
    ch1: SharedChannel,
    c2: Client,
    ch2: SharedChannel,
}

impl TwoChannels {
    async fn attach(addr: &str, name: &str) -> Self {
        Self::attach_with(client(addr), client(addr), name).await
    }

    /// Attaches both channels in `Realtime` and waits until both are
    /// attached and watching. JS waits for the attach only; waiting for the
    /// watch too keeps a broadcast from racing the receiver's stream.
    async fn attach_with(c1: Client, c2: Client, name: &str) -> Self {
        let key = unique_key(name);
        let (ch1, ch2) = (shared_channel(&key), shared_channel(&key));
        let watching1 = watch_initialized(&ch1);
        let watching2 = watch_initialized(&ch2);
        c1.attach_channel(ch1.clone(), AttachChannelOptions::default())
            .await
            .unwrap();
        c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
            .await
            .unwrap();
        wait_for("both channels to watch", Duration::from_secs(10), || {
            is_attached(&ch1) && is_attached(&ch2) && *watching1.lock().unwrap() && {
                *watching2.lock().unwrap()
            }
        })
        .await;
        Self { c1, ch1, c2, ch2 }
    }

    async fn detach(self) {
        for (client, channel) in [(self.c1, self.ch1), (self.c2, self.ch2)] {
            if channel.lock().unwrap().is_attached() {
                client.detach_channel(&channel).await.unwrap();
            }
        }
    }
}

/// Whether the channel's watch stream reported `Initialized`.
fn watch_initialized(channel: &SharedChannel) -> Arc<Mutex<bool>> {
    let initialized = Arc::new(Mutex::new(false));
    channel.lock().unwrap().subscribe({
        let initialized = initialized.clone();
        move |event| {
            if matches!(event, ChannelEvent::Initialized { .. }) {
                *initialized.lock().unwrap() = true;
            }
        }
    });
    initialized
}

/// `(topic, payload)` of the broadcasts a channel received.
type Received = Arc<Mutex<Vec<(String, Value)>>>;

/// Collects `(topic, payload)` of the broadcasts of `topic` (JS
/// `subscribe(topic, ...)`), or of every topic for `None` (JS
/// `subscribe('broadcast', ...)`).
fn collect_broadcasts(
    channel: &SharedChannel,
    topic: Option<&str>,
) -> (Received, yorkie::ChannelSubscriptionId) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let callback = {
        let received = received.clone();
        move |event: &ChannelEvent| {
            if let ChannelEvent::Broadcast { topic, payload, .. } = event {
                received
                    .lock()
                    .unwrap()
                    .push((topic.clone(), payload.clone()));
            }
        }
    };
    let mut channel = channel.lock().unwrap();
    let id = match topic {
        Some(topic) => channel.subscribe_topic(topic, callback),
        None => channel.subscribe(callback),
    };
    (received, id)
}

fn broadcast(channel: &SharedChannel, topic: &str, payload: Value) {
    channel
        .lock()
        .unwrap()
        .broadcast(topic, payload, BroadcastOptions::default())
        .unwrap();
}

fn received_len(received: &Received) -> usize {
    received.lock().unwrap().len()
}

/// JS "should subscribe to presence events": a subscriber registered before
/// attaching sees the count of the first refresh.
#[tokio::test]
async fn presence_event_follows_the_first_refresh() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let channel = shared_channel(&unique_key("channel-presence-event"));
    let counts = collect_counts(&channel);

    client
        .attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("a presence event", Duration::from_secs(10), || {
        !counts.lock().unwrap().is_empty()
    })
    .await;

    client.detach_channel(&channel).await.unwrap();
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "should get presence count".
#[tokio::test]
async fn both_clients_count_two_sessions() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoChannels::attach(&addr, "channel-presence-count").await;

    wait_for("both counts to reach 2", Duration::from_secs(10), || {
        session_count(&t.ch1) >= 2 && session_count(&t.ch2) >= 2
    })
    .await;
    assert_eq!(session_count(&t.ch1), session_count(&t.ch2));
    t.detach().await;
}

/// JS "can attach a channel without activating the client".
#[tokio::test]
async fn channel_attaches_without_activating_the_client() {
    let Some(addr) = rpc_addr() else { return };
    let client = client(&addr);
    let channel = shared_channel(&unique_key("channel-without-activation"));

    client
        .attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("the first refresh", Duration::from_secs(8), || {
        is_attached(&channel)
    })
    .await;

    assert!(client.is_active());
    assert_eq!(
        client.id().as_ref(),
        channel.lock().unwrap().actor_id(),
        "the channel takes the client ID the server issued"
    );
    client.detach_channel(&channel).await.unwrap();
}

/// JS "peers see count drop after detach within TTL window": detaching is
/// local, so the count drops only once the server expires the session
/// (15s TTL, checked every 10s).
#[tokio::test]
async fn peers_see_the_count_drop_after_the_session_expires() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("channel-count-drop");
    let (c1, c2) = (client(&addr), client(&addr));
    let (ch1, ch2) = (shared_channel(&key), shared_channel(&key));
    // c1 polls fast so it sees the drop soon after the server does.
    c1.attach_channel(ch1.clone(), polling(Some(200)))
        .await
        .unwrap();
    c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("both counts to reach 2", Duration::from_secs(10), || {
        session_count(&ch1) == 2 && session_count(&ch2) == 2
    })
    .await;

    c2.detach_channel(&ch2).await.unwrap();
    wait_for("c1 to count one session", Duration::from_secs(30), || {
        session_count(&ch1) == 1
    })
    .await;
    c1.detach_channel(&ch1).await.unwrap();
}

/// JS "recovers transparently when the server forgets the session".
#[tokio::test]
async fn lost_session_is_created_again() {
    let Some(addr) = rpc_addr() else { return };
    let client = client(&addr);
    let channel = shared_channel(&unique_key("channel-lost-session"));
    client
        .attach_channel(channel.clone(), polling(Some(200)))
        .await
        .unwrap();
    wait_for("the first refresh", Duration::from_secs(10), || {
        is_attached(&channel)
    })
    .await;

    let unknown = "000000000000000000000000";
    channel
        .lock()
        .unwrap()
        .set_session_id(Some(unknown.to_owned()));
    wait_for("a new session", Duration::from_secs(15), || {
        channel
            .lock()
            .unwrap()
            .session_id()
            .is_some_and(|session_id| session_id != unknown)
    })
    .await;
    client.detach_channel(&channel).await.unwrap();
}

/// JS "peekChannel works without client.activate()".
#[tokio::test]
async fn peek_channel_works_without_activation() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("channel-peek");
    let writer = client(&addr);
    let channel = shared_channel(&key);
    writer
        .attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("the first refresh", Duration::from_secs(10), || {
        is_attached(&channel)
    })
    .await;

    let peeker = client(&addr);
    assert_eq!(1, peeker.peek_channel(&key).await.unwrap());
    assert!(!peeker.is_active());
    writer.detach_channel(&channel).await.unwrap();
}

/// JS "Polling mode reflects sessionCount via heartbeat without a watch
/// stream".
#[tokio::test]
async fn polling_reflects_the_session_count() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("channel-polling-count");
    let (c1, c2) = (active_client(&addr).await, active_client(&addr).await);
    let (ch1, ch2) = (shared_channel(&key), shared_channel(&key));

    c1.attach_channel(ch1.clone(), polling(Some(200)))
        .await
        .unwrap();
    c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("both counts to reach 2", Duration::from_secs(10), || {
        session_count(&ch1) >= 2 && session_count(&ch2) >= 2
    })
    .await;

    for (client, channel) in [(c1, ch1), (c2, ch2)] {
        client.detach_channel(&channel).await.unwrap();
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "Polling notifies subscribers when sessionCount changes".
#[tokio::test]
async fn polling_notifies_count_changes() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("channel-polling-notify");
    let (c1, c2) = (active_client(&addr).await, active_client(&addr).await);
    let ch1 = shared_channel(&key);
    c1.attach_channel(ch1.clone(), polling(Some(200)))
        .await
        .unwrap();
    let counts = collect_counts(&ch1);

    let ch2 = shared_channel(&key);
    c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("c1 to see two sessions", Duration::from_secs(10), || {
        counts
            .lock()
            .unwrap()
            .last()
            .is_some_and(|count| *count >= 2)
    })
    .await;

    for (client, channel) in [(c1, ch1), (c2, ch2)] {
        client.detach_channel(&channel).await.unwrap();
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}

/// JS "attachChannel rejects RealtimePushOnly/RealtimeSyncOff with
/// ErrInvalidArgument", "changeSyncMode rejects RealtimePushOnly for
/// channels", and "attachChannel rejects channelHeartbeatInterval: 0".
#[tokio::test]
async fn channels_reject_document_only_modes_and_a_zero_heartbeat() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let key = unique_key("channel-invalid-options");

    for sync_mode in [SyncMode::RealtimePushOnly, SyncMode::RealtimeSyncOff] {
        let options = AttachChannelOptions {
            sync_mode: Some(sync_mode),
            ..AttachChannelOptions::default()
        };
        assert!(matches!(
            client.attach_channel(shared_channel(&key), options).await,
            Err(ClientError::InvalidArgument(message))
                if message.starts_with("invalid channel sync mode")
        ));
    }
    let zero = AttachChannelOptions {
        channel_heartbeat_interval: Some(Duration::ZERO),
        ..AttachChannelOptions::default()
    };
    assert!(matches!(
        client.attach_channel(shared_channel(&key), zero).await,
        Err(ClientError::InvalidArgument(message))
            if message == "channel_heartbeat_interval must be greater than 0"
    ));

    let channel = client
        .attach_channel(shared_channel(&key), AttachChannelOptions::default())
        .await
        .unwrap();
    assert!(matches!(
        client
            .change_channel_sync_mode(&channel, SyncMode::RealtimePushOnly)
            .await,
        Err(ClientError::InvalidArgument(message))
            if message.starts_with("invalid channel sync mode")
    ));
    client.detach_channel(&channel).await.unwrap();
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "should subscribe to specific topic for broadcast events".
#[tokio::test]
async fn topic_subscribers_get_only_their_topic() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoChannels::attach(&addr, "channel-topic").await;
    let (chat, _) = collect_broadcasts(&t.ch2, Some("chat"));
    let (notification, _) = collect_broadcasts(&t.ch2, Some("notification"));

    broadcast(&t.ch1, "chat", json!({"message": "Hello, world!"}));
    wait_for("the chat message", Duration::from_secs(5), || {
        received_len(&chat) == 1
    })
    .await;
    broadcast(&t.ch1, "notification", json!({"alert": "New message"}));
    wait_for("the notification", Duration::from_secs(5), || {
        received_len(&notification) == 1
    })
    .await;

    assert_eq!(
        vec![("chat".to_owned(), json!({"message": "Hello, world!"}))],
        *chat.lock().unwrap()
    );
    assert_eq!(
        vec![("notification".to_owned(), json!({"alert": "New message"}))],
        *notification.lock().unwrap()
    );
    t.detach().await;
}

/// JS "should support legacy broadcast subscription" and "Should trigger
/// the handler for a subscribed broadcast event": a subscriber of every
/// event filters broadcasts by topic itself.
#[tokio::test]
async fn broadcast_reaches_the_other_client() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoChannels::attach(&addr, "channel-broadcast").await;
    let (received, _) = collect_broadcasts(&t.ch2, None);

    broadcast(&t.ch1, "test", json!({"a": 1, "b": "2"}));
    wait_for("the broadcast", Duration::from_secs(5), || {
        received_len(&received) == 1
    })
    .await;

    assert_eq!(
        vec![("test".to_owned(), json!({"a": 1, "b": "2"}))],
        *received.lock().unwrap()
    );
    t.detach().await;
}

/// JS "should mix topic-based and type-based subscriptions".
#[tokio::test]
async fn topic_and_all_event_subscriptions_mix() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoChannels::attach(&addr, "channel-mixed").await;
    let (chat, _) = collect_broadcasts(&t.ch2, Some("chat"));
    let (all, _) = collect_broadcasts(&t.ch2, None);

    broadcast(&t.ch1, "chat", json!("message1"));
    wait_for("the chat message", Duration::from_secs(5), || {
        received_len(&chat) == 1
    })
    .await;
    broadcast(&t.ch1, "notification", json!("message2"));
    wait_for("both broadcasts", Duration::from_secs(5), || {
        received_len(&all) == 2
    })
    .await;

    assert_eq!(1, received_len(&chat));
    assert_eq!(
        vec![
            ("chat".to_owned(), json!("message1")),
            ("notification".to_owned(), json!("message2")),
        ],
        *all.lock().unwrap()
    );
    t.detach().await;
}

/// JS "Should successfully broadcast serializeable payload": `broadcast`
/// accepts an attached channel; `Client::broadcast` waits for the send.
#[tokio::test]
async fn attached_channel_broadcasts() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let key = unique_key("channel-broadcast-payload");
    let channel = shared_channel(&key);
    client
        .attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("the first refresh", Duration::from_secs(8), || {
        is_attached(&channel)
    })
    .await;

    broadcast(&channel, "test", json!({"a": 1, "b": "2"}));
    client
        .broadcast(&key, "test", json!({"a": 1}), BroadcastOptions::default())
        .await
        .unwrap();

    client.detach_channel(&channel).await.unwrap();
    assert!(matches!(
        channel
            .lock()
            .unwrap()
            .broadcast("test", json!(1), BroadcastOptions::default()),
        Err(ClientError::NotAttached(_))
    ));
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "Should not trigger the handler for an unsubscribed broadcast event"
/// and "... after unsubscribing".
#[tokio::test]
async fn unsubscribed_topics_get_nothing() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoChannels::attach(&addr, "channel-unsubscribe").await;
    let (test1, id) = collect_broadcasts(&t.ch2, Some("test1"));
    let (test2, _) = collect_broadcasts(&t.ch2, Some("test2"));

    broadcast(&t.ch1, "test1", json!({"a": 1}));
    wait_for("the broadcast", Duration::from_secs(5), || {
        received_len(&test1) == 1
    })
    .await;
    assert_eq!(0, received_len(&test2));

    t.ch2.lock().unwrap().unsubscribe(id);
    broadcast(&t.ch1, "test1", json!({"a": 1}));
    // As in JS, assume a broadcast arrives within a second.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(1, received_len(&test1));
    t.detach().await;
}

/// JS "Should not trigger the handler for a broadcast event sent by the
/// publisher to itself".
#[tokio::test]
async fn broadcast_is_not_echoed_to_the_sender() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoChannels::attach(&addr, "channel-no-echo").await;
    let (sent, _) = collect_broadcasts(&t.ch1, None);
    let (received, _) = collect_broadcasts(&t.ch2, None);

    broadcast(&t.ch1, "test", json!({"a": 1}));
    wait_for("the broadcast", Duration::from_secs(5), || {
        received_len(&received) == 1
    })
    .await;
    // The receiver has it, so the sender would have it too by now.
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(0, received_len(&sent));
    t.detach().await;
}

/// Wraps `ConnectTransport` and fails the first `failures` broadcasts with
/// the error a browser `fetch` failure gives JS (`Unknown`). It also counts
/// broadcasts and channel watches.
struct FlakyBroadcastTransport {
    inner: ConnectTransport,
    failures: AtomicUsize,
    broadcast_calls: AtomicUsize,
    watch_channel_calls: AtomicUsize,
}

impl FlakyBroadcastTransport {
    fn client(addr: &str, failures: usize) -> (Client, Arc<Self>) {
        let options = ClientOptions {
            rpc_addr: addr.to_owned(),
            ..ClientOptions::default()
        };
        let transport = Arc::new(Self {
            inner: ConnectTransport::new(&options).unwrap(),
            failures: AtomicUsize::new(failures),
            broadcast_calls: AtomicUsize::new(0),
            watch_channel_calls: AtomicUsize::new(0),
        });
        (
            Client::with_transport(options, transport.clone()),
            transport,
        )
    }
}

#[async_trait]
impl ClientTransport for FlakyBroadcastTransport {
    async fn activate_client(
        &self,
        request: ActivateClientRequest,
    ) -> ClientResult<ActivateClientResponse> {
        self.inner.activate_client(request).await
    }

    async fn deactivate_client(
        &self,
        request: DeactivateClientRequest,
    ) -> ClientResult<DeactivateClientResponse> {
        self.inner.deactivate_client(request).await
    }

    async fn attach_document(
        &self,
        request: AttachDocumentRequest,
    ) -> ClientResult<AttachDocumentResponse> {
        self.inner.attach_document(request).await
    }

    async fn detach_document(
        &self,
        request: DetachDocumentRequest,
    ) -> ClientResult<DetachDocumentResponse> {
        self.inner.detach_document(request).await
    }

    async fn remove_document(
        &self,
        request: RemoveDocumentRequest,
    ) -> ClientResult<RemoveDocumentResponse> {
        self.inner.remove_document(request).await
    }

    async fn push_pull_changes(
        &self,
        request: PushPullChangesRequest,
    ) -> ClientResult<PushPullChangesResponse> {
        self.inner.push_pull_changes(request).await
    }

    async fn watch_document(&self, request: WatchDocumentRequest) -> ClientResult<WatchStream> {
        self.inner.watch_document(request).await
    }

    async fn refresh_channel(
        &self,
        request: RefreshChannelRequest,
    ) -> ClientResult<RefreshChannelResponse> {
        self.inner.refresh_channel(request).await
    }

    async fn peek_channel(&self, request: PeekChannelRequest) -> ClientResult<PeekChannelResponse> {
        self.inner.peek_channel(request).await
    }

    async fn watch_channel(
        &self,
        request: WatchChannelRequest,
    ) -> ClientResult<ChannelWatchStream> {
        self.watch_channel_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.watch_channel(request).await
    }

    async fn broadcast(&self, request: BroadcastRequest) -> ClientResult<BroadcastResponse> {
        self.broadcast_calls.fetch_add(1, Ordering::SeqCst);
        // Broadcasts of one client go out one at a time (the task queue).
        if self.failures.load(Ordering::SeqCst) > 0 {
            self.failures.fetch_sub(1, Ordering::SeqCst);
            return Err(ClientError::Rpc {
                code: "unknown".to_owned(),
                message: "Failed to fetch".to_owned(),
                yorkie_code: None,
                reason: None,
            });
        }
        self.inner.broadcast(request).await
    }
}

/// JS "Should retry broadcasting on network failure with retry option and
/// succeeds when network is restored": two failed sends are retried after
/// 1s and 2s, and the third reaches the other client.
#[tokio::test]
async fn broadcast_retries_after_network_failures() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, transport) = FlakyBroadcastTransport::client(&addr, 2);
    let t = TwoChannels::attach_with(c1, client(&addr), "channel-broadcast-retry").await;
    let (received, _) = collect_broadcasts(&t.ch2, Some("test"));

    broadcast(&t.ch1, "test", json!({"a": 1, "b": "2"}));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(0, received_len(&received));
    wait_for("the retried broadcast", Duration::from_secs(10), || {
        received_len(&received) == 1
    })
    .await;

    assert_eq!(3, transport.broadcast_calls.load(Ordering::SeqCst));
    assert_eq!(
        vec![("test".to_owned(), json!({"a": 1, "b": "2"}))],
        *received.lock().unwrap()
    );
    t.detach().await;
}

/// JS "Should not retry broadcasting on network failure when maxRetries is
/// set to zero": the error handler gets the error of the only try.
#[tokio::test]
async fn broadcast_without_retries_reports_the_failure() {
    let Some(addr) = rpc_addr() else { return };
    let (c1, transport) = FlakyBroadcastTransport::client(&addr, 1);
    let t = TwoChannels::attach_with(c1, client(&addr), "channel-broadcast-no-retry").await;
    let errors = Arc::new(Mutex::new(Vec::new()));
    let options = BroadcastOptions {
        max_retries: Some(0),
        error: Some(BroadcastErrorHandler::new({
            let errors = errors.clone();
            move |err| errors.lock().unwrap().push(err.clone())
        })),
    };

    t.ch1
        .lock()
        .unwrap()
        .broadcast("test", json!({"a": 1}), options)
        .unwrap();
    wait_for("the error handler", Duration::from_secs(5), || {
        !errors.lock().unwrap().is_empty()
    })
    .await;

    assert!(matches!(
        &errors.lock().unwrap()[..],
        [ClientError::Rpc { code, .. }] if code == "unknown"
    ));
    assert_eq!(1, transport.broadcast_calls.load(Ordering::SeqCst));
    t.detach().await;
}

/// JS "Polling mode does not open a watch stream".
#[tokio::test]
async fn polling_opens_no_watch_stream() {
    let Some(addr) = rpc_addr() else { return };
    let (client, transport) = FlakyBroadcastTransport::client(&addr, 0);
    client.activate().await.unwrap();
    let channel = shared_channel(&unique_key("channel-polling-no-watch"));

    client
        .attach_channel(channel.clone(), polling(None))
        .await
        .unwrap();
    wait_for("the first refresh", Duration::from_secs(8), || {
        is_attached(&channel)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(0, transport.watch_channel_calls.load(Ordering::SeqCst));
    client.detach_channel(&channel).await.unwrap();
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "changeSyncMode transitions Realtime <-> Polling for channels".
#[tokio::test]
async fn sync_mode_switches_between_realtime_and_polling() {
    let Some(addr) = rpc_addr() else { return };
    let (client, transport) = FlakyBroadcastTransport::client(&addr, 0);
    client.activate().await.unwrap();
    let channel = shared_channel(&unique_key("channel-mode-switch"));
    client
        .attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("the watch stream", Duration::from_secs(8), || {
        transport.watch_channel_calls.load(Ordering::SeqCst) == 1
    })
    .await;

    client
        .change_channel_sync_mode(&channel, SyncMode::Polling)
        .await
        .unwrap();
    assert!(session_count(&channel) >= 1);
    client
        .change_channel_sync_mode(&channel, SyncMode::Realtime)
        .await
        .unwrap();
    assert_eq!(2, transport.watch_channel_calls.load(Ordering::SeqCst));

    client.detach_channel(&channel).await.unwrap();
    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS "Polling channel does not receive broadcast events".
#[tokio::test]
async fn polling_channel_gets_no_broadcasts() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("channel-polling-no-broadcast");
    let (c1, c2) = (active_client(&addr).await, active_client(&addr).await);
    let (ch1, ch2) = (shared_channel(&key), shared_channel(&key));
    let watching = watch_initialized(&ch1);
    c2.attach_channel(ch2.clone(), polling(Some(200)))
        .await
        .unwrap();
    c1.attach_channel(ch1.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for("both channels to attach", Duration::from_secs(5), || {
        is_attached(&ch1) && is_attached(&ch2) && *watching.lock().unwrap()
    })
    .await;
    let (received, _) = collect_broadcasts(&ch2, Some("chat"));

    broadcast(&ch1, "chat", json!({"msg": "hello"}));
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(0, received_len(&received));
    for (client, channel) in [(c1, ch1), (c2, ch2)] {
        client.detach_channel(&channel).await.unwrap();
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}
