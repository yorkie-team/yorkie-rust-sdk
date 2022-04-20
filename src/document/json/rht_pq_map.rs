use crate::document::json::element::Element;
use crate::document::time::ticket::Ticket;

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::rc::Rc;

use thiserror::Error;

type BoxedElement = Box<dyn Element>;

#[derive(Debug, Error)]
enum RHTPQMapError {
    #[error("fail to find : {0}")]
    ElementNotFound(String),
}

struct RHTPQMapNode {
    key: String,
    element: BoxedElement,
}

impl Ord for RHTPQMapNode {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.element.created_at().after(&other.element.created_at()) {
            true => Ordering::Greater,
            _ => Ordering::Less,
        }
    }
}

impl PartialOrd for RHTPQMapNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for RHTPQMapNode {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for RHTPQMapNode {}

impl Clone for RHTPQMapNode {
    fn clone(&self) -> RHTPQMapNode {
        RHTPQMapNode {
            key: self.key.clone(),
            element: self.element.clone(),
        }
    }
}

impl RHTPQMapNode {
    pub fn new(key: String, element: BoxedElement) -> RHTPQMapNode {
        RHTPQMapNode { key, element }
    }

    pub fn remove(&self, ticket: Ticket) -> bool {
        return self.element.remove(ticket);
    }

    pub fn is_removed(&self) -> bool {
        match self.element.removed_at() {
            Some(_) => true,
            _ => false,
        }
    }

    pub fn key(&self) -> String {
        self.key.clone()
    }
}

pub struct RHTPriorityQueueMap {
    node_queue_map_by_key: HashMap<String, BinaryHeap<Rc<RefCell<RHTPQMapNode>>>>,
    node_map_by_created_at: HashMap<Ticket, Rc<RefCell<RHTPQMapNode>>>,
}

impl RHTPriorityQueueMap {
    pub fn new() -> RHTPriorityQueueMap {
        RHTPriorityQueueMap {
            node_queue_map_by_key: HashMap::new(),
            node_map_by_created_at: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<BoxedElement> {
        let node = match self.node_queue_map_by_key.get(key) {
            Some(queue) => match queue.peek() {
                Some(node) => Some(node),
                _ => None,
            },
            _ => None,
        };

        if node.is_none() {
            return None;
        }

        let node = node.unwrap().borrow();
        match node.is_removed() {
            false => Some(node.element.clone()),
            true => None,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        match self.node_queue_map_by_key.get(key) {
            Some(queue) => match queue.peek() {
                Some(node) => !node.borrow().is_removed(),
                _ => false,
            },
            _ => false,
        }
    }

    pub fn set(&mut self, key: String, value: BoxedElement) -> Option<BoxedElement> {
        let node = match self.node_queue_map_by_key.get(&key) {
            Some(queue) => match queue.peek() {
                Some(node) => Some(node),
                _ => None,
            },
            _ => None,
        };

        let removed = match node {
            Some(node) => {
                let mut node = node.borrow_mut();
                if node.is_removed() {
                    return None;
                }

                if node.remove(value.created_at()) {
                    return Some(node.element.clone());
                } else {
                    return None;
                }
            }
            _ => None,
        };

        self.set_internal(key, value);

        removed
    }

    fn set_internal(&mut self, key: String, value: BoxedElement) {
        let node = RHTPQMapNode::new(key.clone(), value.clone());
        let node = Rc::new(RefCell::new(node));
        self.node_map_by_created_at
            .insert(value.created_at(), node.clone());

        let queue = self
            .node_queue_map_by_key
            .entry(key)
            .or_insert(BinaryHeap::new());
        queue.push(node);
    }

    pub fn delete(&self, key: String, deleted_at: Ticket) -> Option<BoxedElement> {
        match self.node_queue_map_by_key.get(&key) {
            Some(queue) => match queue.peek() {
                Some(node) => {
                    let node = node.borrow();
                    match node.remove(deleted_at) {
                        true => Some(node.element.clone()),
                        false => None,
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    pub fn delete_by_created_at(
        &self,
        created_at: Ticket,
        deleted_at: Ticket,
    ) -> Option<BoxedElement> {
        if let Some(node) = self.node_map_by_created_at.get(&created_at) {
            let node = node.borrow();
            match node.remove(deleted_at) {
                true => Some(node.element.clone()),
                false => None,
            }
        } else {
            None
        }
    }

    pub fn elements(&self) -> HashMap<String, BoxedElement> {
        let mut elements = HashMap::new();
        for (_, queue) in self.node_queue_map_by_key.iter() {
            for node in queue.iter() {
                let node = node.borrow();
                if node.is_removed() {
                    continue;
                }
                elements.insert(node.key(), node.element.clone());
            }
        }
        elements
    }

    pub fn nodes(&self) -> Vec<Rc<RefCell<RHTPQMapNode>>> {
        let mut nodes = vec![];
        for (_, queue) in self.node_queue_map_by_key.iter() {
            for node in queue.iter() {
                nodes.push(node.clone());
            }
        }
        nodes
    }

    fn purge(&mut self, element: BoxedElement) -> Result<(), Box<RHTPQMapError>> {
        match &self.node_map_by_created_at.get(&element.created_at()) {
            None => Err(Box::new(RHTPQMapError::ElementNotFound(
                element.created_at().key().to_string(),
            ))),
            Some(node) => {
                let node = node.borrow();
                match self.node_queue_map_by_key.get_mut(&node.key()) {
                    None => Err(Box::new(RHTPQMapError::ElementNotFound(
                        element.created_at().key().to_string(),
                    ))),
                    Some(queue) => {
                        let mut subqueue = BinaryHeap::new();
                        while !queue.is_empty() {
                            let item = queue.pop().unwrap();
                            if item.borrow().key() == node.key() {
                                continue;
                            }
                            subqueue.push(item);
                        }
                        while !subqueue.is_empty() {
                            queue.push(subqueue.pop().unwrap());
                        }
                        node.remove(node.element.created_at());
                        Ok(())
                    }
                }
            }
        }
    }

    pub fn to_string(&self) -> String {
        let members = self.elements();

        let mut keys = vec![];
        for (key, _) in members.iter() {
            keys.push(key.clone());
        }
        keys.sort();

        let mut ret = String::new();
        ret.push_str("{");
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                ret.push_str(",");
            }
            let value = members.get(key).unwrap().to_string();
            ret.push_str(&format!("\"{}\":{}", key, value));
        }
        ret.push_str("}");

        ret
    }
}
