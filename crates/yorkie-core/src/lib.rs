#![forbid(unsafe_code)]
//! Core document and CRDT model for Yorkie.

#[allow(dead_code)]
mod change;
#[allow(dead_code)]
mod crdt;
mod document;
mod error;
mod event;
mod history;
mod json;
#[allow(dead_code)]
mod operation;
mod presence;
// Wired into `Document` by the pre-attach re-issue entry point.
#[allow(dead_code)]
mod reissue;
mod schema;
mod time;
/// Protocol-shaped projections for `yorkie-protocol`. Public only so that
/// crate can reach them; not part of the supported API.
#[doc(hidden)]
pub mod wire;

pub use change::{
    ChangePack, Checkpoint, INITIAL_CHECKPOINT, INITIAL_CLIENT_SEQ, INITIAL_SERVER_SEQ,
    MAX_CHECKPOINT, MAX_CLIENT_SEQ, MAX_SERVER_SEQ,
};
pub use crdt::counter::{CounterType, CounterValue};
pub use crdt::element::DataSize;
pub use crdt::root::DocSize;
pub use document::{DocStatus, Document};
pub use error::{ErrorCode, Result, YorkieError};
pub use event::{
    ChangeInfo, DocEvent, DocSyncStatus, EditOpValue, OperationInfo, StreamConnectionStatus,
    StyleOpValue, SubscriptionId, TreeStyleOpValue,
};
pub use history::{HistoryMut, HistoryRef};
pub use json::{
    JsonArray, JsonArrayElement, JsonCounter, JsonObject, JsonText, JsonTree, JsonValue, TextChunk,
    TreeNodeInput,
};
pub use presence::{Presence, PresenceChange, PresenceProxy};
pub use schema::{SchemaRule, TreeNodeRule};
pub use time::{
    ActorId, TimeTicket, TimeTicketStruct, VersionVector, INITIAL_ACTOR_ID, INITIAL_DELIMITER,
    INITIAL_LAMPORT, MAX_ACTOR_ID, MAX_DELIMITER, MAX_LAMPORT, TIME_TICKET_SIZE,
};
