#![forbid(unsafe_code)]
//! Core document and CRDT model for Yorkie.

mod document;
mod error;
mod json;

pub use document::Document;
pub use error::{Result, YorkieError};
pub use json::{JsonArray, JsonObject, JsonValue};
