use super::element::{CrdtElement, DataSize};
use super::object::CrdtObject;
use super::rga_tree_list::RgaTreeListNode;
use super::text::CrdtText;
use crate::{Result, TimeTicket, YorkieError, TIME_TICKET_SIZE};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtElementPair {
    element: CrdtElement,
    parent: Option<CrdtElement>,
}

impl CrdtElementPair {
    fn new(element: CrdtElement, parent: Option<CrdtElement>) -> Self {
        Self { element, parent }
    }

    pub(crate) fn element(&self) -> &CrdtElement {
        &self.element
    }

    pub(crate) fn parent(&self) -> Option<&CrdtElement> {
        self.parent.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrdtGcPair {
    child_id: String,
    child_size: DataSize,
    removed_at: TimeTicket,
}

impl CrdtGcPair {
    fn new(child_id: String, child_size: DataSize, removed_at: TimeTicket) -> Self {
        Self {
            child_id,
            child_size,
            removed_at,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DocSize {
    pub(crate) live: DataSize,
    pub(crate) gc: DataSize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RootStats {
    pub(crate) elements: usize,
    pub(crate) gc_elements: usize,
    pub(crate) gc_pairs: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtRoot {
    root_object: CrdtObject,
    element_pair_by_created_at: BTreeMap<String, CrdtElementPair>,
    gc_element_set_by_created_at: BTreeSet<String>,
    gc_pair_by_child_id: BTreeMap<String, CrdtGcPair>,
    doc_size: DocSize,
}

impl CrdtRoot {
    pub(crate) fn new(root_object: CrdtObject) -> Self {
        let mut root = Self {
            root_object,
            element_pair_by_created_at: BTreeMap::new(),
            gc_element_set_by_created_at: BTreeSet::new(),
            gc_pair_by_child_id: BTreeMap::new(),
            doc_size: DocSize::default(),
        };
        let root_element = root.root_element();
        root.register_element(&root_element, None);
        root.register_removed_descendants(&root_element);
        root.register_gc_pairs(&root_element);
        root
    }

    pub(crate) fn create() -> Self {
        Self::new(CrdtObject::create(TimeTicket::initial()))
    }

    pub(crate) fn find_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtElement> {
        self.element_pair_by_created_at
            .get(&created_at.to_id_string())
            .map(CrdtElementPair::element)
    }

    pub(crate) fn find_element_pair_by_created_at(
        &self,
        created_at: &TimeTicket,
    ) -> Option<&CrdtElementPair> {
        self.element_pair_by_created_at
            .get(&created_at.to_id_string())
    }

    pub(crate) fn create_sub_paths(&self, created_at: &TimeTicket) -> Result<Vec<String>> {
        let Some(mut pair) = self.find_element_pair_by_created_at(created_at) else {
            return Ok(Vec::new());
        };

        let mut sub_paths = Vec::new();
        while let Some(parent) = pair.parent() {
            let child_created_at = pair.element().created_at();
            let sub_path = sub_path_of(parent, child_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(child_created_at.to_id_string()))?;
            sub_paths.insert(0, sub_path);

            pair = self
                .find_element_pair_by_created_at(parent.created_at())
                .ok_or_else(|| {
                    YorkieError::MissingCrdtElement(parent.created_at().to_id_string())
                })?;
        }

        sub_paths.insert(0, "$".to_owned());
        Ok(sub_paths)
    }

    pub(crate) fn create_path(&self, created_at: &TimeTicket) -> Result<String> {
        Ok(self.create_sub_paths(created_at)?.join("."))
    }

    pub(crate) fn register_element(&mut self, element: &CrdtElement, parent: Option<&CrdtElement>) {
        self.register_element_internal(element, parent);
    }

    pub(crate) fn deregister_element(&mut self, element: &CrdtElement) -> usize {
        let mut count = 0;
        self.deregister_element_internal(element, &mut count);
        count
    }

    pub(crate) fn register_removed_element(&mut self, element: &CrdtElement) {
        let created_at = element.created_at().to_id_string();
        if let Some(pair) = self.element_pair_by_created_at.get_mut(&created_at) {
            pair.element = element.deepcopy();
        }

        add_data_size(&mut self.doc_size.gc, element.data_size());
        sub_data_size(&mut self.doc_size.live, element.data_size());
        self.doc_size.live.meta += TIME_TICKET_SIZE;
        self.gc_element_set_by_created_at.insert(created_at);
    }

    pub(crate) fn register_gc_pair(&mut self, child: &RgaTreeListNode) {
        if let Some(removed_at) = child.removed_at() {
            self.register_gc_pair_by_id(child.id_string(), child.data_size(), removed_at.clone());
        }
    }

    pub(crate) fn register_gc_pair_by_id(
        &mut self,
        child_id: String,
        child_size: DataSize,
        removed_at: TimeTicket,
    ) {
        if let Some(pair) = self.gc_pair_by_child_id.remove(&child_id) {
            sub_data_size(&mut self.doc_size.gc, pair.child_size);
            return;
        }

        let pair = CrdtGcPair::new(child_id.clone(), child_size, removed_at);
        add_data_size(&mut self.doc_size.gc, pair.child_size);
        self.gc_pair_by_child_id.insert(child_id, pair);
    }

    pub(crate) fn get_object_member(
        &self,
        parent_created_at: &TimeTicket,
        key: &str,
    ) -> Result<Option<&CrdtElement>> {
        let object = self
            .object_by_created_at(parent_created_at)
            .ok_or_else(|| self.object_parent_error(parent_created_at))?;

        Ok(object.get(key))
    }

    pub(crate) fn object_member_sub_path(
        &self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> Result<Option<&str>> {
        let object = self
            .object_by_created_at(parent_created_at)
            .ok_or_else(|| self.object_parent_error(parent_created_at))?;

        Ok(object.sub_path_of(created_at))
    }

    pub(crate) fn container_sub_path(
        &self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> Result<Option<String>> {
        if let Some(object) = self.object_by_created_at(parent_created_at) {
            return Ok(object.sub_path_of(created_at).map(ToOwned::to_owned));
        }

        if let Some(array) = self.array_by_created_at(parent_created_at) {
            return Ok(array.sub_path_of(created_at));
        }

        Err(self.container_parent_error(parent_created_at))
    }

    pub(crate) fn set_object_member(
        &mut self,
        parent_created_at: &TimeTicket,
        key: impl Into<String>,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<(CrdtElement, Option<CrdtElement>)> {
        let key = key.into();
        let value_created_at = value.created_at().clone();

        if self.object_by_created_at(parent_created_at).is_none() {
            return Err(self.object_parent_error(parent_created_at));
        }

        let (inserted, removed) = {
            let object = self
                .object_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            let removed = object.set(key, value, executed_at);
            let inserted = object
                .get_by_id(&value_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
                .deepcopy();

            (inserted, removed)
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);

        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

        self.register_element(&inserted, Some(&parent));
        if let Some(removed) = &removed {
            self.register_removed_element(removed);
        }
        if inserted.removed_at().is_some() {
            self.register_removed_element(&inserted);
        }

        Ok((inserted, removed))
    }

    pub(crate) fn remove_object_member(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.object_by_created_at(parent_created_at).is_none() {
            return Err(self.object_parent_error(parent_created_at));
        }

        let removed = {
            let object = self
                .object_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            object.delete(created_at, executed_at)?
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);
        if removed.removed_at().is_some() {
            self.register_removed_element(&removed);
        }

        Ok(removed)
    }

    pub(crate) fn get_container_child(
        &self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> Result<Option<CrdtElement>> {
        if let Some(object) = self.object_by_created_at(parent_created_at) {
            return Ok(object.get_by_id(created_at).map(CrdtElement::deepcopy));
        }

        if let Some(array) = self.array_by_created_at(parent_created_at) {
            return Ok(array.get_by_id(created_at).map(CrdtElement::deepcopy));
        }

        Err(self.container_parent_error(parent_created_at))
    }

    pub(crate) fn insert_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        let value_created_at = value.created_at().clone();

        if self.array_by_created_at(parent_created_at).is_none() {
            return Err(self.array_parent_error(parent_created_at));
        }

        let inserted = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            array.insert_after(prev_created_at, value, Some(executed_at))?;
            array
                .get_by_id(&value_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
                .deepcopy()
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);

        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

        self.register_element(&inserted, Some(&parent));
        if inserted.removed_at().is_some() {
            self.register_removed_element(&inserted);
        }

        Ok(inserted)
    }

    pub(crate) fn move_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.array_by_created_at(parent_created_at).is_none() {
            return Err(self.array_parent_error(parent_created_at));
        }

        let (moved, dead_node) = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            let dead_node = array.move_after(prev_created_at, created_at, executed_at)?;
            let dead_node = dead_node.filter(|node| node.removed_at().is_some());
            let moved = array
                .get_by_id(created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?
                .deepcopy();

            (moved, dead_node)
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);
        if let Some(dead_node) = &dead_node {
            self.register_gc_pair(dead_node);
        }
        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;
        if let Some(pair) = self
            .element_pair_by_created_at
            .get_mut(&created_at.to_id_string())
        {
            pair.element = moved.deepcopy();
            pair.parent = Some(parent);
        }

        Ok(moved)
    }

    pub(crate) fn set_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        value: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<(CrdtElement, CrdtElement)> {
        let value_created_at = value.created_at().clone();

        if self.array_by_created_at(parent_created_at).is_none() {
            return Err(self.array_parent_error(parent_created_at));
        }

        let (inserted, removed) = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            let removed = array.set(created_at, value, executed_at)?;
            let inserted = array
                .get_by_id(&value_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(value_created_at.to_id_string()))?
                .deepcopy();

            (inserted, removed)
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);

        let parent = self
            .actual_element_by_created_at(parent_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

        self.register_element(&inserted, Some(&parent));
        self.register_removed_element(&removed);
        if inserted.removed_at().is_some() {
            self.register_removed_element(&inserted);
        }

        Ok((inserted, removed))
    }

    pub(crate) fn remove_container_element(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.object_by_created_at(parent_created_at).is_some() {
            return self.remove_object_member(parent_created_at, created_at, executed_at);
        }

        if self.array_by_created_at(parent_created_at).is_some() {
            return self.remove_array_element(parent_created_at, created_at, executed_at);
        }

        Err(self.container_parent_error(parent_created_at))
    }

    pub(crate) fn get_element_map_size(&self) -> usize {
        self.element_pair_by_created_at.len()
    }

    pub(crate) fn get_garbage_element_set_size(&self) -> usize {
        let mut seen = BTreeSet::new();

        for created_at in &self.gc_element_set_by_created_at {
            seen.insert(created_at.clone());
            if let Some(pair) = self.element_pair_by_created_at.get(created_at) {
                collect_descendant_ids(pair.element(), &mut seen);
            }
        }

        seen.len()
    }

    pub(crate) fn get_garbage_len(&self) -> usize {
        self.get_garbage_element_set_size() + self.gc_pair_by_child_id.len()
    }

    pub(crate) fn garbage_collect(&mut self, vector: &crate::VersionVector) -> Result<usize> {
        let mut count = 0;

        let removed_element_ids = self
            .gc_element_set_by_created_at
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for created_id in removed_element_ids {
            let Some(pair) = self.element_pair_by_created_at.get(&created_id).cloned() else {
                continue;
            };
            let Some(removed_at) = pair.element().removed_at() else {
                continue;
            };
            if !vector.after_or_equal(removed_at) {
                continue;
            }

            if let Some(parent) = pair.parent() {
                if let Some(object) = self.object_by_created_at_mut(parent.created_at()) {
                    object.purge(pair.element())?;
                } else if let Some(array) = self.array_by_created_at_mut(parent.created_at()) {
                    array.purge(pair.element())?;
                }
            }

            count += self.deregister_element(pair.element());
        }

        let gc_pairs = self
            .gc_pair_by_child_id
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for pair in gc_pairs {
            if !vector.after_or_equal(&pair.removed_at) {
                continue;
            }

            if self.purge_gc_pair_by_child_id(&pair.child_id) {
                if let Some(pair) = self.gc_pair_by_child_id.remove(&pair.child_id) {
                    sub_data_size(&mut self.doc_size.gc, pair.child_size);
                }
                count += 1;
            }
        }

        Ok(count)
    }

    pub(crate) fn object(&self) -> &CrdtObject {
        &self.root_object
    }

    pub(crate) fn object_mut(&mut self) -> &mut CrdtObject {
        &mut self.root_object
    }

    pub(crate) fn object_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtObject> {
        if self.root_object.created_at() == created_at {
            return Some(&self.root_object);
        }

        self.root_object.find_object_by_created_at(created_at)
    }

    pub(crate) fn object_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtObject> {
        if self.root_object.created_at() == created_at {
            return Some(&mut self.root_object);
        }

        self.root_object.find_object_by_created_at_mut(created_at)
    }

    pub(crate) fn array_by_created_at(
        &self,
        created_at: &TimeTicket,
    ) -> Option<&super::array::CrdtArray> {
        self.root_object.find_array_by_created_at(created_at)
    }

    pub(crate) fn array_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut super::array::CrdtArray> {
        self.root_object.find_array_by_created_at_mut(created_at)
    }

    pub(crate) fn text_by_created_at(&self, created_at: &TimeTicket) -> Option<&CrdtText> {
        self.root_object.find_text_by_created_at(created_at)
    }

    pub(crate) fn text_by_created_at_mut(
        &mut self,
        created_at: &TimeTicket,
    ) -> Option<&mut CrdtText> {
        self.root_object.find_text_by_created_at_mut(created_at)
    }

    pub(crate) fn doc_size(&self) -> DocSize {
        self.doc_size
    }

    pub(crate) fn deepcopy(&self) -> Self {
        Self::new(self.root_object.deepcopy())
    }

    pub(crate) fn to_json(&self) -> String {
        self.root_object.to_json()
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.root_object.to_sorted_json()
    }

    pub(crate) fn stats(&self) -> RootStats {
        RootStats {
            elements: self.get_element_map_size(),
            gc_elements: self.get_garbage_element_set_size(),
            gc_pairs: self.gc_pair_by_child_id.len(),
        }
    }

    pub(crate) fn root_element(&self) -> CrdtElement {
        CrdtElement::object(self.root_object.deepcopy())
    }

    fn actual_element_by_created_at(&self, created_at: &TimeTicket) -> Option<CrdtElement> {
        if self.root_object.created_at() == created_at {
            return Some(self.root_element());
        }

        self.root_object
            .find_by_created_at(created_at)
            .map(CrdtElement::deepcopy)
    }

    fn refresh_element_pair(&mut self, created_at: &TimeTicket) {
        let Some(element) = self.actual_element_by_created_at(created_at) else {
            return;
        };

        if let Some(pair) = self
            .element_pair_by_created_at
            .get_mut(&created_at.to_id_string())
        {
            pair.element = element;
        }
    }

    fn refresh_element_pair_and_ancestors(&mut self, created_at: &TimeTicket) {
        let mut current = Some(created_at.clone());

        while let Some(created_at) = current {
            current = self
                .find_element_pair_by_created_at(&created_at)
                .and_then(|pair| pair.parent())
                .map(|parent| parent.created_at().clone());

            self.refresh_element_pair(&created_at);
        }
    }

    fn object_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "object",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn array_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "array",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn container_parent_error(&self, parent_created_at: &TimeTicket) -> YorkieError {
        if self.find_by_created_at(parent_created_at).is_some() {
            return YorkieError::UnexpectedCrdtElement {
                id: parent_created_at.to_id_string(),
                expected: "object or array",
            };
        }

        YorkieError::MissingCrdtElement(parent_created_at.to_id_string())
    }

    fn remove_array_element(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        let removed = {
            let array = self
                .array_by_created_at_mut(parent_created_at)
                .ok_or_else(|| YorkieError::MissingCrdtElement(parent_created_at.to_id_string()))?;

            array.delete(created_at, executed_at)?
        };

        self.refresh_element_pair_and_ancestors(parent_created_at);
        if removed.removed_at().is_some() {
            self.register_removed_element(&removed);
        }

        Ok(removed)
    }

    fn register_element_internal(&mut self, element: &CrdtElement, parent: Option<&CrdtElement>) {
        self.element_pair_by_created_at.insert(
            element.created_at().to_id_string(),
            CrdtElementPair::new(element.deepcopy(), parent.map(|parent| parent.deepcopy())),
        );
        add_data_size(&mut self.doc_size.live, element.data_size());

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_element_internal(child, Some(element));
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.register_element_internal(child, Some(element));
                }
            }
            CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
        }
    }

    fn deregister_element_internal(&mut self, element: &CrdtElement, count: &mut usize) {
        let created_at = element.created_at().to_id_string();
        sub_data_size(&mut self.doc_size.gc, element.data_size());
        self.element_pair_by_created_at.remove(&created_at);
        self.gc_element_set_by_created_at.remove(&created_at);
        *count += 1;

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.deregister_element_internal(child, count);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.deregister_element_internal(child, count);
                }
            }
            CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
        }
    }

    fn register_removed_descendants(&mut self, element: &CrdtElement) {
        if element.removed_at().is_some() {
            self.register_removed_element(element);
        }

        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_removed_descendants(child);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    self.register_removed_descendants(child);
                }
            }
            CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
        }
    }

    fn register_gc_pairs(&mut self, element: &CrdtElement) {
        match element {
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    self.register_gc_pairs(child);
                }
            }
            CrdtElement::Array(array) => {
                for node in array.iter_all_nodes() {
                    if node.element().is_none() && node.removed_at().is_some() {
                        self.register_gc_pair(node);
                    }
                    if let Some(child) = node.element() {
                        self.register_gc_pairs(child);
                    }
                }
            }
            CrdtElement::Text(text) => {
                for (child_id, child_size, removed_at) in text.gc_pair_entries() {
                    self.register_gc_pair_by_id(child_id, child_size, removed_at);
                }
            }
            CrdtElement::Primitive(_) => {}
        }
    }

    fn purge_gc_pair_by_child_id(&mut self, child_id: &str) -> bool {
        self.root_object.purge_text_gc_pair_by_id(child_id)
    }
}

