use super::array::CrdtArray;
use super::counter::CrdtCounter;
use super::element::{CrdtElement, CrdtElementMeta, DataSize};
use super::element_rht::ElementRht;
use super::text::CrdtText;
use super::tree::CrdtTree;
use crate::json::escape_json_string;
use crate::{Result, TimeTicket};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtObject {
    meta: CrdtElementMeta,
    member_nodes: ElementRht,
}

impl CrdtObject {
    pub(crate) fn new(created_at: TimeTicket, member_nodes: ElementRht) -> Self {
        Self {
            meta: CrdtElementMeta::new(created_at),
            member_nodes,
        }
    }

    pub(crate) fn create(created_at: TimeTicket) -> Self {
        Self::new(created_at, ElementRht::new())
    }

    pub(crate) fn create_with_members<I, K>(created_at: TimeTicket, members: I) -> Self
    where
        I: IntoIterator<Item = (K, CrdtElement)>,
        K: Into<String>,
    {
        let mut member_nodes = ElementRht::new();
        for (key, value) in members {
            member_nodes.set(key, value.deepcopy(), value.created_at().clone());
        }

        Self::new(created_at, member_nodes)
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        self.meta.created_at()
    }

    pub(crate) fn id(&self) -> &TimeTicket {
        self.meta.id()
    }

    pub(crate) fn moved_at(&self) -> Option<&TimeTicket> {
        self.meta.moved_at()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.meta.removed_at()
    }

    pub(crate) fn positioned_at(&self) -> &TimeTicket {
        self.meta.positioned_at()
    }

    pub(crate) fn set_moved_at(&mut self, moved_at: Option<TimeTicket>) -> bool {
        self.meta.set_moved_at(moved_at)
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.meta.set_removed_at(removed_at);
    }

    pub(crate) fn remove(&mut self, removed_at: Option<TimeTicket>) -> bool {
        self.meta.remove(removed_at)
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.meta.is_removed()
    }

    pub(crate) fn meta_usage(&self) -> usize {
        self.meta.meta_usage()
    }

    pub(crate) fn data_size(&self) -> DataSize {
        DataSize {
            data: 0,
            meta: self.meta_usage(),
        }
    }

    pub(crate) fn sub_path_of(&self, created_at: &TimeTicket) -> Option<&str> {
        self.member_nodes.sub_path_of(created_at)
    }

    pub(crate) fn purge(&mut self, value: &CrdtElement) -> Result<()> {
        self.member_nodes.purge(value)
    }

