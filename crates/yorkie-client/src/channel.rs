//! Channels: lightweight shared rooms that count their sessions and carry
//! broadcast messages.
//!
//! JS reference: `packages/sdk/src/channel/channel.ts` (`Channel`,
//! `ChannelStatus`, `ChannelEventType`, `BroadcastOptions`, `subscribe`,
//! `publish`, `broadcast`, `updateSessionCount`, `validateChannelKey`).
//!
//! A channel is attached with `Client::attach_channel`. The client then
//! refreshes its session on the server every heartbeat interval; the first
//! refresh creates the session and marks the channel attached. In `Realtime`
//! the client also watches the channel, which delivers count changes and
//! broadcasts as they happen.

use crate::error::{ClientError, ClientResult};
use serde_json::Value;
use std::fmt::{self, Debug, Formatter};
use std::sync::{Arc, Mutex};
use yorkie_core::ActorId;

/// A channel shared between the application and the client that attaches it
/// (JS passes the same `Channel` object to `client.attach`).
///
/// As with `SharedDocument`, the client locks it only for synchronous steps
/// and never across an `.await`. Channel event callbacks run while the client
/// holds this lock, so a callback must not lock the same channel again: that
/// deadlocks.
pub type SharedChannel = Arc<Mutex<Channel>>;

/// Separates the key paths of a channel key (JS `KeyPathSeparator`).
const KEY_PATH_SEPARATOR: char = '.';

/// Whether a channel is attached (JS `ChannelStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    Detached,
    Attached,
}

/// An event that occurs in a channel (JS `ChannelEvent`).
///
/// JS `subscribe('presence', cb)` takes `PresenceChanged` and `Initialized`,
/// `subscribe('broadcast', cb)` takes `Broadcast`; in Rust, match on the
/// variant.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelEvent {
    /// The session count changed (JS `presence-changed`).
    PresenceChanged { count: u64 },
    /// The watch stream opened and reported the session count (JS
    /// `initialized`).
    Initialized { count: u64 },
    /// Another client broadcast a message (JS `broadcast`). The server does
    /// not send a client its own broadcasts.
    Broadcast {
        client_id: ActorId,
        topic: String,
        payload: Value,
    },
    /// This channel broadcast a message with `Channel::broadcast` (JS
    /// `local-broadcast`). The client that attached the channel sends it.
    LocalBroadcast {
        client_id: ActorId,
        topic: String,
        payload: Value,
        options: BroadcastOptions,
    },
    /// A sync error the client does not recover from by itself, such as a
    /// failed session refresh (JS `sync-error`). The client keeps retrying,
    /// so a later event means it recovered.
    SyncError { error: ClientError, method: String },
}

/// Options for a broadcast (JS `BroadcastOptions`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BroadcastOptions {
    /// How many times a broadcast that failed with a retryable error is sent
    /// again, waiting 1s, 2s, 4s, and so on up to 20s in between; `None`
    /// retries without limit (JS `maxRetries`, default `Infinity`).
    pub max_retries: Option<u32>,
    /// Called with the error when a broadcast made with `Channel::broadcast`
    /// fails for good (JS `error`).
    pub error: Option<BroadcastErrorHandler>,
}

/// The callback of [`BroadcastOptions::error`]. Two handlers are equal when
/// they are the same callback.
#[derive(Clone)]
pub struct BroadcastErrorHandler(Arc<dyn Fn(&ClientError) + Send + Sync>);

impl BroadcastErrorHandler {
    pub fn new(handler: impl Fn(&ClientError) + Send + Sync + 'static) -> Self {
        Self(Arc::new(handler))
    }

    pub(crate) fn call(&self, err: &ClientError) {
        (self.0)(err);
    }
}

impl Debug for BroadcastErrorHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("BroadcastErrorHandler")
    }
}

impl PartialEq for BroadcastErrorHandler {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Identifies a subscription made with `Channel::subscribe` or
/// `Channel::subscribe_topic` (JS `Unsubscribe` handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelSubscriptionId(u64);

type Callback = Box<dyn FnMut(&ChannelEvent) + Send>;

struct Subscriber {
    id: ChannelSubscriptionId,
    /// Only broadcasts of this topic reach the callback.
    topic: Option<String>,
    callback: Callback,
}

/// A channel that counts the sessions attached to it (JS `Channel`).
pub struct Channel {
    key: String,
    status: ChannelStatus,
    actor_id: Option<ActorId>,
    session_id: Option<String>,
    session_count: u64,
    seq: u64,
    next_subscription_id: u64,
    subscribers: Vec<Subscriber>,
}

impl Channel {
    /// Creates a detached channel. The key must not be empty, contain a
    /// space, start or end with `.`, or contain an empty key path (JS
    /// `validateChannelKey`).
    pub fn new(key: impl Into<String>) -> ClientResult<Self> {
        let key = key.into();
        validate_channel_key(&key)?;
        Ok(Self {
            key,
            status: ChannelStatus::Detached,
            actor_id: None,
            session_id: None,
            session_count: 0,
            seq: 0,
            next_subscription_id: 0,
            subscribers: Vec::new(),
        })
    }

