use crate::time::ActorId;
use crate::{TimeTicket, VersionVector, INITIAL_ACTOR_ID, INITIAL_LAMPORT};

pub(crate) const INITIAL_CLIENT_SEQ: u32 = 0;
pub(crate) const INITIAL_SERVER_SEQ: i64 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeId {
    client_seq: u32,
    server_seq: i64,
    lamport: i64,
    actor_id: ActorId,
    version_vector: VersionVector,
}

impl ChangeId {
    pub(crate) fn new(
        client_seq: u32,
        server_seq: i64,
        lamport: i64,
        actor_id: impl Into<ActorId>,
        version_vector: VersionVector,
    ) -> Self {
        Self {
            client_seq,
            server_seq,
            lamport,
            actor_id: actor_id.into(),
            version_vector,
        }
    }

    pub(crate) fn initial() -> Self {
        Self::new(
            INITIAL_CLIENT_SEQ,
            INITIAL_SERVER_SEQ,
            INITIAL_LAMPORT,
            INITIAL_ACTOR_ID,
            VersionVector::new(),
        )
    }

    pub(crate) fn has_clocks(&self) -> bool {
        !self.version_vector.is_empty() && self.lamport != INITIAL_LAMPORT
    }

    pub(crate) fn next(&self, exclude_clocks: bool) -> Self {
        if exclude_clocks {
            return Self::new(
                self.client_seq + 1,
                INITIAL_SERVER_SEQ,
                self.lamport,
                self.actor_id.clone(),
                VersionVector::new(),
            );
        }

        let lamport = self.lamport + 1;
        let mut version_vector = self.version_vector.clone();
        version_vector.set(self.actor_id.clone(), lamport);

        Self::new(
            self.client_seq + 1,
            INITIAL_SERVER_SEQ,
            lamport,
            self.actor_id.clone(),
            version_vector,
        )
    }

    pub(crate) fn sync_clocks(&self, other: &Self) -> Self {
        if !other.has_clocks() {
            return self.clone();
        }

        let lamport = self.lamport.max(other.lamport) + 1;
        let mut version_vector = self.version_vector.max(&other.version_vector);
        version_vector.set(self.actor_id.clone(), lamport);

        Self::new(
            self.client_seq,
            INITIAL_SERVER_SEQ,
            lamport,
            self.actor_id.clone(),
            version_vector,
        )
    }

    pub(crate) fn set_clocks(&self, lamport: i64, version_vector: VersionVector) -> Self {
        let lamport = self.lamport.max(lamport) + 1;
        let mut version_vector = self.version_vector.max(&version_vector);
        version_vector.set(self.actor_id.clone(), lamport);

        Self::new(
            self.client_seq,
            self.server_seq,
            lamport,
            self.actor_id.clone(),
            version_vector,
        )
    }

    pub(crate) fn create_time_ticket(&self, delimiter: u32) -> TimeTicket {
        TimeTicket::new(self.lamport, delimiter, self.actor_id.clone())
    }

    pub(crate) fn set_actor(&self, actor_id: impl Into<ActorId>) -> Self {
        Self::new(
            self.client_seq,
            INITIAL_SERVER_SEQ,
            self.lamport,
            actor_id,
            self.version_vector.clone(),
        )
    }

    pub(crate) fn set_lamport(&self, lamport: i64) -> Self {
        Self::new(
            self.client_seq,
            self.server_seq,
            lamport,
            self.actor_id.clone(),
            self.version_vector.clone(),
        )
    }

    pub(crate) fn set_version_vector(&self, version_vector: VersionVector) -> Self {
        Self::new(
            self.client_seq,
            self.server_seq,
            self.lamport,
            self.actor_id.clone(),
            version_vector,
        )
    }

    pub(crate) fn client_seq(&self) -> u32 {
        self.client_seq
    }

    pub(crate) fn server_seq(&self) -> i64 {
        self.server_seq
    }

    pub(crate) fn lamport(&self) -> i64 {
        self.lamport
    }

    pub(crate) fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    pub(crate) fn version_vector(&self) -> &VersionVector {
        &self.version_vector
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}:{}:{}",
            self.lamport,
            last_chars(self.actor_id.as_str(), 2),
            self.client_seq
        )
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

#[cfg(test)]
mod tests {
    use super::ChangeId;
    use crate::VersionVector;

    #[test]
    fn creates_next_id_with_clocks() {
        let id = ChangeId::initial();
        let next_id = id.next(false);

        assert_eq!(1, next_id.client_seq());
        assert_eq!(1, next_id.lamport());
        assert_eq!(
            Some(1),
            next_id.version_vector().get(next_id.actor_id().as_str())
        );
        assert_eq!("1:00:1", next_id.to_test_string());
        assert!(next_id.has_clocks());
    }

    #[test]
    fn creates_time_tickets_from_change_clock() {
        let id = ChangeId::initial().next(false);
        let ticket = id.create_time_ticket(7);

        assert_eq!(1, ticket.lamport());
        assert_eq!(7, ticket.delimiter());
        assert_eq!(id.actor_id(), ticket.actor_id());
    }

    #[test]
    fn syncs_clocks_with_other_change() {
        let id = ChangeId::initial().next(false);
        let mut vector = VersionVector::new();
        vector.set("actor-b", 5);
        let other = ChangeId::new(1, 0, 5, "actor-b", vector);

        let synced = id.sync_clocks(&other);

        assert_eq!(6, synced.lamport());
        assert_eq!(
            Some(6),
            synced.version_vector().get(synced.actor_id().as_str())
        );
        assert_eq!(Some(5), synced.version_vector().get("actor-b"));
    }
}
