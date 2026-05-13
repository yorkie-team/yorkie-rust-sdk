#![forbid(unsafe_code)]
//! Core document and CRDT model for Yorkie.

#[allow(dead_code)]
mod crdt;
mod document;
mod error;
mod json;
mod time;

pub use document::Document;
pub use error::{Result, YorkieError};
pub use json::{JsonArray, JsonObject, JsonValue};
pub use time::{
    ActorId, TimeTicket, TimeTicketStruct, VersionVector, INITIAL_ACTOR_ID, INITIAL_DELIMITER,
    INITIAL_LAMPORT, MAX_ACTOR_ID, MAX_DELIMITER, MAX_LAMPORT, TIME_TICKET_SIZE,
};
