//! Shared helpers for integration tests against a live Yorkie server.
#![allow(dead_code)]

pub mod events;
pub mod stub_transport;
pub mod tree;
pub mod two_clients;

use std::time::{SystemTime, UNIX_EPOCH};

/// Server address from `YORKIE_TEST_RPC_ADDR`, or `None` to skip the test.
pub fn rpc_addr() -> Option<String> {
    match std::env::var("YORKIE_TEST_RPC_ADDR") {
        Ok(addr) if !addr.is_empty() => Some(addr),
        _ => {
            eprintln!("skipping: YORKIE_TEST_RPC_ADDR is not set");
            None
        }
    }
}

/// Unique key so tests can run in parallel on one server.
pub fn unique_key(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("{prefix}-{millis}")
}

/// Polls `condition` every 10ms until it holds, failing after five seconds.
pub async fn eventually(what: &str, condition: impl FnMut() -> bool) {
    eventually_within(what, std::time::Duration::from_secs(5), condition).await;
}

/// Polls `condition` every 10ms until it holds, failing after `timeout`.
pub async fn eventually_within(
    what: &str,
    timeout: std::time::Duration,
    mut condition: impl FnMut() -> bool,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Collects values from a document subscription and waits for them (JS
/// `EventCollector` in `test/helper/helper.ts`).
pub struct EventCollector<T>(std::sync::Arc<std::sync::Mutex<Vec<T>>>);

impl<T: Clone + PartialEq + std::fmt::Debug + Send + 'static> EventCollector<T> {
    pub fn new() -> Self {
        Self(Default::default())
    }

    /// Returns a closure that records each value it is given.
    pub fn sink(&self) -> impl FnMut(T) + Send + 'static {
        let events = self.0.clone();
        move |value| events.lock().unwrap().push(value)
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// Drops the values collected so far (JS `reset`).
    pub fn reset(&self) {
        self.0.lock().unwrap().clear();
    }

    /// The values collected so far.
    pub fn values(&self) -> Vec<T> {
        self.0.lock().unwrap().clone()
    }

    /// Waits until a value equal to `expected` has arrived (JS `waitFor`).
    pub async fn wait_for(&self, expected: T) {
        eventually(&format!("{expected:?}"), || {
            self.0.lock().unwrap().contains(&expected)
        })
        .await;
    }

    /// Waits until `n` values have arrived and checks the `n`-th (JS
    /// `waitAndVerifyNthEvent`).
    pub async fn wait_and_verify_nth(&self, n: usize, expected: T) {
        eventually(&format!("event {n}"), || self.len() >= n).await;
        assert_eq!(expected, self.0.lock().unwrap()[n - 1], "event {n}");
    }
}

/// Undoes back to the first of `states`, redoes to the last, then undoes
/// again, checking the document against each state on the way (JS
/// `assertUndoRedo` in `integration_helper.ts`).
pub fn assert_undo_redo(doc: &mut yorkie::Document, states: &[String]) {
    let n = states.len();
    for i in 0..n - 1 {
        doc.history_mut().undo().unwrap();
        assert_eq!(states[n - 2 - i], doc.to_sorted_json(), "undo 1-{i}");
    }
    for i in 0..n - 1 {
        doc.history_mut().redo().unwrap();
        assert_eq!(states[i + 1], doc.to_sorted_json(), "redo{i}");
    }
    for i in 0..n - 1 {
        doc.history_mut().undo().unwrap();
        assert_eq!(states[n - 2 - i], doc.to_sorted_json(), "undo 2-{i}");
    }
}

/// A version vector that covers every ticket of the given actors, or of the
/// initial actor when none is given (JS `maxVectorOf` in
/// `test/helper/helper.ts`).
pub fn max_vector_of(actors: &[&yorkie::ActorId]) -> yorkie::VersionVector {
    let mut vector = yorkie::VersionVector::new();
    if actors.is_empty() {
        vector.set("000000000000000000000000", i64::MAX);
    }
    for actor in actors {
        vector.set((*actor).clone(), i64::MAX);
    }
    vector
}

/// A version vector of the given actor and lamport pairs (JS `vectorOf`).
pub fn vector_of(entries: &[(&yorkie::ActorId, i64)]) -> yorkie::VersionVector {
    let mut vector = yorkie::VersionVector::new();
    for (actor, lamport) in entries {
        vector.set((*actor).clone(), *lamport);
    }
    vector
}

/// Builds a document value from a JSON literal, the way a JS test assigns an
/// object or array literal: numbers that fit an `i32` become Integers, other
/// integers Longs, and the rest Doubles.
pub fn json_value(value: serde_json::Value) -> yorkie::JsonValue {
    use serde_json::Value;
    use yorkie::{JsonArray, JsonObject, JsonValue};
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(value) => JsonValue::Bool(value),
        Value::Number(number) => match number.as_i64() {
            Some(value) => match i32::try_from(value) {
                Ok(value) => JsonValue::Integer(value),
                Err(_) => JsonValue::Long(value),
            },
            None => JsonValue::Double(number.as_f64().expect("a finite number")),
        },
        Value::String(value) => JsonValue::String(value),
        Value::Array(values) => {
            let mut array = JsonArray::new();
            for value in values {
                array.push(json_value(value)).unwrap();
            }
            JsonValue::Array(array)
        }
        Value::Object(members) => {
            let mut object = JsonObject::new();
            for (key, value) in members {
                object.set(key, json_value(value)).unwrap();
            }
            JsonValue::Object(object)
        }
    }
}

/// The presence the document stores for `actor`, whether or not that client
/// is online (JS `getPresenceForTest`). `Document::presence` only answers for
/// online clients, so this marks the client online for the read and then
/// restores the online set.
pub fn presence_for_test(
    doc: &yorkie::SharedDocument,
    actor: &yorkie::ActorId,
) -> Option<yorkie::Presence> {
    let mut doc = doc.lock().unwrap();
    if !doc.has_presence(actor) {
        return None;
    }
    if let Some(presence) = doc.presence(actor) {
        return Some(presence);
    }
    doc.add_online_client(actor.clone());
    let presence = doc.presence(actor);
    doc.remove_online_client(actor);
    presence
}