    pub(crate) fn set(
        &mut self,
        key: impl Into<String>,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Option<CrdtElement> {
        self.member_nodes.set(key, value, executed_at)
    }

    pub(crate) fn delete(
        &mut self,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        self.member_nodes.delete(created_at, executed_at)
    }

    pub(crate) fn delete_by_key(
        &mut self,
        key: &str,
        executed_at: TimeTicket,
    ) -> Option<CrdtElement> {
        self.member_nodes.delete_by_key(key, executed_at)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&CrdtElement> {
        self.member_nodes.get(key).map(|node| node.value())
    }

    pub(crate) fn get_by_id(&self, created_at: &TimeTicket) -> Option<&CrdtElement> {
        self.member_nodes
            .get_by_id(created_at)
            .map(|node| node.value())
    }

    pub(crate) fn find_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtElement> {
        if let Some(element) = self.get_by_id(created_at) {
            return Some(element);
        }

        for (_, child) in self.iter_all() {
            match child {
                CrdtElement::Object(object) => {
                    if let Some(element) = object.find_by_created_at(created_at) {
                        return Some(element);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(element) = array.find_by_created_at(created_at) {
                        return Some(element);
                    }
                }
                CrdtElement::Primitive(_)
                | CrdtElement::Counter(_)
                | CrdtElement::Text(_)
                | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_object_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtObject> {
        for (_, child) in self.iter_all() {
            match child {
                CrdtElement::Object(object) => {
                    if object.created_at() == created_at {
                        return Some(object);
                    }

                    if let Some(object) = object.find_object_by_created_at(created_at) {
                        return Some(object);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(object) = array.find_object_by_created_at(created_at) {
                        return Some(object);
                    }
                }
                CrdtElement::Primitive(_)
                | CrdtElement::Counter(_)
                | CrdtElement::Text(_)
                | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_object_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtObject> {
        for node in self.member_nodes.iter_mut() {
            match node.value_mut() {
                CrdtElement::Object(object) => {
                    if object.created_at() == created_at {
                        return Some(object);
                    }

                    if let Some(object) = object.find_object_by_created_at_mut(created_at) {
                        return Some(object);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(object) = array.find_object_by_created_at_mut(created_at) {
                        return Some(object);
                    }
                }
                CrdtElement::Primitive(_)
                | CrdtElement::Counter(_)
                | CrdtElement::Text(_)
                | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_array_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtArray> {
        for (_, child) in self.iter_all() {
            match child {
                CrdtElement::Array(array) => {
                    if array.created_at() == created_at {
                        return Some(array);
                    }

                    if let Some(array) = array.find_array_by_created_at(created_at) {
                        return Some(array);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(array) = object.find_array_by_created_at(created_at) {
                        return Some(array);
                    }
                }
                CrdtElement::Primitive(_)
                | CrdtElement::Counter(_)
                | CrdtElement::Text(_)
                | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_array_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtArray> {
        for node in self.member_nodes.iter_mut() {
            match node.value_mut() {
                CrdtElement::Array(array) => {
                    if array.created_at() == created_at {
                        return Some(array);
                    }

                    if let Some(array) = array.find_array_by_created_at_mut(created_at) {
                        return Some(array);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(array) = object.find_array_by_created_at_mut(created_at) {
                        return Some(array);
                    }
                }
                CrdtElement::Primitive(_)
                | CrdtElement::Counter(_)
                | CrdtElement::Text(_)
                | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_text_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtText> {
        for (_, child) in self.iter_all() {
            match child {
                CrdtElement::Text(text) => {
                    if text.created_at() == created_at {
                        return Some(text);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(text) = object.find_text_by_created_at(created_at) {
                        return Some(text);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(text) = array.find_text_by_created_at(created_at) {
                        return Some(text);
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_text_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtText> {
        for node in self.member_nodes.iter_mut() {
            match node.value_mut() {
                CrdtElement::Text(text) => {
                    if text.created_at() == created_at {
                        return Some(text);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(text) = object.find_text_by_created_at_mut(created_at) {
                        return Some(text);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(text) = array.find_text_by_created_at_mut(created_at) {
                        return Some(text);
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_counter_by_created_at(
        &self,
        created_at: &TimeTicket,
    ) -> Option<&CrdtCounter> {
        for (_, child) in self.iter_all() {
            match child {
                CrdtElement::Counter(counter) => {
                    if counter.created_at() == created_at {
                        return Some(counter);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(counter) = object.find_counter_by_created_at(created_at) {
                        return Some(counter);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(counter) = array.find_counter_by_created_at(created_at) {
                        return Some(counter);
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Text(_) | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_counter_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtCounter> {
        for node in self.member_nodes.iter_mut() {
            match node.value_mut() {
                CrdtElement::Counter(counter) => {
                    if counter.created_at() == created_at {
                        return Some(counter);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(counter) = object.find_counter_by_created_at_mut(created_at) {
                        return Some(counter);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(counter) = array.find_counter_by_created_at_mut(created_at) {
                        return Some(counter);
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Text(_) | CrdtElement::Tree(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_tree_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtTree> {
        for (_, child) in self.iter_all() {
            match child {
                CrdtElement::Tree(tree) => {
                    if tree.created_at() == created_at {
                        return Some(tree);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(tree) = object.find_tree_by_created_at(created_at) {
                        return Some(tree);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(tree) = array.find_tree_by_created_at(created_at) {
                        return Some(tree);
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_tree_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtTree> {
        for node in self.member_nodes.iter_mut() {
            match node.value_mut() {
                CrdtElement::Tree(tree) => {
                    if tree.created_at() == created_at {
                        return Some(tree);
                    }
                }
                CrdtElement::Object(object) => {
                    if let Some(tree) = object.find_tree_by_created_at_mut(created_at) {
                        return Some(tree);
                    }
                }
                CrdtElement::Array(array) => {
                    if let Some(tree) = array.find_tree_by_created_at_mut(created_at) {
                        return Some(tree);
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn purge_gc_pair_by_id(&mut self, child_id: &str) -> bool {
        for node in self.member_nodes.iter_mut() {
            match node.value_mut() {
                CrdtElement::Text(text) => {
                    if text.purge_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Object(object) => {
                    if object.purge_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Array(array) => {
                    if array.purge_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Tree(tree) => {
                    if tree.purge_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Primitive(_) | CrdtElement::Counter(_) => {}
            }
        }

        false
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.member_nodes.has(key)
    }

    pub(crate) fn keys(&self) -> Vec<&str> {
        self.iter().map(|(key, _)| key).collect()
    }

    pub(crate) fn to_json(&self) -> String {
        let members = self
            .iter()
            .map(|(key, value)| format!("\"{}\":{}", escape_json_string(key), value.to_json()))
            .collect::<Vec<_>>()
            .join(",");

        format!("{{{members}}}")
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        let keys = self
            .iter()
            .map(|(key, _)| key.to_owned())
            .collect::<BTreeSet<_>>();

        let members = keys
            .into_iter()
            .map(|key| {
                let value = self.member_nodes.get(&key).unwrap().value();
                format!(
                    "\"{}\":{}",
                    escape_json_string(&key),
                    value.to_sorted_json()
                )
            })
            .collect::<Vec<_>>()
            .join(",");

        format!("{{{members}}}")
    }

    pub(crate) fn deepcopy(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            member_nodes: self.member_nodes.deepcopy(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &CrdtElement)> {
        self.member_nodes
            .iter()
            .filter(|node| !node.is_removed())
            .map(|node| (node.str_key(), node.value()))
    }

    pub(crate) fn iter_all(&self) -> impl Iterator<Item = (&str, &CrdtElement)> {
        self.member_nodes
            .iter()
            .map(|node| (node.str_key(), node.value()))
    }
}

#[cfg(test)]
mod tests {
    use super::CrdtObject;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::{TimeTicket, TIME_TICKET_SIZE};

    #[test]
    fn serializes_visible_members() {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        object.set("z", primitive_i32(1, t1.clone()), t1);
        object.set("a", primitive_str("first", t2.clone()), t2);

        assert_eq!(vec!["z", "a"], object.keys());
        assert_eq!(r#"{"z":1,"a":"first"}"#, object.to_json());
        assert_eq!(r#"{"a":"first","z":1}"#, object.to_sorted_json());
    }

    #[test]
    fn gets_members_by_key_and_created_time() {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let created_at = ticket(1, "a");

        object.set(
            "title",
            primitive_str("hello", created_at.clone()),
            created_at.clone(),
        );

        assert!(object.has("title"));
        assert_eq!("\"hello\"", object.get("title").unwrap().to_json());
        assert_eq!(
            "\"hello\"",
            object.get_by_id(&created_at).unwrap().to_json()
        );
        assert_eq!(Some("title"), object.sub_path_of(&created_at));
        assert_eq!(None, object.get("missing"));
    }

    #[test]
    fn deletes_members_by_key_and_keeps_internal_lookup() {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let created_at = ticket(1, "a");
        let removed_at = ticket(2, "a");

        object.set(
            "title",
            primitive_str("hello", created_at.clone()),
            created_at.clone(),
        );
        let removed = object.delete_by_key("title", removed_at.clone()).unwrap();

        assert_eq!("\"hello\"", removed.to_json());
        assert!(!object.has("title"));
        assert_eq!(Vec::<&str>::new(), object.keys());
        assert_eq!(
            Some(&removed_at),
            object.get_by_id(&created_at).unwrap().removed_at()
        );
    }

    #[test]
    fn deletes_members_by_created_time() -> crate::Result<()> {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let created_at = ticket(1, "a");
        let removed_at = ticket(2, "a");

        object.set(
            "title",
            primitive_str("hello", created_at.clone()),
            created_at.clone(),
        );
        let removed = object.delete(&created_at, removed_at.clone())?;

        assert_eq!("\"hello\"", removed.to_json());
        assert!(!object.has("title"));
        assert_eq!(
            Some(&removed_at),
            object.get_by_id(&created_at).unwrap().removed_at()
        );
        Ok(())
    }

    #[test]
    fn does_not_duplicate_keys_for_late_concurrent_sets() {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let t1 = TimeTicket::new(1, 0, "actorB");
        let t2 = TimeTicket::new(2, 0, "actorA");

        object.set("color", primitive_str("red", t2.clone()), t2.clone());
        object.set("color", primitive_str("blue", t1.clone()), t1.clone());

        assert_eq!(vec!["color"], object.keys());
        assert_eq!(r#"{"color":"red"}"#, object.to_json());
        assert_eq!(Some(&t2), object.get_by_id(&t1).unwrap().removed_at());
    }

    #[test]
    fn keeps_one_visible_key_after_multiple_late_sets() {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let t1 = TimeTicket::new(1, 0, "actor2");
        let t2 = TimeTicket::new(2, 0, "actor3");
        let t3 = TimeTicket::new(3, 0, "actor1");

        object.set("key", primitive_str("first", t3.clone()), t3.clone());
        object.set("key", primitive_str("second", t1.clone()), t1.clone());
        object.set("key", primitive_str("third", t2.clone()), t2.clone());

        assert_eq!(vec!["key"], object.keys());
        assert_eq!(r#"{"key":"first"}"#, object.to_json());
        assert_eq!(Some(&t3), object.get_by_id(&t1).unwrap().removed_at());
        assert_eq!(Some(&t3), object.get_by_id(&t2).unwrap().removed_at());
    }

    #[test]
    fn purges_members() -> crate::Result<()> {
        let mut object = CrdtObject::create(TimeTicket::initial());
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        object.set("title", primitive_str("old", t1.clone()), t1.clone());
        object.set("title", primitive_str("new", t2.clone()), t2.clone());

        let old = object.get_by_id(&t1).unwrap().deepcopy();
        object.purge(&old)?;

        assert!(object.get_by_id(&t1).is_none());
        assert_eq!(r#"{"title":"new"}"#, object.to_json());

        let current = object.get_by_id(&t2).unwrap().deepcopy();
        object.purge(&current)?;

        assert!(object.get_by_id(&t2).is_none());
        assert_eq!("{}", object.to_json());
        Ok(())
    }

    #[test]
    fn deepcopies_object_members_and_metadata() {
        let mut object = CrdtObject::create(ticket(1, "a"));
        let moved_at = ticket(2, "a");
        let removed_at = ticket(3, "a");
        let member_created_at = ticket(4, "a");

        object.set_moved_at(Some(moved_at.clone()));
        object.set_removed_at(Some(removed_at.clone()));
        object.set(
            "title",
            primitive_str("hello", member_created_at.clone()),
            member_created_at.clone(),
        );

        let copy = object.deepcopy();
        object.delete_by_key("title", ticket(5, "a"));

        assert_eq!(Some(&moved_at), copy.moved_at());
        assert_eq!(Some(&removed_at), copy.removed_at());
        assert_eq!(r#"{"title":"hello"}"#, copy.to_json());
        assert_eq!(None, object.get("title"));
    }

    #[test]
    fn nests_objects() {
        let mut root = CrdtObject::create(TimeTicket::initial());
        let profile_created_at = ticket(1, "a");
        let name_created_at = ticket(2, "a");
        let mut profile = CrdtObject::create(profile_created_at.clone());

        profile.set(
            "name",
            primitive_str("yorkie", name_created_at.clone()),
            name_created_at,
        );
        root.set(
            "profile",
            CrdtElement::object(profile),
            profile_created_at.clone(),
        );

        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, root.to_json());
        assert_eq!(
            &profile_created_at,
            root.get("profile").unwrap().created_at()
        );
    }

    #[test]
    fn reports_object_data_size() {
        let object = CrdtObject::create(TimeTicket::initial());

        assert_eq!(0, object.data_size().data);
        assert_eq!(TIME_TICKET_SIZE, object.data_size().meta);
    }

    #[test]
    fn creates_object_with_deepcopied_members() {
        let member_created_at = ticket(1, "a");
        let object = CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("title", primitive_str("hello", member_created_at.clone()))],
        );

        assert_eq!(r#"{"title":"hello"}"#, object.to_json());
        assert_eq!(
            Some(&member_created_at),
            object.get("title").unwrap().moved_at()
        );
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn primitive_i32(value: i32, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::Integer(value),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
