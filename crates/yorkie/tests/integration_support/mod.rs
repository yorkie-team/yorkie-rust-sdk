//! Shared helpers for integration tests against a live Yorkie server.
#![allow(dead_code)]

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
