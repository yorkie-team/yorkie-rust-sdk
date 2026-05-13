use crate::{TimeTicket, TIME_TICKET_SIZE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrdtElementMeta {
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
}

impl CrdtElementMeta {
    pub(crate) fn new(created_at: TimeTicket) -> Self {
        Self {
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn id(&self) -> &TimeTicket {
        self.created_at()
    }

    pub(crate) fn moved_at(&self) -> Option<&TimeTicket> {
        self.moved_at.as_ref()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.removed_at.as_ref()
    }

    pub(crate) fn positioned_at(&self) -> &TimeTicket {
        self.moved_at.as_ref().unwrap_or(&self.created_at)
    }

    pub(crate) fn set_created_at(&mut self, created_at: TimeTicket) {
        self.created_at = created_at;
    }

    pub(crate) fn set_moved_at(&mut self, moved_at: Option<TimeTicket>) -> bool {
        if self.moved_at.is_none()
            || moved_at
                .as_ref()
                .is_some_and(|candidate| candidate.after(self.moved_at.as_ref().unwrap()))
        {
            self.moved_at = moved_at;
            return true;
        }

        false
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.removed_at = removed_at;
    }

    pub(crate) fn remove(&mut self, removed_at: Option<TimeTicket>) -> bool {
        let Some(removed_at) = removed_at else {
            return false;
        };

        if removed_at.after(&self.created_at)
            && self
                .removed_at
                .as_ref()
                .map(|current| removed_at.after(current))
                .unwrap_or(true)
        {
            self.removed_at = Some(removed_at);
            return true;
        }

        false
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    pub(crate) fn meta_usage(&self) -> usize {
        let mut meta = TIME_TICKET_SIZE;

        if self.moved_at.is_some() {
            meta += TIME_TICKET_SIZE;
        }

        if self.removed_at.is_some() {
            meta += TIME_TICKET_SIZE;
        }

        meta
    }
}

#[cfg(test)]
mod tests {
    use super::CrdtElementMeta;
    use crate::{TimeTicket, TIME_TICKET_SIZE};

    #[test]
    fn tracks_created_id_and_position() {
        let created_at = TimeTicket::new(1, 0, "a");
        let meta = CrdtElementMeta::new(created_at.clone());

        assert_eq!(&created_at, meta.created_at());
        assert_eq!(&created_at, meta.id());
        assert_eq!(&created_at, meta.positioned_at());
        assert_eq!(None, meta.moved_at());
        assert_eq!(TIME_TICKET_SIZE, meta.meta_usage());
    }

    #[test]
    fn updates_position_with_later_move_time() {
        let created_at = TimeTicket::new(1, 0, "a");
        let moved_at = TimeTicket::new(2, 0, "a");
        let older_moved_at = TimeTicket::new(1, 1, "a");
        let newer_moved_at = TimeTicket::new(3, 0, "a");
        let mut meta = CrdtElementMeta::new(created_at);

        assert!(meta.set_moved_at(Some(moved_at.clone())));
        assert_eq!(Some(&moved_at), meta.moved_at());
        assert_eq!(&moved_at, meta.positioned_at());
        assert_eq!(TIME_TICKET_SIZE * 2, meta.meta_usage());

        assert!(!meta.set_moved_at(Some(older_moved_at)));
        assert_eq!(Some(&moved_at), meta.moved_at());

        assert!(meta.set_moved_at(Some(newer_moved_at.clone())));
        assert_eq!(Some(&newer_moved_at), meta.moved_at());
    }

    #[test]
    fn removes_with_later_remove_time() {
        let created_at = TimeTicket::new(2, 0, "a");
        let older_removed_at = TimeTicket::new(1, 0, "a");
        let removed_at = TimeTicket::new(3, 0, "a");
        let newer_removed_at = TimeTicket::new(4, 0, "a");
        let mut meta = CrdtElementMeta::new(created_at);

        assert!(!meta.remove(None));
        assert!(!meta.remove(Some(older_removed_at)));
        assert!(!meta.is_removed());

        assert!(meta.remove(Some(removed_at.clone())));
        assert_eq!(Some(&removed_at), meta.removed_at());
        assert!(meta.is_removed());
        assert_eq!(TIME_TICKET_SIZE * 2, meta.meta_usage());

        assert!(!meta.remove(Some(TimeTicket::new(3, 0, "a"))));
        assert_eq!(Some(&removed_at), meta.removed_at());

        assert!(meta.remove(Some(newer_removed_at.clone())));
        assert_eq!(Some(&newer_removed_at), meta.removed_at());
    }

    #[test]
    fn sets_created_and_removed_times_directly() {
        let created_at = TimeTicket::new(1, 0, "a");
        let new_created_at = TimeTicket::new(2, 0, "a");
        let removed_at = TimeTicket::new(3, 0, "a");
        let mut meta = CrdtElementMeta::new(created_at);

        meta.set_created_at(new_created_at.clone());
        assert_eq!(&new_created_at, meta.created_at());

        meta.set_removed_at(Some(removed_at.clone()));
        assert_eq!(Some(&removed_at), meta.removed_at());

        meta.set_removed_at(None);
        assert_eq!(None, meta.removed_at());
    }
}