fn sub_path_of(parent: &CrdtElement, created_at: &TimeTicket) -> Option<String> {
    match parent {
        CrdtElement::Object(object) => object.sub_path_of(created_at).map(ToOwned::to_owned),
        CrdtElement::Array(array) => array.sub_path_of(created_at),
        CrdtElement::Primitive(_) | CrdtElement::Text(_) => None,
    }
}

fn collect_descendant_ids(element: &CrdtElement, seen: &mut BTreeSet<String>) {
    match element {
        CrdtElement::Object(object) => {
            for (_, child) in object.iter_all() {
                seen.insert(child.created_at().to_id_string());
                collect_descendant_ids(child, seen);
            }
        }
        CrdtElement::Array(array) => {
            for child in array.iter_all() {
                seen.insert(child.created_at().to_id_string());
                collect_descendant_ids(child, seen);
            }
        }
        CrdtElement::Primitive(_) | CrdtElement::Text(_) => {}
    }
}

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

fn sub_data_size(target: &mut DataSize, size: DataSize) {
    target.data = target.data.saturating_sub(size.data);
    target.meta = target.meta.saturating_sub(size.meta);
}

#[cfg(test)]
mod tests {
    use super::CrdtRoot;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::text::CrdtText;
    use crate::{TimeTicket, VersionVector, TIME_TICKET_SIZE};
    use std::collections::BTreeMap;

