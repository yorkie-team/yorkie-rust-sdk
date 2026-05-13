mod actor_id;
mod ticket;
mod version_vector;

pub use actor_id::{ActorId, INITIAL_ACTOR_ID, MAX_ACTOR_ID};
pub use ticket::{
    TimeTicket, TimeTicketStruct, INITIAL_DELIMITER, INITIAL_LAMPORT, MAX_DELIMITER, MAX_LAMPORT,
    TIME_TICKET_SIZE,
};
pub use version_vector::VersionVector;
