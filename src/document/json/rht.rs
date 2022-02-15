use crate::document::time::ticket::Ticket;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// RHTNode is a node of RHT(Replicated Hashtable).
struct RHTNode {
    key: String,
    val: String,
    updated_at: Ticket,
    removed_at: Option<Ticket>,
}

impl RHTNode {
    pub fn new(key: String, val: String, updated_at: Ticket) -> RHTNode {
        RHTNode {
            key,
            val,
            updated_at,
            removed_at: None,
        }
    }

    /// key returns the key of this node.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// value returns the value of this node.
    pub fn value(&self) -> &str {
        &self.val
    }

    /// updated_at returns the last update time.
    pub fn updated_at(&self) -> &Ticket {
        &self.updated_at
    }

    /// removed_at returns the deletion time of this node.
    pub fn removed_at(&self) -> Option<&Ticket> {
        if let Some(removed_at) = &self.removed_at {
            return Some(&removed_at);
        }

        return None;
    }

    /// remove removes this node. It only marks the deleted time (tombstone).
    pub fn remove(&mut self, removed_at: Ticket) {
        if let Some(v) = &self.removed_at {
            if removed_at.after(v) {
                self.removed_at = Some(removed_at)
            }
            return;
        }
        self.removed_at = Some(removed_at);
    }

    pub fn is_removed(&self) -> bool {
        if let None = self.removed_at {
            return false;
        }
        true
    }
}

/// RHT is a hashtable with logical clock(Replicated hashtable).
/// For more details about RHT: http://csl.skku.edu/papers/jpdc11.pdf
pub struct RHT {
    node_map_by_key: HashMap<String, Rc<RefCell<RHTNode>>>,
    node_map_by_created_at: HashMap<Ticket, Rc<RefCell<RHTNode>>>,
}

impl RHT {
    pub fn new() -> RHT {
        RHT {
            node_map_by_key: HashMap::new(),
            node_map_by_created_at: HashMap::new(),
        }
    }

    /// insert sets the value of the given key.
    pub fn insert(&mut self, key: String, val: String, executed_at: Ticket) {
        if let Some(node) = self.node_map_by_key.get(&key) {
            if executed_at.after(&node.borrow().updated_at) {
                self.insert_exec(key, val, executed_at);
            }
            return;
        }

        self.insert_exec(key, val, executed_at);
    }

    fn insert_exec(&mut self, key: String, val: String, executed_at: Ticket) {
        let node = RHTNode::new(key.clone(), val, executed_at.clone());

        let node = Rc::new(RefCell::new(node));
        self.node_map_by_key.insert(key, Rc::clone(&node));
        self.node_map_by_created_at.insert(executed_at, node);
    }

    /// get returns the value of the given key.
    pub fn get(&self, key: &str) -> String {
        if let Some(node) = &self.node_map_by_key.get(key) {
            println!("self= {:p}, node1 = {:p}", self, node.as_ptr());
            let node = node.borrow();
            if node.is_removed() {
                return String::from("");
            }
            return node.value().to_string();
        }

        String::from("")
    }

    /// has returns whether the element exists of the given key or not.
    pub fn has(&self, key: &str) -> bool {
        if let Some(node) = self.node_map_by_key.get(key) {
            return !node.borrow().is_removed();
        }

        false
    }

    /// remove removes the Element of the given key.
    pub fn remove(&mut self, key: &str, executed_at: Ticket) -> String {
        if let Some(node) = self.node_map_by_key.get(key) {
            let mut node = node.borrow_mut();
            if let Some(removed_at) = &node.removed_at {
                if executed_at.after(removed_at) {
                    node.remove(executed_at);
                    return node.value().to_string();
                }
            } else {
                node.remove(executed_at);
                return node.value().to_string();
            }
        }

        String::from("")
    }

    /// elements returns a map of elements because the map easy to use for loop.
    pub fn elements(&self) -> HashMap<String, String> {
        self.node_map_by_key
            .iter()
            .map(|(key, node)| (key.clone(), node.borrow().value().to_string()))
            .collect()
    }

    pub fn clone(&self) -> RHT {
        let mut rht = RHT::new();

        self.node_map_by_key.iter().for_each(|(_, node)| {
            let node = node.borrow();
            rht.insert(
                node.key().to_string(),
                node.value().to_string(),
                node.updated_at().clone(),
            )
        });

        rht
    }