    #[test]
    fn creates_root_with_initial_object() -> crate::Result<()> {
        let root = CrdtRoot::create();

        assert_eq!(1, root.get_element_map_size());
        assert!(root.find_by_created_at(&TimeTicket::initial()).is_some());
        assert_eq!("$", root.create_path(&TimeTicket::initial())?);
        assert_eq!("", root.create_path(&TimeTicket::max())?);
        assert_eq!("{}", root.to_json());
        assert_eq!("{}", root.to_sorted_json());
        Ok(())
    }

    #[test]
    fn registers_and_finds_object_members() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        assert_eq!(2, root.get_element_map_size());
        assert_eq!(Some(&member), root.find_by_created_at(&created_at));
        assert_eq!("$.k1", root.create_path(&created_at)?);
        assert_eq!(r#"{"k1":"k1"}"#, root.to_json());
        Ok(())
    }

    #[test]
    fn deregisters_object_members() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let removed_at = ticket(2, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        let removed = root.object_mut().delete_by_key("k1", removed_at).unwrap();
        assert_eq!(1, root.deregister_element(&removed));

        assert_eq!(1, root.get_element_map_size());
        assert!(root.find_by_created_at(&created_at).is_none());
    }

    #[test]
    fn registers_nested_object_descendants() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let profile_created_at = ticket(1, "a");
        let name_created_at = ticket(2, "a");
        let mut profile = CrdtObject::create(profile_created_at.clone());
        let name = primitive_str("yorkie", name_created_at.clone());

