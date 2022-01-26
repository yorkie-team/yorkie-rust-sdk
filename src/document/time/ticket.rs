use std::{u32, u64};
use std::str;
use crate::document::time::actor_id::{ActorID};

const MAX_LAMPORT: u64 = u64::MAX;
const MAX_DELIMITER: u32 = u32::MAX;

/// Ticket is a timestamp of the logical clock. Ticket is immutable.
struct Ticket {
    lamport: u64,
    delimiter: u32,
    actor_id: ActorID,
}

impl Ticket {
    pub fn new(lamport: u64, delimiter: u32, actor_id: ActorID) -> Ticket {
        Ticket{lamport, delimiter, actor_id}
    }

    /// annotated_string returns a string containing the metadata of the ticket
    /// for debugging purpose.
    pub fn annotated_string(&self) -> Result<String, str::Utf8Error> {
        let id = self.actor_id.as_str()?;
        Ok(format!("{}:{}:{}", self.lamport, self.delimiter, id))
    }

    /// key returns the key string for this Ticket.
    pub fn key(&self) -> Result<String, str::Utf8Error> {
        let id = self.actor_id.as_str()?;
        Ok(format!("{}:{}:{}", self.lamport, self.delimiter, id))
    }

    pub fn lamport(&self) -> u64 {
        self.lamport
    }

    pub fn delimiter(&self) -> u32 {
        self.delimiter
    }

    pub fn actor_id(&self) -> &ActorID {
        &self.actor_id
    }

    /// compare returns an integer comparing two Ticket.
    /// The result will be 0 if id==other, -1 if id < other, and +1 if id > other.
    /// If the receiver or argument is nil, it would panic at runtime.
    pub fn compare(&self, other: &Ticket) -> i8 {
        if self.lamport > other.lamport {
            return 1;
        } else if self.lamport < other.lamport {
            return -1;
        }

        return 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // fn annotated_string() {
    //     let ticket = Ticket::new(0, 0, ActorID{id:String::from("test")});
    //     assert_eq!(ticket.annotated_string(), "0:0:test");
    // }

    // #[test]
    // fn key() {
    //     let ticket = Ticket::new(0, 0, ActorID{id:String::from("test")});
    //     assert_eq!(ticket.annotated_string(), "0:0:test");
    // }
}