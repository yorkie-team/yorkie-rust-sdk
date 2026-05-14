use super::element::{CrdtElement, CrdtElementMeta, DataSize};
use super::object::CrdtObject;
use super::rga_tree_list::{RgaTreeList, RgaTreeListNode};
use super::text::CrdtText;
use crate::{Result, TimeTicket};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtArray {
    meta: CrdtElementMeta,
    elements: RgaTreeList,
}

impl CrdtArray {
    pub(crate) fn new(created_at: TimeTicket, elements: RgaTreeList) -> Self {
        Self {
            meta: CrdtElementMeta::new(created_at),
            elements,
        }
    }

    pub(crate) fn create(created_at: TimeTicket) -> Self {
        Self::new(created_at, RgaTreeList::new())
    }

    pub(crate) fn create_with_elements<I>(created_at: TimeTicket, elements: I) -> Result<Self>
    where
        I: IntoIterator<Item = CrdtElement>,
    {
        let mut list = RgaTreeList::new();
        for element in elements {
            list.add(element.deepcopy())?;
        }

        Ok(Self::new(created_at, list))
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

    pub(crate) fn sub_path_of(&self, created_at: &TimeTicket) -> Option<String> {
        self.elements.sub_path_of(created_at)
    }

    pub(crate) fn purge(&mut self, value: &CrdtElement) -> Result<()> {
        self.elements.purge(value)
    }

    pub(crate) fn insert_after(
        &mut self,
        prev_created_at: &TimeTicket,
        value: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Result<RgaTreeListNode> {
        self.elements
            .insert_after(prev_created_at, value, executed_at)
    }

    pub(crate) fn move_after(
        &mut self,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<Option<RgaTreeListNode>> {
        self.elements
            .move_after(prev_created_at, created_at, executed_at)
    }

    pub(crate) fn get(&self, index: usize) -> Option<&CrdtElement> {
        self.elements
            .get_by_index(index)
            .and_then(RgaTreeListNode::element)
    }

    pub(crate) fn get_by_id(&self, created_at: &TimeTicket) -> Option<&CrdtElement> {
        self.elements
            .get_by_id(created_at)
            .and_then(RgaTreeListNode::element)
    }

    pub(crate) fn get_prev_created_at(&self, created_at: &TimeTicket) -> Result<TimeTicket> {
        self.elements.find_prev_created_at(created_at)
    }

    pub(crate) fn pos_created_at(&self, element_created_at: &TimeTicket) -> Result<TimeTicket> {
        self.elements.pos_created_at(element_created_at)
    }

    pub(crate) fn delete(
        &mut self,
        created_at: &TimeTicket,
        edited_at: TimeTicket,
    ) -> Result<CrdtElement> {
        self.elements.delete(created_at, edited_at)
    }

    pub(crate) fn delete_by_index(
        &mut self,
        index: usize,
        edited_at: TimeTicket,
    ) -> Result<Option<CrdtElement>> {
        self.elements.delete_by_index(index, edited_at)
    }

    pub(crate) fn set(
        &mut self,
        created_at: &TimeTicket,
        element: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        self.elements.set(created_at, element, executed_at)
    }

    pub(crate) fn get_last_created_at(&self) -> TimeTicket {
        self.elements.last_created_at()
    }

    pub(crate) fn len(&self) -> usize {
        self.elements.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub(crate) fn find_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtElement> {
        if let Some(element) = self.get_by_id(created_at) {
            return Some(element);
        }

        for child in self.iter_all() {
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
                CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_object_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtObject> {
        for child in self.iter_all() {
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
                CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_object_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtObject> {
        for node in self.elements.iter_all_mut() {
            let Some(child) = node.element_mut() else {
                continue;
            };

            match child {
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
                CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_array_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtArray> {
        for child in self.iter_all() {
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
                CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_array_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtArray> {
        for node in self.elements.iter_all_mut() {
            let Some(child) = node.element_mut() else {
                continue;
            };

            match child {
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
                CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_text_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtText> {
        for child in self.iter_all() {
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
                CrdtElement::Primitive(_) => {}
            }
        }

        None
    }

    pub(crate) fn find_text_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtText> {
        for node in self.elements.iter_all_mut() {
            let Some(child) = node.element_mut() else {
                continue;
            };

            match child {
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
                CrdtElement::Primitive(_) => {}
            }
        }

        None
    }

    pub(crate) fn purge_text_gc_pair_by_id(&mut self, child_id: &str) -> bool {
        for node in self.elements.iter_all_mut() {
            let Some(child) = node.element_mut() else {
                continue;
            };

            match child {
                CrdtElement::Text(text) => {
                    if text.purge_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Object(object) => {
                    if object.purge_text_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Array(array) => {
                    if array.purge_text_gc_pair_by_id(child_id) {
                        return true;
                    }
                }
                CrdtElement::Primitive(_) => {}
            }
        }

        false
    }

    pub(crate) fn to_json(&self) -> String {
        self.elements.to_json()
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.elements.to_sorted_json()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            elements: self.elements.deepcopy(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CrdtElement> {
        self.elements.iter().filter_map(RgaTreeListNode::element)
    }

    pub(crate) fn iter_all(&self) -> impl Iterator<Item = &CrdtElement> {
        self.elements
            .iter_all()
            .filter_map(RgaTreeListNode::element)
    }

    pub(crate) fn iter_all_nodes(&self) -> impl Iterator<Item = &RgaTreeListNode> {
        self.elements.iter_all()
    }
}

#[cfg(test)]
mod tests {
    use super::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::TimeTicket;

    #[test]
    fn serializes_visible_elements() -> crate::Result<()> {
        let mut array = CrdtArray::create(ticket(1, "a"));
        let one_at = ticket(2, "a");
        let two_at = ticket(3, "a");

        array.insert_after(
            &TimeTicket::initial(),
            primitive("one", one_at.clone()),
            Some(one_at),
        )?;
        array.insert_after(
            &array.get_last_created_at(),
            primitive("two", two_at.clone()),
            None,
        )?;
        array.delete(&two_at, ticket(4, "a"))?;

        assert_eq!(1, array.len());
        assert_eq!(r#"["one"]"#, array.to_json());
        Ok(())
    }

    #[test]
    fn finds_nested_descendants() -> crate::Result<()> {
        let array_at = ticket(1, "a");
        let object_at = ticket(2, "a");
        let name_at = ticket(3, "a");
        let mut object = CrdtObject::create(object_at.clone());
        object.set(
            "name",
            primitive("yorkie", name_at.clone()),
            name_at.clone(),
        );
        let array = CrdtArray::create_with_elements(array_at, vec![CrdtElement::object(object)])?;

        assert!(array.find_by_created_at(&name_at).is_some());
        assert!(array.find_object_by_created_at(&object_at).is_some());
        assert_eq!(r#"[{"name":"yorkie"}]"#, array.to_json());
        Ok(())
    }

    fn primitive(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
