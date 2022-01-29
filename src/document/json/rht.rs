use crate::document::time::ticket;

struct RHTNode {
    key: String,
    val: String,
    updated_at: ticket::Ticket,
    removed_at: Option<ticket::Ticket>,
}

impl RHTNode {
    fn new(key: String, val: String, updated_at: ticket::Ticket) -> RHTNode {
        RHTNode {
            key,
            val,
            updated_at,
            removed_at: None,
        }
    }

    fn key(&self) -> &String {
        &self.key
    }

    fn value(&self) -> &String {
        &self.val
    }

    fn updated_at(&self) -> &ticket::Ticket {
        &self.updated_at
    }

    fn removed_at(&self) -> Option<&ticket::Ticket> {
        match &self.removed_at {
            Some(removed_at) => Some(&removed_at),
            _ => None
        }
    }

    fn remove(&mut self, removed_at: ticket::Ticket) {
        self.removed_at = Some(removed_at)
    }

    fn is_removed(&self) -> bool {
        match self.removed_at {
            None => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod rht_node_tests {
    use super::*;
    use crate::document::time::{ticket, actor_id};

    #[test]
    fn remove() {
        let id = actor_id::ActorID::from_hex("0000000000abcdef01234567").unwrap();

        let mut node = RHTNode::new(
            String::from("key"),
            String::from("value"),
            ticket::Ticket::new(0, 0, id.clone()),
        );
        assert!(!node.is_removed());

        let removed_at = ticket::Ticket::new(0, 1, id.clone());
        node.remove(removed_at.clone());
        assert_eq!(node.removed_at().unwrap(), &removed_at);
        assert!(node.is_removed());
    }
}