        profile.set("name", name.deepcopy(), name_created_at.clone());
        let profile = CrdtElement::object(profile);
        root.object_mut()
            .set("profile", profile.deepcopy(), profile_created_at.clone());
        let parent = root.root_element();
        root.register_element(&profile, Some(&parent));

        assert_eq!(3, root.get_element_map_size());
        assert_eq!("$.profile", root.create_path(&profile_created_at)?);
        assert_eq!("$.profile.name", root.create_path(&name_created_at)?);
        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, root.to_json());
        Ok(())
    }

    #[test]
    fn tracks_removed_elements_for_garbage_collection() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let removed_at = ticket(2, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        let removed = root.object_mut().delete_by_key("k1", removed_at).unwrap();
        root.register_removed_element(&removed);

        assert_eq!(1, root.get_garbage_element_set_size());
        assert_eq!(1, root.get_garbage_len());
        assert_eq!(
            removed.removed_at(),
            root.find_by_created_at(&created_at).unwrap().removed_at()
        );
        assert_eq!(2, root.stats().elements);
        assert_eq!(1, root.stats().gc_elements);
    }

    #[test]
    fn deepcopies_root_object_and_rebuilds_indexes() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        let copy = root.deepcopy();
        root.object_mut().delete_by_key("k1", ticket(2, "a"));

        assert_eq!(r#"{"k1":"k1"}"#, copy.to_json());
        assert_eq!("$.k1", copy.create_path(&created_at)?);
        assert_eq!(2, copy.get_element_map_size());
        Ok(())
    }

    #[test]
    fn tracks_array_dead_positions_for_garbage_collection() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let array_at = ticket(1, "a");
        let one_at = ticket(2, "a");
        let two_at = ticket(3, "a");
        let moved_at = ticket(4, "a");

        root.set_object_member(
            &TimeTicket::initial(),
            "items",
            CrdtElement::array(CrdtArray::create(array_at.clone())),
            array_at.clone(),
        )?;
        root.insert_array_element(
            &array_at,
            &TimeTicket::initial(),
            primitive_str("one", one_at.clone()),
            one_at.clone(),
        )?;
        root.insert_array_element(
            &array_at,
            &one_at,
            primitive_str("two", two_at.clone()),
            two_at.clone(),
        )?;

        root.move_array_element(&array_at, &two_at, &one_at, moved_at)?;

        assert_eq!(r#"{"items":["two","one"]}"#, root.to_json());
        assert_eq!(1, root.stats().gc_pairs);
        assert_eq!(1, root.get_garbage_len());

        let copy = root.deepcopy();
        assert_eq!(1, copy.stats().gc_pairs);
        assert_eq!(1, copy.get_garbage_len());
        assert_eq!("$.items.1", copy.create_path(&one_at)?);
        Ok(())
    }

    #[test]
    fn registers_text_members_and_text_gc_pairs() -> crate::Result<()> {
        let text_at = ticket(1, "a");
        let mut text = CrdtText::create(text_at.clone());
        text.edit_by_index(0, 0, "Hello World", None, ticket(2, "a"), None)?;

        let mut attrs = BTreeMap::new();
        attrs.insert("b".to_owned(), "true".to_owned());
        text.set_style_by_index(0, 5, attrs, ticket(3, "a"), None)?;
        text.remove_style_by_index(0, 5, &["b".to_owned()], ticket(4, "a"), None)?;
        text.edit_by_index(5, 11, "", None, ticket(5, "a"), None)?;

        assert_eq!(2, text.gc_pair_entries().len());

        let root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("message", CrdtElement::text(text))],
        ));

        assert_eq!(2, root.get_element_map_size());
        assert_eq!("$.message", root.create_path(&text_at)?);
        assert_eq!(r#"{"message":[{"val":"Hello"}]}"#, root.to_json());
        assert_eq!(
            "Hello",
            root.text_by_created_at(&text_at).unwrap().to_string()
        );
        assert_eq!(2, root.stats().gc_pairs);
        assert_eq!(2, root.get_garbage_len());

        let copy = root.deepcopy();
        assert_eq!(2, copy.stats().gc_pairs);
        assert_eq!(r#"{"message":[{"val":"Hello"}]}"#, copy.to_json());
        Ok(())
    }

    #[test]
    fn garbage_collects_text_internal_nodes() -> crate::Result<()> {
        let text_at = ticket(1, "a");
        let mut text = CrdtText::create(text_at.clone());
        text.edit_by_index(0, 0, "Hello World", None, ticket(2, "a"), None)?;
        text.edit_by_index(5, 10, "Yorkie", None, ticket(3, "a"), None)?;
        text.edit_by_index(0, 5, "", None, ticket(4, "a"), None)?;
        text.edit_by_index(6, 7, "", None, ticket(5, "a"), None)?;

        let mut root = CrdtRoot::new(CrdtObject::create_with_members(
            TimeTicket::initial(),
            [("message", CrdtElement::text(text))],
        ));

        assert_eq!(
            "Yorkie",
            root.text_by_created_at(&text_at).unwrap().to_string()
        );
        assert_eq!(3, root.get_garbage_len());

        let mut vector = VersionVector::new();
        vector.set("a", 5);
        assert_eq!(3, root.garbage_collect(&vector)?);

        assert_eq!(0, root.get_garbage_len());
        assert_eq!(r#"{"message":[{"val":"Yorkie"}]}"#, root.to_json());
        assert_eq!(
            r#"[0:00:0:0 {} ""][3:a:0:0 {} "Yorkie"]"#,
            root.text_by_created_at(&text_at).unwrap().to_test_string()
        );
        Ok(())
    }

    #[test]
    fn finds_mutable_text_members_in_the_live_tree() -> crate::Result<()> {
        let mut root = CrdtRoot::create();
        let text_at = ticket(1, "a");

        root.set_object_member(
            &TimeTicket::initial(),
            "message",
            CrdtElement::text(CrdtText::create(text_at.clone())),
            text_at.clone(),
        )?;

        root.text_by_created_at_mut(&text_at)
            .unwrap()
            .edit_by_index(0, 0, "Hi", None, ticket(2, "a"), None)?;

        assert_eq!(r#"{"message":[{"val":"Hi"}]}"#, root.to_json());
        assert_eq!("$.message", root.create_path(&text_at)?);
        Ok(())
    }

    #[test]
    fn reports_document_size_for_registered_elements() {
        let mut root = CrdtRoot::create();
        let created_at = ticket(1, "a");
        let member = primitive_str("k1", created_at.clone());

        root.object_mut()
            .set("k1", member.deepcopy(), created_at.clone());
        let parent = root.root_element();
        root.register_element(&member, Some(&parent));

        assert_eq!(TIME_TICKET_SIZE * 2, root.doc_size().live.meta);
        assert!(root.doc_size().live.data > 0);
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