    /// Returns the key of this channel (JS `getKey`).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the first key path, which the server shards channels by (JS
    /// `getFirstKeyPath`).
    pub fn first_key_path(&self) -> &str {
        first_key_path(&self.key)
    }

    /// JS `getStatus`.
    pub fn status(&self) -> ChannelStatus {
        self.status
    }

    /// JS `isAttached`.
    pub fn is_attached(&self) -> bool {
        self.status == ChannelStatus::Attached
    }

    /// Returns the actor of the client this channel is attached to (JS
    /// `getActorID`).
    pub fn actor_id(&self) -> Option<&ActorId> {
        self.actor_id.as_ref()
    }

    /// Returns the session ID the server issued on the first refresh (JS
    /// `getSessionID`).
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Replaces the session ID; `None` makes the next refresh create a new
    /// session (JS `setSessionID`, where `''` clears it).
    #[doc(hidden)]
    pub fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    /// Returns how many sessions the channel had at the last update (JS
    /// `getSessionCount`).
    pub fn session_count(&self) -> u64 {
        self.session_count
    }

    /// Registers a callback for every event of this channel (JS
    /// `subscribe('all', cb)` or `subscribe(cb)`). Callbacks run
    /// synchronously while the caller holds `&mut Channel`.
    pub fn subscribe(
        &mut self,
        callback: impl FnMut(&ChannelEvent) + Send + 'static,
    ) -> ChannelSubscriptionId {
        self.add_subscriber(None, Box::new(callback))
    }

    /// Registers a callback for the `Broadcast` events of one topic (JS
    /// `subscribe(topic, cb)`). Callbacks run as in `subscribe`.
    pub fn subscribe_topic(
        &mut self,
        topic: impl Into<String>,
        callback: impl FnMut(&ChannelEvent) + Send + 'static,
    ) -> ChannelSubscriptionId {
        self.add_subscriber(Some(topic.into()), Box::new(callback))
    }

    /// Removes a subscription. Unknown IDs are ignored.
    pub fn unsubscribe(&mut self, id: ChannelSubscriptionId) {
        self.subscribers.retain(|subscriber| subscriber.id != id);
    }

    /// Sends `payload` under `topic` to the other clients watching this
    /// channel (JS `broadcast`). The channel must be attached.
    ///
    /// As in JS, this only publishes `LocalBroadcast`: the client that
    /// attached the channel picks it up and sends it in the background,
    /// retrying as `options` says. If it fails for good, `options.error`
    /// gets the error. `Client::broadcast` sends one and waits for it.
    pub fn broadcast(
        &mut self,
        topic: impl Into<String>,
        payload: Value,
        options: BroadcastOptions,
    ) -> ClientResult<()> {
        let (ChannelStatus::Attached, Some(client_id)) = (self.status, self.actor_id.clone())
        else {
            return Err(ClientError::NotAttached(self.key.clone()));
        };
        self.publish(ChannelEvent::LocalBroadcast {
            client_id,
            topic: topic.into(),
            payload,
            options,
        });
        Ok(())
    }

    fn add_subscriber(
        &mut self,
        topic: Option<String>,
        callback: Callback,
    ) -> ChannelSubscriptionId {
        let id = ChannelSubscriptionId(self.next_subscription_id);
        self.next_subscription_id += 1;
        self.subscribers.push(Subscriber {
            id,
            topic,
            callback,
        });
        id
    }

    /// Delivers an event to this channel's subscribers (JS `publish`).
    pub(crate) fn publish(&mut self, event: ChannelEvent) {
        for subscriber in &mut self.subscribers {
            let wanted = match (&subscriber.topic, &event) {
                (None, _) => true,
                (Some(wanted), ChannelEvent::Broadcast { topic, .. }) => wanted == topic,
                (Some(_), _) => false,
            };
            if wanted {
                (subscriber.callback)(&event);
            }
        }
    }

    /// JS `applyStatus`.
    pub(crate) fn apply_status(&mut self, status: ChannelStatus) {
        self.status = status;
    }

    /// JS `setActor`.
    pub(crate) fn set_actor(&mut self, actor_id: ActorId) {
        self.actor_id = Some(actor_id);
    }

    /// Takes a session count unless it is older than the last one taken:
    /// `seq` 0 (a refresh response or a watch initialization) always wins,
    /// otherwise only a newer `seq` does (JS `updateSessionCount`). Returns
    /// whether the count was taken.
    pub(crate) fn update_session_count(&mut self, session_count: u64, seq: u64) -> bool {
        if seq == 0 || seq > self.seq {
            self.session_count = session_count;
            self.seq = seq;
            return true;
        }
        false
    }
}

impl Debug for Channel {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("key", &self.key)
            .field("status", &self.status)
            .field("actor_id", &self.actor_id)
            .field("session_id", &self.session_id)
            .field("session_count", &self.session_count)
            .field("seq", &self.seq)
            .field("subscribers", &self.subscribers.len())
            .finish()
    }
}

