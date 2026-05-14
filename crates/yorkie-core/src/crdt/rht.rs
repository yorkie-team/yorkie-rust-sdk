use super::element::DataSize;
use crate::json::escape_json_string;
use crate::{TimeTicket, TIME_TICKET_SIZE};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RhtNode {
    key: String,
    value: String,
    updated_at: TimeTicket,
    is_removed: bool,
}

impl RhtNode {
    pub(crate) fn new(
        key: impl Into<String>,
        value: impl Into<String>,
        updated_at: TimeTicket,
        is_removed: bool,
    ) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            updated_at,
            is_removed,
        }
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn updated_at(&self) -> &TimeTicket {
        &self.updated_at
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.is_removed
    }

    pub(crate) fn id_string(&self) -> String {
        format!("{}:{}", self.updated_at.to_id_string(), self.key)
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.is_removed.then_some(&self.updated_at)
    }

    pub(crate) fn data_size(&self) -> DataSize {
        DataSize {
            data: string_size(&self.key) + string_size(&self.value),
            meta: TIME_TICKET_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rht {
    node_by_key: BTreeMap<String, RhtNode>,
    number_of_removed_elements: usize,
}

impl Rht {
    pub(crate) fn new() -> Self {
        Self {
            node_by_key: BTreeMap::new(),
            number_of_removed_elements: 0,
        }
    }

    pub(crate) fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        executed_at: TimeTicket,
    ) -> (Option<RhtNode>, Option<RhtNode>) {
        let key = key.into();
        let value = value.into();
        let prev = self.node_by_key.get(&key).cloned();

        if prev
            .as_ref()
            .is_some_and(|node| node.is_removed() && executed_at.after(node.updated_at()))
        {
            self.number_of_removed_elements -= 1;
        }

        if prev
            .as_ref()
            .map(|node| executed_at.after(node.updated_at()))
            .unwrap_or(true)
        {
            let node = RhtNode::new(key.clone(), value, executed_at, false);
            self.node_by_key.insert(key, node.clone());

            if prev.as_ref().is_some_and(RhtNode::is_removed) {
                return (prev, Some(node));
            }

            return (None, Some(node));
        }

        if prev.as_ref().is_some_and(RhtNode::is_removed) {
            return (prev, None);
        }

        (None, None)
    }

    pub(crate) fn set_internal(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        updated_at: TimeTicket,
        removed: bool,
    ) {
        let key = key.into();
        let node = RhtNode::new(key.clone(), value, updated_at, removed);
        self.node_by_key.insert(key, node);

        if removed {
            self.number_of_removed_elements += 1;
        }
    }

    pub(crate) fn remove(&mut self, key: &str, executed_at: TimeTicket) -> Vec<RhtNode> {
        let prev = self.node_by_key.get(key).cloned();

        if prev
            .as_ref()
            .map(|node| !executed_at.after(node.updated_at()))
            .unwrap_or(false)
        {
            return Vec::new();
        }

        let Some(prev) = prev else {
            self.number_of_removed_elements += 1;
            let node = RhtNode::new(key, "", executed_at, true);
            self.node_by_key.insert(key.to_owned(), node.clone());
            return vec![node];
        };

        let mut gc_nodes = Vec::new();
        if prev.is_removed() {
            gc_nodes.push(prev.clone());
        } else {
            self.number_of_removed_elements += 1;
        }

        let node = RhtNode::new(key, prev.value(), executed_at, true);
        self.node_by_key.insert(key.to_owned(), node.clone());
        gc_nodes.push(node);

        gc_nodes
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.node_by_key
            .get(key)
            .is_some_and(|node| !node.is_removed())
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.node_by_key.get(key).map(RhtNode::value)
    }

    pub(crate) fn get_node(&self, key: &str) -> Option<&RhtNode> {
        self.node_by_key.get(key)
    }

    pub(crate) fn len(&self) -> usize {
        self.node_by_key.len() - self.number_of_removed_elements
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn to_json(&self) -> String {
        if self.is_empty() {
            return "{}".to_owned();
        }

        let items = self
            .node_by_key
            .iter()
            .filter(|(_, node)| !node.is_removed())
            .map(|(key, node)| {
                format!(
                    "\"{}\":\"{}\"",
                    escape_json_string(key),
                    escape_json_string(node.value())
                )
            })
            .collect::<Vec<_>>();

        format!("{{{}}}", items.join(","))
    }

    pub(crate) fn to_object(&self) -> BTreeMap<String, String> {
        self.node_by_key
            .iter()
            .filter(|(_, node)| !node.is_removed())
            .map(|(key, node)| (key.clone(), node.value().to_owned()))
            .collect()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &RhtNode> {
        self.node_by_key.values()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        let mut rht = Self::new();
        for node in self.iter() {
            rht.set_internal(
                node.key().to_owned(),
                node.value().to_owned(),
                node.updated_at().clone(),
                node.is_removed(),
            );
        }
        rht
    }

    pub(crate) fn purge(&mut self, child: &RhtNode) {
        let Some(node) = self.node_by_key.get(child.key()) else {
            return;
        };

        if node.id_string() != child.id_string() {
            return;
        }

        self.node_by_key.remove(child.key());
        self.number_of_removed_elements -= 1;
    }
}

impl Default for Rht {
    fn default() -> Self {
        Self::new()
    }
}

fn string_size(value: &str) -> usize {
    value.encode_utf16().count() * 2
}

#[cfg(test)]
mod tests {
    use super::{Rht, RhtNode};
    use crate::{TimeTicket, TIME_TICKET_SIZE};
    use std::collections::BTreeMap;

    #[test]
    fn sets_a_value_and_exposes_the_new_node() {
        let mut rht = Rht::new();
        let executed_at = ticket(1, "a");

        let (removed, node) = rht.set("bold", "true", executed_at.clone());

        assert!(removed.is_none());
        let node = node.unwrap();
        assert_eq!("bold", node.key());
        assert_eq!("true", node.value());
        assert_eq!(&executed_at, node.updated_at());
        assert_eq!(Some("true"), rht.get("bold"));
        assert!(rht.has("bold"));
        assert_eq!(1, rht.len());
        assert_eq!(r#"{"bold":"true"}"#, rht.to_json());
    }

    #[test]
    fn keeps_the_latest_update_for_the_same_key() {
        let mut rht = Rht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        rht.set("color", "red", t1.clone());
        let (removed, node) = rht.set("color", "blue", t2.clone());

        assert!(removed.is_none());
        assert_eq!("blue", node.unwrap().value());
        assert_eq!(Some("blue"), rht.get("color"));
        assert_eq!(
            Some(&t2),
            rht.get_node("color").unwrap().updated_at().into()
        );
    }

    #[test]
    fn ignores_older_live_updates() {
        let mut rht = Rht::new();
        let old = ticket(1, "b");
        let new = ticket(2, "a");

        rht.set("color", "blue", new.clone());
        let (removed, node) = rht.set("color", "red", old);

        assert!(removed.is_none());
        assert!(node.is_none());
        assert_eq!(Some("blue"), rht.get("color"));
        assert_eq!(
            Some(&new),
            rht.get_node("color").unwrap().updated_at().into()
        );
    }

    #[test]
    fn removes_existing_values_and_returns_gc_nodes() {
        let mut rht = Rht::new();
        let set_at = ticket(1, "a");
        let remove_at = ticket(2, "a");

        rht.set("italic", "true", set_at);
        let gc_nodes = rht.remove("italic", remove_at.clone());

        assert_eq!(1, gc_nodes.len());
        assert_eq!("italic", gc_nodes[0].key());
        assert_eq!("true", gc_nodes[0].value());
        assert_eq!(Some(&remove_at), gc_nodes[0].removed_at());
        assert!(!rht.has("italic"));
        assert_eq!(Some("true"), rht.get("italic"));
        assert_eq!(0, rht.len());
    }

    #[test]
    fn creates_tombstones_for_missing_keys() {
        let mut rht = Rht::new();
        let remove_at = ticket(1, "a");

        let gc_nodes = rht.remove("underline", remove_at.clone());

        assert_eq!(1, gc_nodes.len());
        assert_eq!("underline", gc_nodes[0].key());
        assert_eq!("", gc_nodes[0].value());
        assert_eq!(Some(&remove_at), gc_nodes[0].removed_at());
        assert!(!rht.has("underline"));
        assert_eq!(Some(""), rht.get("underline"));
        assert_eq!(0, rht.len());
    }

    #[test]
    fn replaces_removed_nodes_with_newer_live_values() {
        let mut rht = Rht::new();
        let remove_at = ticket(1, "a");
        let set_at = ticket(2, "a");

        rht.remove("bold", remove_at.clone());
        let (removed, node) = rht.set("bold", "true", set_at);

        assert_eq!(Some(&remove_at), removed.unwrap().removed_at());
        assert_eq!("true", node.unwrap().value());
        assert!(rht.has("bold"));
        assert_eq!(1, rht.len());
    }

    #[test]
    fn returns_previous_removed_node_when_newer_remove_wins() {
        let mut rht = Rht::new();
        let first = ticket(1, "a");
        let second = ticket(2, "a");

        let first_nodes = rht.remove("bold", first.clone());
        let gc_nodes = rht.remove("bold", second.clone());

        assert_eq!(2, gc_nodes.len());
        assert_eq!(first_nodes[0], gc_nodes[0]);
        assert_eq!(Some(&second), gc_nodes[1].removed_at());
        assert_eq!(0, rht.len());
    }

    #[test]
    fn returns_removed_node_when_late_set_loses_to_tombstone() {
        let mut rht = Rht::new();
        let remove_at = ticket(2, "a");
        let set_at = ticket(1, "a");

        rht.remove("bold", remove_at.clone());
        let (removed, node) = rht.set("bold", "true", set_at);

        assert_eq!(Some(&remove_at), removed.unwrap().removed_at());
        assert!(node.is_none());
        assert!(!rht.has("bold"));
        assert_eq!(0, rht.len());
    }

    #[test]
    fn ignores_late_removes() {
        let mut rht = Rht::new();
        let set_at = ticket(2, "a");
        let remove_at = ticket(1, "a");

        rht.set("color", "blue", set_at);

        assert!(rht.remove("color", remove_at).is_empty());
        assert!(rht.has("color"));
        assert_eq!(Some("blue"), rht.get("color"));
        assert_eq!(1, rht.len());
    }

    #[test]
    fn serializes_visible_values_with_sorted_keys() {
        let mut rht = Rht::new();

        rht.set("z", "last", ticket(1, "a"));
        rht.set("a", "quote\"slash\\", ticket(2, "a"));
        rht.remove("z", ticket(3, "a"));

        assert_eq!(r#"{"a":"quote\"slash\\"}"#, rht.to_json());
        let expected = BTreeMap::from([("a".to_owned(), "quote\"slash\\".to_owned())]);
        assert_eq!(expected, rht.to_object());
    }

    #[test]
    fn deepcopies_nodes_and_removed_count() {
        let mut rht = Rht::new();
        rht.set("color", "blue", ticket(1, "a"));
        rht.remove("bold", ticket(2, "a"));

        let copy = rht.deepcopy();
        rht.set("color", "red", ticket(3, "a"));
        rht.set("bold", "true", ticket(4, "a"));

        assert_eq!(Some("blue"), copy.get("color"));
        assert!(!copy.has("bold"));
        assert_eq!(1, copy.len());
    }

    #[test]
    fn purges_only_the_matching_current_tombstone() {
        let mut rht = Rht::new();
        let first = ticket(1, "a");
        let second = ticket(2, "a");

        let old = rht.remove("bold", first).pop().unwrap();
        let current = rht.remove("bold", second).pop().unwrap();

        rht.purge(&old);
        assert!(rht.get_node("bold").is_some());
        assert_eq!(0, rht.len());

        rht.purge(&current);
        assert!(rht.get_node("bold").is_none());
    }

    #[test]
    fn reports_node_identity_and_data_size() {
        let ticket = ticket(3, "a");
        let node = RhtNode::new("naive", "한", ticket.clone(), false);

        assert_eq!(format!("{}:naive", ticket.to_id_string()), node.id_string());
        assert_eq!(None, node.removed_at());
        assert_eq!(10 + 2, node.data_size().data);
        assert_eq!(TIME_TICKET_SIZE, node.data_size().meta);
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
