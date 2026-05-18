#![forbid(unsafe_code)]
//! Core document and CRDT model for Yorkie.

#[allow(dead_code)]
mod change;
#[allow(dead_code)]
mod crdt;
mod document;
mod error;
mod json;
#[allow(dead_code)]
mod operation;
mod time;
pub mod wire;

pub use change::{
    ChangePack, Checkpoint, INITIAL_CHECKPOINT, INITIAL_CLIENT_SEQ, INITIAL_SERVER_SEQ,
    MAX_CHECKPOINT, MAX_CLIENT_SEQ, MAX_SERVER_SEQ,
};
pub use crdt::counter::{CounterType, CounterValue};
pub use document::Document;
pub use error::{Result, YorkieError};
pub use json::{JsonArray, JsonArrayElement, JsonCounter, JsonObject, JsonValue};
pub use time::{
    ActorId, TimeTicket, TimeTicketStruct, VersionVector, INITIAL_ACTOR_ID, INITIAL_DELIMITER,
    INITIAL_LAMPORT, MAX_ACTOR_ID, MAX_DELIMITER, MAX_LAMPORT, TIME_TICKET_SIZE,
};
