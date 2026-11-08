//! Channel session count ("presence counter") integration tests against a
//! live server.
//!
//! JS reference: `packages/sdk/test/integration/presence_test.ts`. A detach
//! is local, so peers see the count fall only after the server reclaims the
//! session by its TTL (15s) and the next refresh; those cases take up to
//! ~25s, as in JS.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::two_clients::active_client;
use integration_support::{deactivate, eventually_within, rpc_addr, unique_key};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yorkie::{
    AttachChannelOptions, Channel, ChannelEvent, ChannelStatus, Client, ClientOptions,
    SharedChannel, SyncMode,
};

fn shared_channel(key: &str) -> SharedChannel {
    Arc::new(Mutex::new(Channel::new(key).unwrap()))
}

fn manual() -> AttachChannelOptions {
    AttachChannelOptions {
        sync_mode: Some(SyncMode::Manual),
        ..AttachChannelOptions::default()
    }
}

fn session_count(channel: &SharedChannel) -> u64 {
    channel.lock().unwrap().session_count()
}

fn is_attached(channel: &SharedChannel) -> bool {
    channel.lock().unwrap().is_attached()
}

fn status(channel: &SharedChannel) -> ChannelStatus {
    channel.lock().unwrap().status()
}

/// Waits until the channel's first refresh has set the server-issued session
/// ID and marked it attached (JS `waitForAttached`, 8s). `attach_channel`
/// returns before that happens.
async fn wait_for_attached(channel: &SharedChannel) {
    eventually_within("the channel to attach", Duration::from_secs(8), || {
        let channel = channel.lock().unwrap();
        channel.is_attached() && channel.session_id().is_some()
    })
    .await;
}

/// Waits until the channel's session count is `expected` (JS
/// `waitForCount`, 25s unless given).
async fn wait_for_count(channel: &SharedChannel, expected: u64, timeout: Option<Duration>) {
    eventually_within(
        &format!("a session count of {expected}"),
        timeout.unwrap_or(Duration::from_secs(25)),
        || session_count(channel) == expected,
    )
    .await;
}

const EIGHT_SECONDS: Option<Duration> = Some(Duration::from_secs(8));

/// JS "single client presence counter test" (presence_test.ts:64).
#[tokio::test]
async fn single_client_presence_counter() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;

    // Create presence counter
    let channel_key = unique_key("presence");
    let channel = shared_channel(&channel_key);

    // Test initial state
    assert_eq!(channel_key, channel.lock().unwrap().key());
    assert_eq!(ChannelStatus::Detached, status(&channel));
    assert!(!is_attached(&channel));
    assert_eq!(0, session_count(&channel));

    // Attach presence counter (returns before server-side attach completes)
    c1.attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&channel).await;

    // Verify attached state
    assert_eq!(ChannelStatus::Attached, status(&channel));
    assert!(is_attached(&channel));
    assert_eq!(1, session_count(&channel));

    // Detach presence counter
    c1.detach_channel(&channel).await.unwrap();

    // Verify detached state
    assert_eq!(ChannelStatus::Detached, status(&channel));
    assert!(!is_attached(&channel));

    deactivate(&[&c1]).await;
}

/// JS "multiple clients presence counter test" (presence_test.ts:98).
#[tokio::test]
async fn multiple_clients_presence_counter() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;

    // Create channels for the same room
    let channel_key = unique_key("presence-room");
    let ch1 = shared_channel(&channel_key);
    let ch2 = shared_channel(&channel_key);
    let ch3 = shared_channel(&channel_key);

    // First client attaches
    c1.attach_channel(ch1.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&ch1).await;
    assert_eq!(1, session_count(&ch1));

    // Second client attaches
    c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&ch2).await;
    assert_eq!(2, session_count(&ch2));

    // First client should receive the update via Realtime watch stream
    wait_for_count(&ch1, 2, EIGHT_SECONDS).await;

    // Third client attaches
    c3.attach_channel(ch3.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&ch3).await;
    assert_eq!(3, session_count(&ch3));

    // Wait for the other Realtime peers to observe count=3
    wait_for_count(&ch1, 3, EIGHT_SECONDS).await;
    wait_for_count(&ch2, 3, EIGHT_SECONDS).await;

    // One client detaches (local-only; peers learn about it via the server's
    // TTL-driven session reclamation and the next heartbeat). ch2's own
    // count is not checked: it is detached and keeps the last-seen value.
    c2.detach_channel(&ch2).await.unwrap();

    // Other clients should see the count decrease within the TTL window
    wait_for_count(&ch1, 2, None).await;
    wait_for_count(&ch3, 2, None).await;

    // Cleanup
    c1.detach_channel(&ch1).await.unwrap();
    c3.detach_channel(&ch3).await.unwrap();
    deactivate(&[&c1, &c2, &c3]).await;
}