    pub fn to_string(&self) -> String {
        let members = self.elements();

        let mut result = String::from("{");
        let mut keys = members.keys().collect::<Vec<_>>();
        keys.sort();
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                result.push(',');
            }
            let value = members.get(key as &str).unwrap();
            result.push_str(&format!("{}:{}", key, value));
        }
        result.push('}');

        result
    }
}

#[cfg(test)]
mod rht_node_tests {
    use super::*;
    use crate::document::time::actor_id::ActorID;
    use crate::document::time::ticket::Ticket;

    #[test]
    fn remove() {
        let id = ActorID::from_hex("0000000000abcdef01234567").unwrap();

        let mut node = RHTNode::new(
            String::from("key"),
            String::from("value"),
            Ticket::new(0, 0, id.clone()),
        );
        assert!(!node.is_removed());

        let removed_at = Ticket::new(0, 1, id.clone());
        node.remove(removed_at.clone());
        assert_eq!(node.removed_at().unwrap(), &removed_at);
        assert!(node.is_removed());

        let before_removed_at = Ticket::new(0, 0, id.clone());
        node.remove(before_removed_at);
        assert_eq!(node.removed_at().unwrap(), &removed_at);
        assert!(node.is_removed());
    }
}

#[cfg(test)]
mod rht_tests {
    use super::*;
    use crate::document::time::actor_id::ActorID;
    use crate::document::time::ticket::Ticket;

    #[test]
    fn insert() {
        let mut rht = RHT::new();
        let key = "key";
        let val = "value";
        let id = ActorID::from_hex("0000000000abcdef01234567").unwrap();
        let executed_at = Ticket::new(0, 0, id.clone());

        rht.insert(key.to_string(), val.to_string(), executed_at);
        assert_eq!(rht.get(key), val);
        assert!(rht.has(key));

        // when after ticket
        let val = "value2";
        let executed_at = Ticket::new(0, 1, id.clone());
        rht.insert(key.to_string(), val.to_string(), executed_at);
        assert_eq!(rht.get(key), val);
        assert!(rht.has(key));

        // when before ticket
        let val = "value3";
        let executed_at = Ticket::new(0, 0, id.clone());
        rht.insert(key.to_string(), val.to_string(), executed_at);
        assert_ne!(rht.get(key), val);
        assert!(rht.has(key));
    }

    #[test]
    fn get_when_empty_map() {
        let rht = RHT::new();

        assert_eq!(rht.get("key"), "");
        assert!(!rht.has("key"));
    }

    #[test]
    fn remove() {
        let mut rht = RHT::new();
        let key = "key";
        let val = "value";
        let id = ActorID::from_hex("0000000000abcdef01234567").unwrap();
        let executed_at = Ticket::new(0, 0, id.clone());

        // when removed_at is None
        rht.insert(key.to_string(), val.to_string(), executed_at.clone());
        assert_eq!(rht.remove(key, executed_at.clone()), val);
        assert!(!rht.has(key));

        // invalid key
        assert_eq!(rht.remove("", executed_at.clone()), "");

        // when after executed_at
        // TODO: Is this the intended behavior?
        let executed_at = Ticket::new(0, 1, id.clone());
        assert_eq!(rht.remove(key, executed_at.clone()), val);
        assert!(!rht.has(key));
    }

    #[test]
    fn elements() {
        let mut rht = RHT::new();
        let keys = vec!["key", "key2"];
        let values = vec!["value", "value2"];
        let id = ActorID::from_hex("0000000000abcdef01234567").unwrap();
        let executed_at = Ticket::new(0, 0, id.clone());

        for (i, key) in keys.iter().enumerate() {
            rht.insert(key.to_string(), values[i].to_string(), executed_at.clone());
        }

        let elements = rht.elements();
        for i in 0..keys.len() {
            assert_eq!(elements.get(keys[i]).unwrap(), values[i]);
        }
    }

    #[test]
    fn to_string() {
        let mut rht = RHT::new();
        assert_eq!(rht.to_string(), "{}");

        let id = ActorID::from_hex("0000000000abcdef01234567").unwrap();
        let executed_at = Ticket::new(0, 0, id.clone());
        rht.insert("b".to_string(), "2".to_string(), executed_at.clone());
        rht.insert("c".to_string(), "3".to_string(), executed_at.clone());
        rht.insert("a".to_string(), "1".to_string(), executed_at);

        assert_eq!(rht.to_string(), "{a:1,b:2,c:3}");
    }
}
