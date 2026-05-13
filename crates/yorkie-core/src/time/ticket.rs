use super::{ActorId, INITIAL_ACTOR_ID, MAX_ACTOR_ID};
use crate::{Result, YorkieError};
use std::cmp::Ordering;

pub const TIME_TICKET_SIZE: usize = 8 + 4 + 12;
pub const INITIAL_LAMPORT: i64 = 0;
pub const INITIAL_DELIMITER: u32 = 0;
pub const MAX_DELIMITER: u32 = u32::MAX;
pub const MAX_LAMPORT: i64 = i64::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeTicketStruct {
    pub lamport: String,
    pub delimiter: u32,
    pub actor_id: ActorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeTicket {
    lamport: i64,
    delimiter: u32,
    actor_id: ActorId,
}

impl TimeTicket {
    pub fn new(lamport: i64, delimiter: u32, actor_id: impl Into<ActorId>) -> Self {
        Self {
            lamport,
            delimiter,
            actor_id: actor_id.into(),
        }
    }

    pub fn initial() -> Self {
        Self::new(INITIAL_LAMPORT, INITIAL_DELIMITER, INITIAL_ACTOR_ID)
    }

    pub fn max() -> Self {
        Self::new(MAX_LAMPORT, MAX_DELIMITER, MAX_ACTOR_ID)
    }

    pub fn from_struct(value: TimeTicketStruct) -> Result<Self> {
        let lamport = value
            .lamport
            .parse::<i64>()
            .map_err(|_| YorkieError::InvalidTimeTicketLamport(value.lamport.clone()))?;

        Ok(Self::new(lamport, value.delimiter, value.actor_id))
    }

    pub fn to_id_string(&self) -> String {
        format!("{}:{}:{}", self.lamport, self.actor_id, self.delimiter)
    }

    pub fn to_struct(&self) -> TimeTicketStruct {
        TimeTicketStruct {
            lamport: self.lamport_as_string(),
            delimiter: self.delimiter,
            actor_id: self.actor_id.clone(),
        }
    }

    pub fn to_test_string(&self) -> String {
        format!(
            "{}:{}:{}",
            self.lamport,
            last_chars(self.actor_id.as_str(), 2),
            self.delimiter
        )
    }

    pub fn set_actor(&self, actor_id: impl Into<ActorId>) -> Self {
        Self::new(self.lamport, self.delimiter, actor_id)
    }

    pub fn lamport_as_string(&self) -> String {
        self.lamport.to_string()
    }

    pub fn lamport(&self) -> i64 {
        self.lamport
    }

    pub fn delimiter(&self) -> u32 {
        self.delimiter
    }

    pub fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    pub fn after(&self, other: &Self) -> bool {
        self > other
    }
}

impl Ord for TimeTicket {
    fn cmp(&self, other: &Self) -> Ordering {
        self.lamport
            .cmp(&other.lamport)
            .then_with(|| self.actor_id.cmp(&other.actor_id))
            .then_with(|| self.delimiter.cmp(&other.delimiter))
    }
}

impl PartialOrd for TimeTicket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn last_chars(value: &str, count: usize) -> &str {
    value
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map(|(idx, _)| &value[idx..])
        .unwrap_or(value)
}