/// JS "presence event subscription test" (presence_test.ts:153).
#[tokio::test]
async fn presence_event_subscription() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    let channel_key = unique_key("presence-events");
    let ch1 = shared_channel(&channel_key);
    let ch2 = shared_channel(&channel_key);

    // Track events on ch1
    let events = Arc::new(Mutex::new(Vec::new()));
    ch1.lock().unwrap().subscribe({
        let events = events.clone();
        move |event| events.lock().unwrap().push(event.clone())
    });

    // First client attaches
    c1.attach_channel(ch1.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&ch1).await;

    // The first heartbeat fires before the watch stream opens, so the first
    // event is a `PresenceChanged` from the heartbeat response, or an
    // `Initialized` from the watch stream.
    {
        let events = events.lock().unwrap();
        assert!(!events.is_empty());
        match &events[0] {
            ChannelEvent::PresenceChanged { count } | ChannelEvent::Initialized { count } => {
                assert_eq!(1, *count);
            }
            other => panic!("unexpected first event {other:?}"),
        }
    }

    // Second client attaches
    c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&ch2).await;
    wait_for_count(&ch1, 2, EIGHT_SECONDS).await;

    // Should have observed the count transition to 2
    let presence_counts: Vec<u64> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            ChannelEvent::PresenceChanged { count } => Some(*count),
            _ => None,
        })
        .collect();
    assert!(!presence_counts.is_empty());
    assert_eq!(Some(&2), presence_counts.last());

    // Cleanup
    c1.detach_channel(&ch1).await.unwrap();
    c2.detach_channel(&ch2).await.unwrap();
    deactivate(&[&c1, &c2]).await;
}

/// JS "presence detach reduces count test" (presence_test.ts:214).
#[tokio::test]
async fn presence_detach_reduces_count() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    let channel_key = unique_key("presence-detach");
    let ch1 = shared_channel(&channel_key);
    let ch2 = shared_channel(&channel_key);

    // Both attach
    c1.attach_channel(ch1.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    c2.attach_channel(ch2.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&ch1).await;
    wait_for_attached(&ch2).await;
    wait_for_count(&ch1, 2, EIGHT_SECONDS).await;
    wait_for_count(&ch2, 2, EIGHT_SECONDS).await;

    // One detaches (local-only; peers learn via TTL + heartbeat)
    c1.detach_channel(&ch1).await.unwrap();

    // Other channels should see the decrease once the server reclaims c1's
    // session and ch2's next heartbeat returns the updated count.
    wait_for_count(&ch2, 1, None).await;

    // Cleanup
    c2.detach_channel(&ch2).await.unwrap();
    deactivate(&[&c1, &c2]).await;
}

/// JS "channel heartbeat keeps session alive" (presence_test.ts:247).
#[tokio::test]
async fn channel_heartbeat_keeps_the_session_alive() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = Client::new(ClientOptions {
        rpc_addr: addr.clone(),
        channel_heartbeat_interval: Duration::from_millis(1000),
        ..ClientOptions::default()
    })
    .unwrap();
    c1.activate().await.unwrap();

    let channel = shared_channel(&unique_key("presence-heartbeat"));

    // Attach presence
    c1.attach_channel(channel.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&channel).await;
    assert_eq!(1, session_count(&channel));

    // Wait for 3 heartbeat cycles. The presence should still be active
    // because the heartbeat refreshes the TTL.
    tokio::time::sleep(Duration::from_millis(3500)).await;

    // Verify presence is still active
    assert!(is_attached(&channel));
    assert_eq!(1, session_count(&channel));

    // Cleanup
    c1.detach_channel(&channel).await.unwrap();
    deactivate(&[&c1]).await;
}

