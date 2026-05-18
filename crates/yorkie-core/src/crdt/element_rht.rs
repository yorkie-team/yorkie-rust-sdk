use super::element::CrdtElement;
use crate::{Result, TimeTicket, YorkieError};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementRhtNode {
    str_key: String,
    value: CrdtElement,
}

impl ElementRhtNode {
    fn new(str_key: String, value: CrdtElement) -> Self {
        Self { str_key, value }
    }

    pub(crate) fn str_key(&self) -> &str {
        &self.str_key
    }

    pub(crate) fn value(&self) -> &CrdtElement {
        &self.value
    }

    pub(crate) fn value_mut(&mut self) -> &mut CrdtElement {
        &mut self.value
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.value.is_removed()
    }

    fn remove(&mut self, removed_at: TimeTicket) -> bool {
        self.value.remove(Some(removed_at))
    }

    fn deepcopy(&self) -> Self {
        Self {
            str_key: self.str_key.clone(),
            value: self.value.deepcopy(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementRht {
    node_by_key: BTreeMap<String, String>,
    node_by_created_at: BTreeMap<String, ElementRhtNode>,
    created_order: Vec<String>,
}

impl ElementRht {
    pub(crate) fn new() -> Self {
        Self {
            node_by_key: BTreeMap::new(),
            node_by_created_at: BTreeMap::new(),
            created_order: Vec::new(),
        }
    }

    pub(crate) fn set(
        &mut self,
        key: impl Into<String>,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Option<CrdtElement> {
        let key = key.into();
        let current_id = self.node_by_key.get(&key).cloned();
        let mut removed = None;

        if let Some(current_id) = current_id.as_deref() {
            if let Some(node) = self.node_by_created_at.get_mut(current_id) {
                if !node.is_removed() && node.remove(executed_at.clone()) {
                    removed = Some(node.value().deepcopy());
                }
            }
        }

        let current_state = current_id
            .as_deref()
            .and_then(|current_id| self.node_by_created_at.get(current_id))
            .map(|node| (node.is_removed(), node.value().positioned_at().clone()));

        let new_id = value.created_at().to_id_string();
        let inserted = self
            .node_by_created_at
            .insert(new_id.clone(), ElementRhtNode::new(key.clone(), value))
            .is_none();

        if inserted {
            self.created_order.push(new_id.clone());
        }

        match current_state {
            None => {
                self.node_by_key.insert(key, new_id.clone());
                self.set_new_node_moved_at(&new_id, executed_at);
            }
            Some((_, positioned_at)) if executed_at.after(&positioned_at) => {
                self.node_by_key.insert(key, new_id.clone());
                self.set_new_node_moved_at(&new_id, executed_at);
            }
            Some((false, positioned_at)) => {
                if let Some(node) = self.node_by_created_at.get_mut(&new_id) {
                    node.value.remove(Some(positioned_at));
                }
            }
            Some((true, _)) => {}
        }

        removed
    }

    pub(crate) fn set_internal(&mut self, key: impl Into<String>, value: CrdtElement) {
        let key = key.into();
        let new_id = value.created_at().to_id_string();
        let is_visible = !value.is_removed();
        let positioned_at = value.positioned_at().clone();
        let inserted = self
            .node_by_created_at
            .insert(new_id.clone(), ElementRhtNode::new(key.clone(), value))
            .is_none();

        if inserted {
            self.created_order.push(new_id.clone());
        }

        if !is_visible {
            return;
        }

        let should_be_visible = self
            .node_by_key
            .get(&key)
            .and_then(|current_id| self.node_by_created_at.get(current_id))
            .map(|current| positioned_at.after(current.value().positioned_at()))
            .unwrap_or(true);

        if should_be_visible {
            self.node_by_key.insert(key, new_id);
        }
    }

    pub(crate) fn delete(
        &mut self,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        let created_id = created_at.to_id_string();
        let node = self
            .node_by_created_at
            .get_mut(&created_id)
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_id.clone()))?;

        node.remove(executed_at);
        Ok(node.value().deepcopy())
    }

    pub(crate) fn sub_path_of(&self, created_at: &TimeTicket) -> Option<&str> {
        self.node_by_created_at
            .get(&created_at.to_id_string())
            .map(ElementRhtNode::str_key)
    }

    pub(crate) fn purge(&mut self, element: &CrdtElement) -> Result<()> {
        let created_id = element.created_at().to_id_string();
        let str_key = self
            .node_by_created_at
            .get(&created_id)
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_id.clone()))?
            .str_key()
            .to_owned();

        if self
            .node_by_key
            .get(&str_key)
            .is_some_and(|current_id| current_id == &created_id)
        {
            self.node_by_key.remove(&str_key);
        }

        self.node_by_created_at.remove(&created_id);
        self.created_order
            .retain(|current_id| current_id != &created_id);

        Ok(())
    }

    pub(crate) fn delete_by_key(
        &mut self,
        key: &str,
        removed_at: TimeTicket,
    ) -> Option<CrdtElement> {
        let created_id = self.node_by_key.get(key)?.clone();
        let node = self.node_by_created_at.get_mut(&created_id)?;

        if node.remove(removed_at) {
            return Some(node.value().deepcopy());
        }

        None
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub(crate) fn get_by_id(&self, created_at: &TimeTicket) -> Option<&ElementRhtNode> {
        self.node_by_created_at.get(&created_at.to_id_string())
    }

    pub(crate) fn get(&self, key: &str) -> Option<&ElementRhtNode> {
        let created_id = self.node_by_key.get(key)?;
        let node = self.node_by_created_at.get(created_id)?;

        (!node.is_removed()).then_some(node)
    }

    pub(crate) fn deepcopy(&self) -> Self {
        Self {
            node_by_key: self.node_by_key.clone(),
            node_by_created_at: self
                .node_by_created_at
                .iter()
                .map(|(created_id, node)| (created_id.clone(), node.deepcopy()))
                .collect(),
            created_order: self.created_order.clone(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &ElementRhtNode> {
        self.created_order
            .iter()
            .filter_map(|created_id| self.node_by_created_at.get(created_id))
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut ElementRhtNode> {
        self.node_by_created_at.values_mut()
    }

    fn set_new_node_moved_at(&mut self, created_id: &str, executed_at: TimeTicket) {
        if let Some(node) = self.node_by_created_at.get_mut(created_id) {
            node.value.set_moved_at(Some(executed_at));
        }
    }
}

impl Default for ElementRht {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ElementRht;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::{TimeTicket, YorkieError};

    #[test]
    fn keeps_the_latest_position_for_a_key() {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        rht.set("title", primitive("one", t1.clone()), t1.clone());
        let removed = rht.set("title", primitive("two", t2.clone()), t2.clone());

        assert_eq!("\"two\"", rht.get("title").unwrap().value().to_json());
        assert_eq!("\"one\"", removed.unwrap().to_json());
        assert!(rht.get_by_id(&t1).unwrap().is_removed());
        assert_eq!(Some(&t2), rht.get_by_id(&t1).unwrap().value().removed_at());
        assert_eq!(vec!["title"], visible_keys(&rht));
    }

    #[test]
    fn ignores_late_concurrent_sets_for_the_same_key() {
        let mut rht = ElementRht::new();
        let t1 = ticket_with_actor(1, "b");
        let t2 = ticket_with_actor(2, "a");

        rht.set("color", primitive("red", t2.clone()), t2.clone());
        let removed = rht.set("color", primitive("blue", t1.clone()), t1.clone());

        assert!(removed.is_none());
        assert_eq!("\"red\"", rht.get("color").unwrap().value().to_json());
        assert_eq!(Some(&t2), rht.get_by_id(&t1).unwrap().value().removed_at());
        assert_eq!(vec!["color"], visible_keys(&rht));
    }

    #[test]
    fn keeps_one_visible_entry_after_multiple_late_sets() {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");

        rht.set("status", primitive("done", t3.clone()), t3.clone());
        rht.set("status", primitive("open", t1.clone()), t1.clone());
        rht.set("status", primitive("pending", t2.clone()), t2.clone());

        assert_eq!("\"done\"", rht.get("status").unwrap().value().to_json());
        assert_eq!(Some(&t3), rht.get_by_id(&t1).unwrap().value().removed_at());
        assert_eq!(Some(&t3), rht.get_by_id(&t2).unwrap().value().removed_at());
        assert_eq!(vec!["status"], visible_keys(&rht));
    }

    #[test]
    fn deletes_current_node_by_key() {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        rht.set("title", primitive("hello", t1.clone()), t1.clone());
        let removed = rht.delete_by_key("title", t2.clone()).unwrap();

        assert_eq!("\"hello\"", removed.to_json());
        assert!(!rht.has("title"));
        assert!(rht.get("title").is_none());
        assert_eq!(Some(&t2), rht.get_by_id(&t1).unwrap().value().removed_at());
        assert_eq!(Vec::<&str>::new(), visible_keys(&rht));
    }

    #[test]
    fn deletes_node_by_created_time() -> crate::Result<()> {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        rht.set("title", primitive("hello", t1.clone()), t1.clone());
        let removed = rht.delete(&t1, t2.clone())?;

        assert_eq!("\"hello\"", removed.to_json());
        assert!(!rht.has("title"));
        assert_eq!(Some(&t2), rht.get_by_id(&t1).unwrap().value().removed_at());
        Ok(())
    }

    #[test]
    fn reports_missing_node_when_delete_target_is_unknown() {
        let mut rht = ElementRht::new();
        let missing = ticket(1, "a");
        let executed_at = ticket(2, "a");

        let err = rht.delete(&missing, executed_at).unwrap_err();

        assert_eq!(YorkieError::MissingCrdtElement(missing.to_id_string()), err);
    }

    #[test]
    fn purges_nodes_without_disturbing_other_current_entries() -> crate::Result<()> {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        rht.set("title", primitive("old", t1.clone()), t1.clone());
        rht.set("title", primitive("new", t2.clone()), t2.clone());

        let old = rht.get_by_id(&t1).unwrap().value().deepcopy();
        rht.purge(&old)?;

        assert!(rht.get_by_id(&t1).is_none());
        assert_eq!("\"new\"", rht.get("title").unwrap().value().to_json());

        let current = rht.get_by_id(&t2).unwrap().value().deepcopy();
        rht.purge(&current)?;

        assert!(rht.get_by_id(&t2).is_none());
        assert!(!rht.has("title"));
        Ok(())
    }

    #[test]
    fn returns_sub_path_for_created_time() {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");

        rht.set("title", primitive("hello", t1.clone()), t1.clone());

        assert_eq!(Some("title"), rht.sub_path_of(&t1));
        assert_eq!(None, rht.sub_path_of(&ticket(2, "a")));
    }

    #[test]
    fn deepcopies_nodes_and_key_indexes() {
        let mut rht = ElementRht::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");

        rht.set("title", primitive("old", t1.clone()), t1.clone());
        rht.set("title", primitive("new", t2.clone()), t2.clone());

        let copy = rht.deepcopy();
        rht.delete_by_key("title", t3.clone());

        assert!(!rht.has("title"));
        assert_eq!("\"new\"", copy.get("title").unwrap().value().to_json());
        assert_eq!(Some(&t2), copy.get("title").unwrap().value().moved_at());
        assert_eq!(vec!["title"], visible_keys(&copy));
    }

    fn primitive(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        ticket_with_actor(lamport, actor_id)
    }

    fn ticket_with_actor(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }

    fn visible_keys(rht: &ElementRht) -> Vec<&str> {
        rht.iter()
            .filter(|node| !node.is_removed())
            .map(|node| node.str_key())
            .collect()
    }
}