/// The first key path of a channel key (JS `getFirstKeyPath`).
pub(crate) fn first_key_path(key: &str) -> &str {
    key.split(KEY_PATH_SEPARATOR).next().unwrap_or(key)
}

/// JS `validateChannelKey`.
fn validate_channel_key(key: &str) -> ClientResult<()> {
    let invalid = |message: &str| Err(ClientError::InvalidArgument(message.to_owned()));
    if key.is_empty() {
        return invalid("channel key must not be empty");
    }
    if key.contains(' ') {
        return invalid("channel key must not contain a whitespace");
    }
    if key.starts_with(KEY_PATH_SEPARATOR) {
        return invalid("channel key must not start with a period");
    }
    if key.ends_with(KEY_PATH_SEPARATOR) {
        return invalid("channel key must not end with a period");
    }
    if key.contains("..") {
        return invalid("channel key path must not empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JS `validateChannelKey`.
    #[test]
    fn rejects_invalid_channel_keys() {
        for (key, message) in [
            ("", "channel key must not be empty"),
            ("a b", "channel key must not contain a whitespace"),
            (".room", "channel key must not start with a period"),
            ("room.", "channel key must not end with a period"),
            ("room..1", "channel key path must not empty"),
        ] {
            assert_eq!(
                ClientError::InvalidArgument(message.to_owned()),
                Channel::new(key).unwrap_err(),
                "{key:?}"
            );
        }

        let channel = Channel::new("room.1.cursor").unwrap();
        assert_eq!("room.1.cursor", channel.key());
        assert_eq!("room", channel.first_key_path());
        assert_eq!(ChannelStatus::Detached, channel.status());
        assert!(!channel.is_attached());
        assert_eq!(None, channel.session_id());
        assert_eq!(0, channel.session_count());
    }

    /// JS `updateSessionCount`: seq 0 always applies, otherwise only a newer
    /// seq does.
    #[test]
    fn session_count_ignores_stale_updates() {
        let mut channel = Channel::new("room").unwrap();
        assert!(channel.update_session_count(2, 5));
        assert!(!channel.update_session_count(1, 4));
        assert!(!channel.update_session_count(1, 5));
        assert_eq!(2, channel.session_count());
        assert!(channel.update_session_count(3, 6));
        assert!(channel.update_session_count(1, 0));
        assert_eq!(1, channel.session_count());
        assert!(channel.update_session_count(4, 1));
        assert_eq!(4, channel.session_count());
    }

    /// JS `subscribe(topic, cb)` takes only the broadcasts of that topic.
    #[test]
    fn topic_subscribers_get_only_their_broadcasts() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut channel = Channel::new("room").unwrap();
        channel.subscribe_topic("chat", {
            let events = events.clone();
            move |event| events.lock().unwrap().push(event.clone())
        });
        let broadcast = |topic: &str| ChannelEvent::Broadcast {
            client_id: ActorId::new("000000000000000000000001"),
            topic: topic.to_owned(),
            payload: Value::from(topic),
        };

        channel.publish(broadcast("notification"));
        channel.publish(ChannelEvent::PresenceChanged { count: 2 });
        channel.publish(broadcast("chat"));

        assert_eq!(vec![broadcast("chat")], *events.lock().unwrap());
    }

    /// JS `broadcast`: only an attached channel broadcasts, and it publishes
    /// `LocalBroadcast` for the client to send.
    #[test]
    fn broadcast_publishes_a_local_broadcast_when_attached() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut channel = Channel::new("room").unwrap();
        channel.subscribe({
            let events = events.clone();
            move |event| events.lock().unwrap().push(event.clone())
        });
        assert_eq!(
            Err(ClientError::NotAttached("room".to_owned())),
            channel.broadcast("chat", Value::from(1), BroadcastOptions::default())
        );

        let client_id = ActorId::new("000000000000000000000001");
        channel.set_actor(client_id.clone());
        channel.apply_status(ChannelStatus::Attached);
        let options = BroadcastOptions {
            max_retries: Some(0),
            error: Some(BroadcastErrorHandler::new(|_| {})),
        };
        channel
            .broadcast("chat", Value::from(1), options.clone())
            .unwrap();

        assert_eq!(
            vec![ChannelEvent::LocalBroadcast {
                client_id,
                topic: "chat".to_owned(),
                payload: Value::from(1),
                options,
            }],
            *events.lock().unwrap()
        );
    }

    #[test]
    fn unsubscribed_callbacks_get_no_events() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut channel = Channel::new("room").unwrap();
        let id = channel.subscribe({
            let events = events.clone();
            move |event| events.lock().unwrap().push(event.clone())
        });

        channel.publish(ChannelEvent::PresenceChanged { count: 1 });
        channel.unsubscribe(id);
        channel.publish(ChannelEvent::PresenceChanged { count: 2 });

        assert_eq!(
            vec![ChannelEvent::PresenceChanged { count: 1 }],
            *events.lock().unwrap()
        );
        assert!(format!("{channel:?}").contains("subscribers: 0"));
    }
}
