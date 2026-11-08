//! Shared helpers for integration tests against a live Yorkie server.
#![allow(dead_code)]

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
pub async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
