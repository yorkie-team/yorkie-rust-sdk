use crate::document::time::ticket::Ticket;

pub trait Element {
    fn to_string(&self) -> String;
    fn clone(&self) -> Box<dyn Element>;
    fn created_at(&self) -> Ticket;
    fn moved_at(&self) -> Option<Ticket>;
    fn set_moved_at(&self, ticket: Ticket);
    fn removed_at(&self) -> Option<Ticket>;
    fn remove(&self, ticket: Ticket) -> bool;
}

impl Clone for Box<dyn Element> {
    fn clone(&self) -> Box<dyn Element> {
        self.clone()
    }
}
