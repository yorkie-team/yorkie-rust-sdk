//! Channel integration tests against a live Yorkie server.
//!
//! JS reference: `packages/sdk/test/integration/channel_test.ts` and
//! `channel_polling_test.ts`, with `withTwoClientsAndChannels` from
//! `integration_helper.ts`. As in JS, most clients are never activated: the
//! first channel refresh activates them.

mod integration_support;

use integration_support::{rpc_addr, unique_key};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yorkie::{
    AttachChannelOptions, Channel, ChannelEvent, Client, ClientError, ClientOptions,
    DeactivateOptions, SharedChannel, SyncMode,
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
        let key = unique_key(name);
        let (c1, c2) = (client(addr), client(addr));
        let (ch1, ch2) = (shared_channel(&key), shared_channel(&key));
        c1.attach_channel(ch1.clone(), AttachChannelOptions::default())
            .await
            .unwrap();
        c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
            .await
            .unwrap();
        wait_for("both channels to attach", Duration::from_secs(10), || {
            is_attached(&ch1) && is_attached(&ch2)
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
