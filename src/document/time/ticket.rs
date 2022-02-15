use crate::document::time::actor_id::ActorID;
use std::cmp::Ordering;
use std::{u32, u64};

const MAX_LAMPORT: u64 = u64::MAX;
const MAX_DELIMITER: u32 = u32::MAX;

/// Ticket is a timestamp of the logical clock. Ticket is immutable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ticket {
    lamport: u64,
    delimiter: u32,
    actor_id: ActorID,
}

impl Ticket {
    pub fn new(lamport: u64, delimiter: u32, actor_id: ActorID) -> Ticket {
        Ticket {
            lamport,
            delimiter,
            actor_id,
        }
    }

    /// annotated_string returns a string containing the metadata of the ticket
    /// for debugging purpose.
    pub fn annotated_string(&self) -> String {
        let id = self.actor_id.to_string();
        format!("{}:{}:{}", self.lamport, self.delimiter, id)
    }

    /// key returns the key string for this Ticket.
    pub fn key(&self) -> String {
        self.annotated_string()
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

    /// cmp returns an cmp::Ordering comparing two Ticket.
    pub fn cmp(&self, other: &Ticket) -> Ordering {
        match self.lamport.cmp(&other.lamport) {
            Ordering::Equal => (),
            etc => return etc,
        }

        match self.actor_id.cmp(&other.actor_id) {
            Ordering::Equal => (),
            etc => return etc,
        }

        match self.delimiter.cmp(&other.delimiter) {
            Ordering::Equal => (),
            etc => return etc,
        }

        return Ordering::Equal;
    }

    // after returns whether the given ticket was created later.
    pub fn after(&self, other: &Ticket) -> bool {
        match self.cmp(other) {
            Ordering::Greater => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotated_string() {
        let hex_str = "0123456789abcdef01234567";
        let actor_id = ActorID::from_hex(hex_str).unwrap();
        let ticket = Ticket::new(0, 0, actor_id);

        let annotated_str = ticket.annotated_string();
        assert_eq!(annotated_str, format!("0:0:{}", hex_str));
    }

    #[test]
    fn cmp() {
        let hex_str = "0123456789abcdef01234567";
        let actor_id = ActorID::from_hex(hex_str).unwrap();

        // compare for lamport
        let before_ticket = Ticket::new(0, 0, actor_id.clone());
        let after_ticket = Ticket::new(1, 0, actor_id.clone());

        assert_eq!(Ordering::Less, before_ticket.cmp(&after_ticket));
        assert_eq!(Ordering::Greater, after_ticket.cmp(&before_ticket));
        assert_eq!(Ordering::Equal, after_ticket.cmp(&after_ticket));

        // compare for actor_id
        let hex_str = "0000000000abcdef01234567";
        let before_actor_id = ActorID::from_hex(hex_str).unwrap();
        let before_ticket = Ticket::new(0, 0, before_actor_id);
        let after_ticket = Ticket::new(0, 0, actor_id.clone());

        assert_eq!(Ordering::Less, before_ticket.cmp(&after_ticket));
        assert_eq!(Ordering::Greater, after_ticket.cmp(&before_ticket));
        assert_eq!(Ordering::Equal, after_ticket.cmp(&after_ticket));

        // compare for delimiter
        let before_ticket = Ticket::new(0, 0, actor_id.clone());
        let after_ticket = Ticket::new(0, 1, actor_id.clone());

        assert_eq!(Ordering::Less, before_ticket.cmp(&after_ticket));
        assert_eq!(Ordering::Greater, after_ticket.cmp(&before_ticket));
        assert_eq!(Ordering::Equal, after_ticket.cmp(&after_ticket));
    }

    #[test]
    fn after() {
        let hex_str = "0123456789abcdef01234567";
        let actor_id = ActorID::from_hex(hex_str).unwrap();

        // compare for lamport
        let before_ticket = Ticket::new(0, 0, actor_id.clone());
        let after_ticket = Ticket::new(1, 0, actor_id.clone());

        assert!(!before_ticket.after(&after_ticket));
        assert!(after_ticket.after(&before_ticket));
    }
}
