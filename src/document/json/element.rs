use crate::document::time::ticket::Ticket;

pub trait Element {
    fn to_string(&self) -> String;
    fn deepcopy(&self) -> Box<dyn Element>;
    fn created_at(&self) -> Ticket;
    fn moved_at(&self) -> Option<Ticket>;
    fn set_moved_at(&mut self, ticket: Ticket);
    fn removed_at(&self) -> Option<Ticket>;
    fn remove(&mut self, ticket: Ticket) -> bool;
}

impl Clone for Box<dyn Element> {
    fn clone(&self) -> Box<dyn Element> {
        self.deepcopy()
    }
}