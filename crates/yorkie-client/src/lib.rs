#![forbid(unsafe_code)]
//! Network client layer for Yorkie.

pub use yorkie_core::{Document, JsonArray, JsonObject, JsonValue, Result, YorkieError};

/// User-settable options for a Yorkie client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientOptions {
    pub rpc_addr: Option<String>,
    pub api_key: Option<String>,
}

/// A placeholder client type for the initial SDK scaffold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Client {
    options: ClientOptions,
}

impl Client {
    pub fn new(options: ClientOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &ClientOptions {
        &self.options
    }
}