/// JS "presence manual sync mode test" (presence_test.ts:277).
#[tokio::test]
async fn presence_manual_sync_mode() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;

    // Create presences for the same room
    let channel_key = unique_key("presence-manual");
    let ch1 = shared_channel(&channel_key);
    let ch2 = shared_channel(&channel_key);

    // Attach client1 with manual sync mode (no watch stream). Manual mode
    // does not auto-refresh, so sync once to send the first refresh and get
    // the session count.
    c1.attach_channel(ch1.clone(), manual()).await.unwrap();
    c1.sync_channel(&ch1).await.unwrap();
    assert_eq!(1, session_count(&ch1));

    // Attach client2 with manual sync mode
    c2.attach_channel(ch2.clone(), manual()).await.unwrap();
    c2.sync_channel(&ch2).await.unwrap();
    assert_eq!(2, session_count(&ch2));

    // In manual mode, p1's count doesn't update automatically even though p2
    // was attached. An absence can only be checked over a window.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(1, session_count(&ch1), "p1 should still be 1");

    // Must call sync() explicitly to refresh TTL and fetch latest count
    c1.sync_channel(&ch1).await.unwrap();
    assert_eq!(2, session_count(&ch1), "p1 should update to 2 after sync");

    // Detach p2 and verify p1 doesn't auto-update
    c2.detach_channel(&ch2).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(2, session_count(&ch1), "p1 should still be 2");

    // The detach is local: the server reclaims c2's session only after its
    // TTL elapses (~15s). Poll-sync until the server count drops to 1.
    let drop_deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while session_count(&ch1) != 1 {
        assert!(
            tokio::time::Instant::now() < drop_deadline,
            "manual-mode peer did not observe count drop within TTL window (got {})",
            session_count(&ch1)
        );
        c1.sync_channel(&ch1).await.unwrap();
        if session_count(&ch1) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    assert_eq!(1, session_count(&ch1), "p1 eventually observes 1");

    // Cleanup
    c1.detach_channel(&ch1).await.unwrap();
    deactivate(&[&c1, &c2]).await;
}

/// JS "presence realtime vs manual mode comparison test"
/// (presence_test.ts:339).
#[tokio::test]
async fn presence_realtime_and_manual_modes_compared() {
    let Some(addr) = rpc_addr() else { return };
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let c3 = active_client(&addr).await;

    let channel_key = unique_key("presence-mode-compare");
    let realtime_ch = shared_channel(&channel_key);
    let manual_ch = shared_channel(&channel_key);
    let third_ch = shared_channel(&channel_key);

    // c1: Attach with realtime mode (default)
    c1.attach_channel(realtime_ch.clone(), AttachChannelOptions::default())
        .await
        .unwrap();
    wait_for_attached(&realtime_ch).await;
    assert_eq!(1, session_count(&realtime_ch));

    // c2: Attach with manual mode. Manual mode skips auto-refresh; an
    // explicit sync sends the first refresh and gets the session count.
    c2.attach_channel(manual_ch.clone(), manual())
        .await
        .unwrap();
    c2.sync_channel(&manual_ch).await.unwrap();
    assert_eq!(2, session_count(&manual_ch));

    // c1's realtime presence should automatically receive the update
    wait_for_count(&realtime_ch, 2, EIGHT_SECONDS).await;
    assert_eq!(
        2,
        session_count(&realtime_ch),
        "realtime presence should auto-update"
    );

    // c2's manual presence doesn't receive updates
    assert_eq!(
        2,
        session_count(&manual_ch),
        "manual presence should not auto-update"
    );

    // c3: Attach another client (manual mode)
    c3.attach_channel(third_ch.clone(), manual()).await.unwrap();
    c3.sync_channel(&third_ch).await.unwrap();
    assert_eq!(3, session_count(&third_ch));

    // c1's realtime presence receives the update automatically
    wait_for_count(&realtime_ch, 3, EIGHT_SECONDS).await;
    assert_eq!(
        3,
        session_count(&realtime_ch),
        "realtime presence should be 3"
    );

    // c2's manual presence still doesn't update
    assert_eq!(2, session_count(&manual_ch), "manual presence should be 2");

    // Cleanup
    c1.detach_channel(&realtime_ch).await.unwrap();
    c2.detach_channel(&manual_ch).await.unwrap();
    c3.detach_channel(&third_ch).await.unwrap();
    deactivate(&[&c1, &c2, &c3]).await;
}
