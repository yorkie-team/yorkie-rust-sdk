pub const INITIAL_CLIENT_SEQ: u32 = 0;
pub const INITIAL_SERVER_SEQ: i64 = 0;
pub const MAX_CLIENT_SEQ: u32 = u32::MAX;
pub const MAX_SERVER_SEQ: i64 = i64::MAX;

pub const INITIAL_CHECKPOINT: Checkpoint = Checkpoint::new(INITIAL_SERVER_SEQ, INITIAL_CLIENT_SEQ);
pub const MAX_CHECKPOINT: Checkpoint = Checkpoint::new(MAX_SERVER_SEQ, MAX_CLIENT_SEQ);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    server_seq: i64,
    client_seq: u32,
}

impl Checkpoint {
    pub const fn new(server_seq: i64, client_seq: u32) -> Self {
        Self {
            server_seq,
            client_seq,
        }
    }

    pub fn initial() -> Self {
        INITIAL_CHECKPOINT
    }

    pub fn max() -> Self {
        MAX_CHECKPOINT
    }

    pub fn next_server_seq(self, server_seq: i64) -> Self {
        if self.server_seq == server_seq {
            return self;
        }

        Self::new(server_seq, self.client_seq)
    }

    pub fn next_client_seq(self) -> Self {
        self.increase_client_seq(1)
    }

    pub fn increase_client_seq(self, inc: u32) -> Self {
        if inc == 0 {
            return self;
        }

        Self::new(self.server_seq, self.client_seq + inc)
    }

    pub fn sync_client_seq(self, client_seq: u32) -> Self {
        if self.client_seq < client_seq {
            return Self::new(self.server_seq, client_seq);
        }

        self
    }

    pub fn forward(self, other: Self) -> Self {
        if self == other {
            return self;
        }

        Self::new(
            self.server_seq.max(other.server_seq),
            self.client_seq.max(other.client_seq),
        )
    }

    pub fn server_seq_as_string(&self) -> String {
        self.server_seq.to_string()
    }

    pub fn server_seq(&self) -> i64 {
        self.server_seq
    }

    pub fn client_seq(&self) -> u32 {
        self.client_seq
    }

    pub fn to_test_string(&self) -> String {
        format!(
            "serverSeq={}, clientSeq={}",
            self.server_seq, self.client_seq
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Checkpoint, INITIAL_CHECKPOINT, MAX_CHECKPOINT};

    #[test]
    fn creates_initial_and_max_checkpoints() {
        assert_eq!(Checkpoint::new(0, 0), Checkpoint::initial());
        assert_eq!(INITIAL_CHECKPOINT, Checkpoint::initial());
        assert_eq!(MAX_CHECKPOINT, Checkpoint::max());
    }

    #[test]
    fn advances_checkpoint_sequences() {
        let checkpoint = Checkpoint::initial();

        assert_eq!(Checkpoint::new(5, 0), checkpoint.next_server_seq(5));
        assert_eq!(Checkpoint::new(0, 1), checkpoint.next_client_seq());
        assert_eq!(Checkpoint::new(0, 0), checkpoint.increase_client_seq(0));
        assert_eq!(Checkpoint::new(0, 5), checkpoint.increase_client_seq(5));
    }

    #[test]
    fn syncs_client_sequence_only_when_greater() {
        let checkpoint = Checkpoint::new(10, 20);

        assert_eq!(Checkpoint::new(10, 20), checkpoint.sync_client_seq(5));
        assert_eq!(Checkpoint::new(10, 30), checkpoint.sync_client_seq(30));
    }

    #[test]
    fn forwards_to_maximum_sequences() {
        let checkpoint = Checkpoint::new(10, 20);

        assert_eq!(checkpoint, checkpoint.forward(Checkpoint::new(1, 2)));
        assert_eq!(
            Checkpoint::new(20, 30),
            checkpoint.forward(Checkpoint::new(20, 30))
        );
        assert_eq!(
            Checkpoint::new(10, 30),
            checkpoint.forward(Checkpoint::new(5, 30))
        );
        assert_eq!(
            Checkpoint::new(20, 20),
            checkpoint.forward(Checkpoint::new(20, 5))
        );
    }

    #[test]
    fn formats_checkpoint_for_tests() {
        let checkpoint = Checkpoint::new(10, 20);

        assert_eq!("10", checkpoint.server_seq_as_string());
        assert_eq!("serverSeq=10, clientSeq=20", checkpoint.to_test_string());
    }
}
