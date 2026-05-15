use super::element::{CrdtElementMeta, DataSize};
use super::rht::{Rht, RhtNode};
use crate::json::escape_json_string;
use crate::{JsonValue, Result, TimeTicket, YorkieError, TIME_TICKET_SIZE};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

const ELEMENT_PADDING_SIZE: usize = 2;
const DEFAULT_ROOT_TYPE: &str = "root";
const DEFAULT_TEXT_TYPE: &str = "text";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeNodeId {
    created_at: TimeTicket,
    offset: usize,
}

impl TreeNodeId {
    pub(crate) fn new(created_at: TimeTicket, offset: usize) -> Self {
        Self { created_at, offset }
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    pub(crate) fn split(&self, offset: usize) -> Self {
        Self::new(self.created_at.clone(), self.offset + offset)
    }

    pub(crate) fn has_same_created_at(&self, other: &Self) -> bool {
        self.created_at == other.created_at
    }

    pub(crate) fn to_id_string(&self) -> String {
        format!("{}:{}", self.created_at.to_id_string(), self.offset)
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!("{}/{}", self.created_at.to_test_string(), self.offset)
    }
}

impl Ord for TreeNodeId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.created_at
            .cmp(&other.created_at)
            .then_with(|| self.offset.cmp(&other.offset))
    }
}

impl PartialOrd for TreeNodeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreePos {
    parent_id: TreeNodeId,
    left_sibling_id: TreeNodeId,
}

impl TreePos {
    pub(crate) fn new(parent_id: TreeNodeId, left_sibling_id: TreeNodeId) -> Self {
        Self {
            parent_id,
            left_sibling_id,
        }
    }

    pub(crate) fn parent_id(&self) -> &TreeNodeId {
        &self.parent_id
    }

    pub(crate) fn left_sibling_id(&self) -> &TreeNodeId {
        &self.left_sibling_id
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{}:{}",
            self.parent_id.to_test_string(),
            self.left_sibling_id.to_test_string()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeNode {
    id: TreeNodeId,
    node_type: String,
    value: String,
    children: Vec<TreeNode>,
    attrs: Option<Rht>,
    removed_at: Option<TimeTicket>,
    ins_prev_id: Option<TreeNodeId>,
    ins_next_id: Option<TreeNodeId>,
    merged_from: Option<TreeNodeId>,
    merged_at: Option<TimeTicket>,
    merged_into: Option<TreeNodeId>,
}

impl TreeNode {
    pub(crate) fn new(
        id: TreeNodeId,
        node_type: impl Into<String>,
        attrs: Option<Rht>,
        value: impl Into<String>,
        children: Vec<TreeNode>,
    ) -> Self {
        Self {
            id,
            node_type: node_type.into(),
            value: value.into(),
            children,
            attrs,
            removed_at: None,
            ins_prev_id: None,
            ins_next_id: None,
            merged_from: None,
            merged_at: None,
            merged_into: None,
        }
    }

    pub(crate) fn create_element(
        id: TreeNodeId,
        node_type: impl Into<String>,
        attrs: Option<Rht>,
        children: Vec<TreeNode>,
    ) -> Self {
        Self::new(id, node_type, attrs, "", children)
    }

    pub(crate) fn create_text(id: TreeNodeId, value: impl Into<String>) -> Self {
        Self::new(id, DEFAULT_TEXT_TYPE, None, value, Vec::new())
    }

    pub(crate) fn create_root(id: TreeNodeId) -> Self {
        Self::create_element(id, DEFAULT_ROOT_TYPE, None, Vec::new())
    }

    pub(crate) fn id(&self) -> &TreeNodeId {
        &self.id
    }

    pub(crate) fn id_string(&self) -> String {
        self.id.to_id_string()
    }

    pub(crate) fn node_type(&self) -> &str {
        &self.node_type
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn children(&self) -> impl Iterator<Item = &TreeNode> {
        self.children.iter().filter(|child| !child.is_removed())
    }

    pub(crate) fn all_children(&self) -> impl Iterator<Item = &TreeNode> {
        self.children.iter()
    }

    pub(crate) fn all_children_mut(&mut self) -> impl Iterator<Item = &mut TreeNode> {
        self.children.iter_mut()
    }

    pub(crate) fn attrs(&self) -> Option<&Rht> {
        self.attrs.as_ref()
    }

    pub(crate) fn attrs_mut(&mut self) -> Option<&mut Rht> {
        self.attrs.as_mut()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.removed_at.as_ref()
    }

    pub(crate) fn ins_prev_id(&self) -> Option<&TreeNodeId> {
        self.ins_prev_id.as_ref()
    }

    pub(crate) fn ins_next_id(&self) -> Option<&TreeNodeId> {
        self.ins_next_id.as_ref()
    }

    pub(crate) fn merged_from(&self) -> Option<&TreeNodeId> {
        self.merged_from.as_ref()
    }

    pub(crate) fn merged_at(&self) -> Option<&TimeTicket> {
        self.merged_at.as_ref()
    }

    pub(crate) fn merged_into(&self) -> Option<&TreeNodeId> {
        self.merged_into.as_ref()
    }

    pub(crate) fn set_ins_prev_id(&mut self, id: Option<TreeNodeId>) {
        self.ins_prev_id = id;
    }

    pub(crate) fn set_ins_next_id(&mut self, id: Option<TreeNodeId>) {
        self.ins_next_id = id;
    }

    pub(crate) fn set_merged_from(&mut self, id: Option<TreeNodeId>) {
        self.merged_from = id;
    }

    pub(crate) fn set_merged_at(&mut self, ticket: Option<TimeTicket>) {
        self.merged_at = ticket;
    }

    pub(crate) fn set_merged_into(&mut self, id: Option<TreeNodeId>) {
        self.merged_into = id;
    }

    pub(crate) fn is_text(&self) -> bool {
        self.node_type == DEFAULT_TEXT_TYPE
    }

    pub(crate) fn has_text_child(&self) -> bool {
        self.children().any(TreeNode::is_text)
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    pub(crate) fn can_style(&self, edited_at: &TimeTicket, client_lamport_at_change: i64) -> bool {
        if self.is_text() {
            return false;
        }

        let node_existed = self.id.created_at().lamport() <= client_lamport_at_change;
        node_existed
            && self
                .removed_at
                .as_ref()
                .map(|removed_at| edited_at.after(removed_at))
                .unwrap_or(true)
    }

    pub(crate) fn len(&self) -> usize {
        if self.is_text() {
            return utf16_len(&self.value);
        }

        self.children().map(|child| child.padded_size(false)).sum()
    }

    pub(crate) fn total_len(&self) -> usize {
        if self.is_text() {
            return utf16_len(&self.value);
        }

        self.all_children()
            .map(|child| child.padded_size(true))
            .sum()
    }

    pub(crate) fn padded_size(&self, include_removed: bool) -> usize {
        let mut size = if include_removed {
            self.total_len()
        } else {
            self.len()
        };

        if !self.is_text() {
            size += ELEMENT_PADDING_SIZE;
        }

        size
    }

    pub(crate) fn append(&mut self, node: TreeNode) {
        self.children.push(node);
    }

    pub(crate) fn insert_at(&mut self, index: usize, node: TreeNode) {
        self.children.insert(index, node);
    }

    pub(crate) fn insert_visible_at(
        &mut self,
        visible_offset: usize,
        node: TreeNode,
    ) -> Result<()> {
        let physical_offset = self.physical_child_offset(visible_offset)?;
        self.children.insert(physical_offset, node);
        Ok(())
    }

    pub(crate) fn split_element_at(
        &mut self,
        visible_offset: usize,
        id: TreeNodeId,
    ) -> Result<(TreeNode, DataSize)> {
        if self.is_text() {
            return Err(YorkieError::InvalidTreePosition(
                "text node cannot be split as element".to_owned(),
            ));
        }

        let physical_offset = self.physical_child_offset(visible_offset)?;
        let prev_size = self.data_size();
        let right_children = self.children.split_off(physical_offset);
        let mut right = TreeNode::create_element(
            id,
            self.node_type.clone(),
            self.attrs.clone(),
            right_children,
        );
        right.removed_at = self.removed_at.clone();
        right.ins_prev_id = Some(self.id.clone());
        right.ins_next_id = self.ins_next_id.clone();
        self.ins_next_id = Some(right.id.clone());

        let left_size = self.data_size();
        let right_size = right.data_size();
        Ok((
            right,
            DataSize {
                data: left_size.data + right_size.data - prev_size.data,
                meta: left_size.meta + right_size.meta - prev_size.meta,
            },
        ))
    }

    pub(crate) fn split_text_child_at(
        &mut self,
        child_index: usize,
        absolute_offset: usize,
    ) -> Result<DataSize> {
        let Some(child) = self.children.get(child_index) else {
            return Err(YorkieError::InvalidTreePosition(
                "split child not found".to_owned(),
            ));
        };
        if !child.is_text() {
            return Ok(DataSize::default());
        }

        let child_start = child.id().offset();
        let child_end = child_start + child.len();
        if absolute_offset <= child_start || absolute_offset >= child_end {
            return Ok(DataSize::default());
        }

        let relative_offset = absolute_offset - child_start;
        let prev_size = child.data_size();
        let right_id = child.id().split(relative_offset);
        let right_removed_at = child.removed_at().cloned();
        let right_ins_next_id = child.ins_next_id().cloned();
        let right_merged_from = child.merged_from().cloned();
        let right_merged_at = child.merged_at().cloned();
        let (left_value, right_value) = split_utf16(child.value(), relative_offset);
        if right_value.is_empty() {
            return Ok(DataSize::default());
        }

        let child = self
            .children
            .get_mut(child_index)
            .expect("child existence checked above");
        child.value = left_value;
        child.ins_next_id = Some(right_id.clone());

        let mut right = TreeNode::create_text(right_id, right_value);
        right.removed_at = right_removed_at;
        right.ins_prev_id = Some(child.id().clone());
        right.ins_next_id = right_ins_next_id;
        right.merged_from = right_merged_from;
        right.merged_at = right_merged_at;

        let left_size = child.data_size();
        let right_size = right.data_size();
        self.children.insert(child_index + 1, right);

        Ok(DataSize {
            data: left_size.data + right_size.data - prev_size.data,
            meta: left_size.meta + right_size.meta - prev_size.meta,
        })
    }

    fn physical_child_offset(&self, visible_offset: usize) -> Result<usize> {
        let visible_count = self.children().count();
        if visible_offset > visible_count {
            return Err(YorkieError::InvalidTreePosition(
                "child offset is out of range".to_owned(),
            ));
        }

        if visible_offset == visible_count {
            return Ok(self.children.len());
        }

        self.children
            .iter()
            .enumerate()
            .filter(|(_, child)| !child.is_removed())
            .nth(visible_offset)
            .map(|(index, _)| index)
            .ok_or_else(|| {
                YorkieError::InvalidTreePosition("child offset is out of range".to_owned())
            })
    }

    pub(crate) fn remove(&mut self, removed_at: TimeTicket) -> bool {
        if self.removed_at.is_none() {
            self.removed_at = Some(removed_at);
            return true;
        }

        if self
            .removed_at
            .as_ref()
            .is_some_and(|current| removed_at.after(current))
        {
            self.removed_at = Some(removed_at);
        }
        false
    }

    pub(crate) fn clear_removed_recursively(&mut self) {
        self.removed_at = None;
        for child in self.all_children_mut() {
            child.clear_removed_recursively();
        }
    }

    pub(crate) fn remove_recursively(
        &mut self,
        removed_at: TimeTicket,
        entries: &mut Vec<(String, DataSize, TimeTicket)>,
        removed_nodes: &mut Vec<TreeNode>,
    ) {
        if self.remove(removed_at.clone()) {
            entries.push((self.id_string(), self.data_size(), removed_at.clone()));
            removed_nodes.push(self.clone());
        }

        for child in self.all_children_mut() {
            child.remove_recursively(removed_at.clone(), entries, removed_nodes);
        }
    }

    pub(crate) fn set_attr(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        edited_at: TimeTicket,
    ) -> (Option<RhtNode>, Option<RhtNode>) {
        self.attrs
            .get_or_insert_with(Rht::new)
            .set(key, value, edited_at)
    }

    pub(crate) fn remove_attr(&mut self, key: &str, edited_at: TimeTicket) -> Vec<RhtNode> {
        self.attrs
            .get_or_insert_with(Rht::new)
            .remove(key, edited_at)
    }

    pub(crate) fn data_size(&self) -> DataSize {
        let mut size = DataSize {
            data: if self.is_text() {
                utf16_len(&self.value) * 2
            } else {
                0
            },
            meta: TIME_TICKET_SIZE,
        };

        if self.removed_at.is_some() {
            size.meta += TIME_TICKET_SIZE;
        }

        if let Some(attrs) = &self.attrs {
            for node in attrs.iter().filter(|node| node.removed_at().is_none()) {
                let node_size = node.data_size();
                size.data += node_size.data;
                size.meta += node_size.meta;
            }
        }

        size
    }

    pub(crate) fn gc_pair_entries(&self) -> Vec<(String, DataSize, TimeTicket)> {
        let mut entries = Vec::new();
        if let Some(attrs) = &self.attrs {
            for node in attrs.iter().filter(|node| node.removed_at().is_some()) {
                entries.push((
                    node.id_string(),
                    node.data_size(),
                    node.removed_at().unwrap().clone(),
                ));
            }
        }

        for child in self.all_children() {
            if let Some(removed_at) = child.removed_at() {
                entries.push((child.id_string(), child.data_size(), removed_at.clone()));
            }
            entries.extend(child.gc_pair_entries());
        }

        entries
    }

    pub(crate) fn purge_gc_pair_by_id(&mut self, child_id: &str) -> bool {
        if let Some(attrs) = &mut self.attrs {
            if attrs.purge_by_id(child_id) {
                return true;
            }
        }

        if let Some(index) = self
            .children
            .iter()
            .position(|child| child.id_string() == child_id)
        {
            self.children.remove(index);
            return true;
        }

        for child in self.all_children_mut() {
            if child.purge_gc_pair_by_id(child_id) {
                return true;
            }
        }

        false
    }

    pub(crate) fn to_json(&self) -> String {
        if self.is_text() {
            return format!(
                "{{\"type\":\"{}\",\"value\":\"{}\"}}",
                escape_json_string(&self.node_type),
                escape_json_string(&self.value)
            );
        }

        let children = self
            .children()
            .map(TreeNode::to_json)
            .collect::<Vec<_>>()
            .join(",");

        let mut json = format!(
            "{{\"type\":\"{}\",\"children\":[{}]",
            escape_json_string(&self.node_type),
            children
        );

        if let Some(attrs) = &self.attrs {
            if !attrs.is_empty() {
                json.push_str(",\"attributes\":");
                json.push_str(&attributes_to_json(attrs));
            }
        }

        json.push('}');
        json
    }

    pub(crate) fn to_xml(&self) -> String {
        if self.is_text() {
            return escape_xml_text(&self.value);
        }

        let attrs = self.attributes_to_xml();
        let children = self
            .children()
            .map(TreeNode::to_xml)
            .collect::<Vec<_>>()
            .join("");

        format!(
            "<{}{}>{}</{}>",
            self.node_type, attrs, children, self.node_type
        )
    }

    pub(crate) fn to_test_string(&self) -> String {
        if self.is_text() {
            return format!(
                "{{type:{},value:{},size:{},removed:{}}}",
                self.node_type,
                escape_json_string(&self.value),
                self.len(),
                self.is_removed()
            );
        }

        let children = self
            .children()
            .map(TreeNode::to_test_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{type:{},children:[{}],size:{},removed:{}}}",
            self.node_type,
            children,
            self.len(),
            self.is_removed()
        )
    }

    fn attributes_to_xml(&self) -> String {
        let Some(attrs) = &self.attrs else {
            return String::new();
        };

        let attrs = attrs
            .iter()
            .filter(|node| node.removed_at().is_none())
            .map(|node| {
                format!(
                    "{}=\"{}\"",
                    escape_xml_attribute(node.key()),
                    escape_xml_attribute(&attribute_value_to_xml_value(node.value()))
                )
            })
            .collect::<Vec<_>>();

        if attrs.is_empty() {
            return String::new();
        }

        format!(" {}", attrs.join(" "))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtTree {
    meta: CrdtElementMeta,
    root: TreeNode,
    node_by_id: BTreeMap<TreeNodeId, TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeStyleChange {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) from_path: Vec<usize>,
    pub(crate) to_path: Vec<usize>,
    pub(crate) attributes: BTreeMap<String, String>,
    pub(crate) attributes_to_remove: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeEditChange {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) from_path: Vec<usize>,
    pub(crate) to_path: Vec<usize>,
    pub(crate) value: Option<Vec<String>>,
    pub(crate) split_level: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeEditResult {
    pub(crate) changes: Vec<TreeEditChange>,
    pub(crate) gc_pairs: Vec<(String, DataSize, TimeTicket)>,
    pub(crate) diff: DataSize,
    pub(crate) removed_nodes: Vec<TreeNode>,
    pub(crate) from_idx: usize,
    pub(crate) to_idx: usize,
    pub(crate) inserted_size: usize,
}

impl CrdtTree {
    pub(crate) fn new(root: TreeNode, created_at: TimeTicket) -> Self {
        let mut tree = Self {
            meta: CrdtElementMeta::new(created_at),
            root,
            node_by_id: BTreeMap::new(),
        };
        tree.rebuild_node_map();
        tree.rebuild_merge_state();
        tree
    }

    pub(crate) fn create(root: TreeNode, created_at: TimeTicket) -> Self {
        Self::new(root, created_at)
    }

    pub(crate) fn create_empty(created_at: TimeTicket, root_created_at: TimeTicket) -> Self {
        Self::new(
            TreeNode::create_root(TreeNodeId::new(root_created_at, 0)),
            created_at,
        )
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

    pub(crate) fn root(&self) -> &TreeNode {
        &self.root
    }

    pub(crate) fn root_mut(&mut self) -> &mut TreeNode {
        &mut self.root
    }

    pub(crate) fn find_floor_node(&self, id: &TreeNodeId) -> Option<&TreeNode> {
        let (key, node) = self.node_by_id.range(..=id.clone()).next_back()?;
        key.has_same_created_at(id).then_some(node)
    }

    pub(crate) fn node_map_len(&self) -> usize {
        self.node_by_id.len()
    }

    pub(crate) fn find_pos(&self, index: usize, prefer_text: bool) -> Result<TreePos> {
        let tree_pos = find_tree_pos(&self.root, Vec::new(), index, prefer_text)?;
        tree_pos_to_crdt_pos(&self.root, &tree_pos)
    }

    pub(crate) fn index_to_path(&self, index: usize) -> Result<Vec<usize>> {
        let tree_pos = find_tree_pos(&self.root, Vec::new(), index, true)?;
        tree_pos_to_path(&self.root, &tree_pos)
    }

    pub(crate) fn path_to_index(&self, path: &[usize]) -> Result<usize> {
        let tree_pos = path_to_tree_pos(&self.root, path)?;
        index_of_tree_pos(&self.root, &tree_pos, false)
    }

    pub(crate) fn path_to_pos(&self, path: &[usize]) -> Result<TreePos> {
        let tree_pos = path_to_tree_pos(&self.root, path)?;
        tree_pos_to_crdt_pos(&self.root, &tree_pos)
    }

    pub(crate) fn path_to_pos_range(&self, path: &[usize]) -> Result<(TreePos, TreePos)> {
        let from_index = self.path_to_index(path)?;
        Ok((
            self.find_pos(from_index, true)?,
            self.find_pos(from_index + 1, true)?,
        ))
    }

    pub(crate) fn style_by_range_with_changes(
        &mut self,
        range: (TreePos, TreePos),
        attributes: BTreeMap<String, String>,
        edited_at: TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<(
        Vec<RhtNode>,
        DataSize,
        Vec<TreeStyleChange>,
        BTreeMap<String, String>,
        Vec<String>,
    )> {
        let mut diff = self.split_text_range(&range)?;
        let from_idx = self.pos_to_index(&range.0)?;
        let to_idx = self.pos_to_index(&range.1)?;
        let targets = self.style_target_paths(from_idx, to_idx, &edited_at, version_vector);

        let mut gc_nodes = Vec::new();
        let mut changes = Vec::new();
        let mut previous_attributes = BTreeMap::new();
        let mut attributes_to_remove = Vec::new();
        let mut captured_previous = false;

        for target in targets {
            if target.kind == TreeTokenKind::End
                && self.has_unknown_split_sibling(&target.path, version_vector)?
            {
                continue;
            }

            let mut apply_targets = vec![(target.path.clone(), target.from, target.to, true)];
            if target.kind == TreeTokenKind::Start {
                for sibling_path in
                    self.unknown_split_sibling_paths(&target.path, version_vector)?
                {
                    if let Some(visible_path) = physical_to_visible_path(&self.root, &sibling_path)
                    {
                        let from = self.path_to_index(&visible_path)?;
                        apply_targets.push((sibling_path, from, from + 1, false));
                    }
                }
            }

            for (path, from, to, is_primary) in apply_targets {
                let node = node_at_path_mut(&mut self.root, &path)?;
                if is_primary
                    && !node.can_style(
                        &edited_at,
                        client_lamport(version_vector, node.id().created_at()),
                    )
                {
                    continue;
                }
                if !is_primary && node.is_removed() {
                    continue;
                }

                if is_primary && !captured_previous {
                    for key in attributes.keys() {
                        if let Some(attrs) = node.attrs() {
                            if attrs.has(key) {
                                if let Some(value) = attrs.get(key) {
                                    previous_attributes.insert(key.clone(), value.to_owned());
                                }
                                continue;
                            }
                        }
                        attributes_to_remove.push(key.clone());
                    }
                    captured_previous = true;
                }

                let mut affected_attrs = BTreeMap::new();
                for (key, value) in &attributes {
                    let (prev, curr) = node.set_attr(key.clone(), value.clone(), edited_at.clone());
                    if let Some(prev) = prev {
                        gc_nodes.push(prev);
                    }
                    if let Some(curr) = curr {
                        affected_attrs.insert(key.clone(), value.clone());
                        if target.kind != TreeTokenKind::End || !is_primary {
                            add_data_size(&mut diff, curr.data_size());
                        }
                    }
                }

                if !affected_attrs.is_empty() {
                    changes.push(TreeStyleChange {
                        from,
                        to,
                        from_path: self.index_to_path(from)?,
                        to_path: self.index_to_path(to)?,
                        attributes: affected_attrs,
                        attributes_to_remove: Vec::new(),
                    });
                }
            }
        }

        self.rebuild_node_map();

        Ok((
            gc_nodes,
            diff,
            changes,
            previous_attributes,
            attributes_to_remove,
        ))
    }

    pub(crate) fn remove_style_by_range_with_changes(
        &mut self,
        range: (TreePos, TreePos),
        attributes_to_remove: &[String],
        edited_at: TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<(
        Vec<RhtNode>,
        DataSize,
        Vec<TreeStyleChange>,
        BTreeMap<String, String>,
    )> {
        let mut diff = self.split_text_range(&range)?;
        let from_idx = self.pos_to_index(&range.0)?;
        let to_idx = self.pos_to_index(&range.1)?;
        let targets = self.style_target_paths(from_idx, to_idx, &edited_at, version_vector);

        let mut gc_nodes = Vec::new();
        let mut changes = Vec::new();
        let mut previous_attributes = BTreeMap::new();
        let mut captured_previous = false;

        for target in targets {
            if target.kind == TreeTokenKind::End
                && self.has_unknown_split_sibling(&target.path, version_vector)?
            {
                continue;
            }

            let mut apply_targets = vec![(target.path.clone(), target.from, target.to, true)];
            if target.kind == TreeTokenKind::Start {
                for sibling_path in
                    self.unknown_split_sibling_paths(&target.path, version_vector)?
                {
                    if let Some(visible_path) = physical_to_visible_path(&self.root, &sibling_path)
                    {
                        let from = self.path_to_index(&visible_path)?;
                        apply_targets.push((sibling_path, from, from + 1, false));
                    }
                }
            }

            for (path, from, to, is_primary) in apply_targets {
                let node = node_at_path_mut(&mut self.root, &path)?;
                if is_primary
                    && !node.can_style(
                        &edited_at,
                        client_lamport(version_vector, node.id().created_at()),
                    )
                {
                    continue;
                }
                if !is_primary && node.is_removed() {
                    continue;
                }

                if is_primary && !captured_previous {
                    for key in attributes_to_remove {
                        if let Some(attrs) = node.attrs() {
                            if attrs.has(key) {
                                if let Some(value) = attrs.get(key) {
                                    previous_attributes.insert(key.clone(), value.to_owned());
                                }
                            }
                        }
                    }
                    captured_previous = true;
                }

                let mut removed_any = false;
                for key in attributes_to_remove {
                    for removed in node.remove_attr(key, edited_at.clone()) {
                        add_data_size(&mut diff, removed.data_size());
                        gc_nodes.push(removed);
                        removed_any = true;
                    }
                }

                if removed_any || !attributes_to_remove.is_empty() {
                    changes.push(TreeStyleChange {
                        from,
                        to,
                        from_path: self.index_to_path(from)?,
                        to_path: self.index_to_path(to)?,
                        attributes: BTreeMap::new(),
                        attributes_to_remove: attributes_to_remove.to_vec(),
                    });
                }
            }
        }

        self.rebuild_node_map();

        Ok((gc_nodes, diff, changes, previous_attributes))
    }

    pub(crate) fn edit_by_range_with_changes(
        &mut self,
        range: (TreePos, TreePos),
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
        edited_at: TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<TreeEditResult> {
        let from_idx = self.pos_to_index(&range.0)?;
        let to_idx = self.pos_to_index(&range.1)?;
        let from_path = self.index_to_path(from_idx)?;
        let to_path = self.index_to_path(to_idx)?;
        let mut diff = DataSize::default();
        add_data_size(&mut diff, self.split_text_at_position(&range.0)?);
        add_data_size(&mut diff, self.split_text_at_position(&range.1)?);
        self.rebuild_node_map();
        let mut gc_pairs = Vec::new();
        let mut removed_nodes = Vec::new();
        let from_parent_path = self
            .physical_path_by_node_id(range.0.parent_id())
            .ok_or_else(|| YorkieError::InvalidTreePosition("parent not found".to_owned()))?;

        if from_idx < to_idx {
            let targets = self.edit_targets(from_idx, to_idx, &edited_at, version_vector);
            for path in targets.remove_paths.iter().rev() {
                let node = node_at_path_mut(&mut self.root, path)?;
                if node.remove(edited_at.clone()) {
                    gc_pairs.push((node.id_string(), node.data_size(), edited_at.clone()));
                    removed_nodes.push(node.clone());
                }
            }
            self.merge_nodes(&from_parent_path, &targets.merge_paths, &edited_at)?;
        }

        if split_level > 0 {
            let contents_len = contents.as_ref().map(Vec::len).unwrap_or_default();
            add_data_size(
                &mut diff,
                self.split_elements_at_position(&range.0, split_level, &edited_at, contents_len)?,
            );
        }

        let mut inserted_size = 0;
        let inserted_value = contents
            .as_ref()
            .map(|nodes| nodes.iter().map(TreeNode::to_json).collect::<Vec<_>>());

        if let Some(contents) = contents {
            let (parent_path, offset) = self.position_to_parent_offset(&range.0)?;
            let parent = node_at_visible_path_mut(&mut self.root, &parent_path)?;
            let mut insert_offset = offset;
            for mut content in contents {
                content.clear_removed_recursively();
                inserted_size += content.padded_size(false);
                add_data_size(&mut diff, subtree_data_size(&content));
                parent.insert_visible_at(insert_offset, content)?;
                insert_offset += 1;
            }
        }

        self.rebuild_node_map();
        self.rebuild_merge_state();

        Ok(TreeEditResult {
            changes: vec![TreeEditChange {
                from: from_idx,
                to: to_idx,
                from_path,
                to_path,
                value: inserted_value,
                split_level: (split_level > 0).then_some(split_level),
            }],
            gc_pairs,
            diff,
            removed_nodes,
            from_idx,
            to_idx,
            inserted_size,
        })
    }

    pub(crate) fn data_size(&self) -> DataSize {
        let mut size = DataSize {
            data: 0,
            meta: self.meta_usage(),
        };

        for node in self.nodes().filter(|node| !node.is_removed()) {
            let node_size = node.data_size();
            size.data += node_size.data;
            size.meta += node_size.meta;
        }

        size
    }

    pub(crate) fn to_json(&self) -> String {
        self.root.to_json()
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.to_json()
    }

    pub(crate) fn to_xml(&self) -> String {
        self.root.to_xml()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        let mut tree = Self {
            meta: self.meta.clone(),
            root: self.root.clone(),
            node_by_id: BTreeMap::new(),
        };
        tree.rebuild_node_map();
        tree.rebuild_merge_state();
        tree
    }

    pub(crate) fn gc_pair_entries(&self) -> Vec<(String, DataSize, TimeTicket)> {
        self.root.gc_pair_entries()
    }

    pub(crate) fn purge_gc_pair_by_id(&mut self, child_id: &str) -> bool {
        let purged = self.root.purge_gc_pair_by_id(child_id);
        if purged {
            self.rebuild_node_map();
            self.rebuild_merge_state();
        }
        purged
    }

    pub(crate) fn nodes(&self) -> impl Iterator<Item = &TreeNode> {
        let mut nodes = Vec::new();
        collect_nodes(&self.root, &mut nodes);
        nodes.into_iter()
    }

    fn pos_to_index(&self, pos: &TreePos) -> Result<usize> {
        for index in 0..=self.root.len() {
            if self.find_pos(index, true)? == *pos {
                return Ok(index);
            }
        }

        Err(YorkieError::InvalidTreePosition(
            "position is not visible in the current tree".to_owned(),
        ))
    }

    fn style_target_paths(
        &self,
        from_idx: usize,
        to_idx: usize,
        edited_at: &TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Vec<StyleTarget> {
        let mut targets = Vec::new();
        let mut seen = BTreeSet::new();
        for token in collect_tokens_between(&self.root, from_idx, to_idx, false) {
            if token.kind == TreeTokenKind::Text {
                continue;
            }
            let Ok(node) = node_at_path(&self.root, &token.path) else {
                continue;
            };
            let key = node.id_string();
            if node.can_style(
                edited_at,
                client_lamport(version_vector, node.id().created_at()),
            ) && seen.insert(key)
            {
                targets.push(StyleTarget {
                    path: token.path,
                    from: token.index,
                    to: token.index + 1,
                    kind: token.kind,
                });
            }
        }
        targets
    }

    fn edit_targets(
        &self,
        from_idx: usize,
        to_idx: usize,
        edited_at: &TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> EditTargets {
        let mut targets = EditTargets::default();
        let mut removed = BTreeSet::new();
        let mut merged = BTreeSet::new();

        for token in collect_tokens_between(&self.root, from_idx, to_idx, true) {
            let Ok(node) = node_at_path(&self.root, &token.path) else {
                continue;
            };

            if token.kind == TreeTokenKind::Start
                && !token.ended
                && ticket_known(version_vector, node.id().created_at())
            {
                targets.merge_paths.push(token.path.clone());
                merged.insert(token.path.clone());
            }

            let parent_removed = split_parent_path(&token.path)
                .map(|(parent_path, _)| {
                    removed.contains(&parent_path) && !merged.contains(&parent_path)
                })
                .unwrap_or(false);

            if node_can_delete(node, edited_at, version_vector) || parent_removed {
                if matches!(token.kind, TreeTokenKind::Text | TreeTokenKind::Start)
                    && removed.insert(token.path.clone())
                {
                    targets.remove_paths.push(token.path.clone());

                    if !node.is_text()
                        && node.ins_next_id().is_some()
                        && !merged.contains(&token.path)
                    {
                        for split_path in self
                            .unknown_split_sibling_paths(&token.path, version_vector)
                            .unwrap_or_default()
                        {
                            if removed.insert(split_path.clone()) {
                                targets.remove_paths.push(split_path.clone());
                            }
                            for descendant_path in descendant_paths(&self.root, &split_path) {
                                if removed.insert(descendant_path.clone()) {
                                    targets.remove_paths.push(descendant_path);
                                }
                            }
                        }
                    }
                }
            }
        }

        targets
    }

    fn split_text_range(&mut self, range: &(TreePos, TreePos)) -> Result<DataSize> {
        let mut diff = DataSize::default();
        add_data_size(&mut diff, self.split_text_at_position(&range.0)?);
        add_data_size(&mut diff, self.split_text_at_position(&range.1)?);
        self.rebuild_node_map();
        Ok(diff)
    }

    fn split_text_at_position(&mut self, pos: &TreePos) -> Result<DataSize> {
        let parent_path = self
            .path_by_node_id(pos.parent_id())
            .ok_or_else(|| YorkieError::InvalidTreePosition("parent not found".to_owned()))?;
        let parent = node_at_visible_path_mut(&mut self.root, &parent_path)?;
        let split_offset = pos.left_sibling_id().offset();

        let Some(child_index) = parent.children.iter().position(|child| {
            !child.is_removed()
                && child.is_text()
                && pos.left_sibling_id().has_same_created_at(child.id())
                && child.id().offset() < split_offset
                && split_offset < child.id().offset() + child.len()
        }) else {
            return Ok(DataSize::default());
        };

        parent.split_text_child_at(child_index, split_offset)
    }

    fn split_elements_at_position(
        &mut self,
        pos: &TreePos,
        split_level: usize,
        edited_at: &TimeTicket,
        contents_len: usize,
    ) -> Result<DataSize> {
        let (mut parent_path, mut offset) = self.position_to_parent_offset(pos)?;
        let mut diff = DataSize::default();

        for split_index in 0..split_level {
            if parent_path.is_empty() {
                return Err(YorkieError::InvalidTreePosition(
                    "root node cannot be split".to_owned(),
                ));
            }

            let Some((grand_parent_path, visible_index)) = split_parent_path(&parent_path) else {
                return Err(YorkieError::InvalidTreePosition(
                    "root node cannot be split".to_owned(),
                ));
            };
            let split_id =
                TreeNodeId::new(split_issue_ticket(edited_at, contents_len, split_index), 0);
            let parent = node_at_visible_path_mut(&mut self.root, &grand_parent_path)?;
            let physical_index = parent.physical_child_offset(visible_index)?;
            let node = parent.children.get_mut(physical_index).ok_or_else(|| {
                YorkieError::InvalidTreePosition("split element not found".to_owned())
            })?;
            let (right, split_diff) = node.split_element_at(offset, split_id)?;
            add_data_size(&mut diff, split_diff);
            parent.children.insert(physical_index + 1, right);

            offset = visible_index + 1;
            parent_path = grand_parent_path;
        }

        Ok(diff)
    }

    fn merge_nodes(
        &mut self,
        from_parent_path: &[usize],
        merge_paths: &[Vec<usize>],
        edited_at: &TimeTicket,
    ) -> Result<()> {
        if merge_paths.is_empty() {
            return Ok(());
        }

        let from_parent_id = node_at_path(&self.root, from_parent_path)?.id().clone();
        for merge_path in merge_paths {
            if merge_path == from_parent_path {
                continue;
            }

            let moved_children = {
                let source = node_at_path_mut(&mut self.root, merge_path)?;
                let source_id = source.id().clone();
                source.set_merged_into(Some(from_parent_id.clone()));

                let mut moved = Vec::new();
                let mut remaining = Vec::new();
                for mut child in source.children.drain(..) {
                    if child.is_removed() {
                        remaining.push(child);
                    } else {
                        child.set_merged_from(Some(source_id.clone()));
                        child.set_merged_at(Some(edited_at.clone()));
                        moved.push(child);
                    }
                }
                source.children = remaining;
                moved
            };

            if !moved_children.is_empty() {
                let target = node_at_path_mut(&mut self.root, from_parent_path)?;
                target.children.extend(moved_children);
            }
        }

        Ok(())
    }

    fn position_to_parent_offset(&self, pos: &TreePos) -> Result<(Vec<usize>, usize)> {
        let parent_path = self
            .path_by_node_id(pos.parent_id())
            .ok_or_else(|| YorkieError::InvalidTreePosition("parent not found".to_owned()))?;
        let parent = node_at_visible_path(&self.root, &parent_path)?;

        if pos.left_sibling_id().has_same_created_at(parent.id())
            && pos.left_sibling_id().offset() == 0
        {
            return Ok((parent_path, 0));
        }

        for (offset, child) in parent.children().enumerate() {
            if pos.left_sibling_id().has_same_created_at(child.id()) {
                if child.is_text() {
                    let child_start = child.id().offset();
                    let child_end = child_start + child.len();
                    if pos.left_sibling_id().offset() == child_start {
                        return Ok((parent_path, offset));
                    }
                    if pos.left_sibling_id().offset() == child_end {
                        return Ok((parent_path, offset + 1));
                    }
                    continue;
                }
                return Ok((parent_path, offset + 1));
            }
        }

        Err(YorkieError::InvalidTreePosition(
            "left sibling not found".to_owned(),
        ))
    }

    fn path_by_node_id(&self, id: &TreeNodeId) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        find_visible_path_by_node_id(&self.root, id, &mut Vec::new(), &mut path).then_some(path)
    }

    fn physical_path_by_node_id(&self, id: &TreeNodeId) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        find_path_by_node_id(&self.root, id, &mut Vec::new(), &mut path).then_some(path)
    }

    fn floor_node_path(&self, id: &TreeNodeId) -> Option<Vec<usize>> {
        let (key, _) = self.node_by_id.range(..=id.clone()).next_back()?;
        if !key.has_same_created_at(id) {
            return None;
        }

        self.physical_path_by_node_id(key)
    }

    fn advance_past_unknown_split_siblings(
        &self,
        path: &[usize],
        version_vector: Option<&crate::VersionVector>,
        relax_parent_check: bool,
        skip_actor_id: Option<&str>,
    ) -> Result<Vec<usize>> {
        let Some(version_vector) = version_vector else {
            return Ok(path.to_vec());
        };

        let mut current_path = path.to_vec();
        loop {
            let current = node_at_path(&self.root, &current_path)?;
            let Some(ins_next_id) = current.ins_next_id().cloned() else {
                break;
            };
            let Some(next_path) = self.floor_node_path(&ins_next_id) else {
                break;
            };
            let next = node_at_path(&self.root, &next_path)?;
            if next.is_text() {
                break;
            }

            if !relax_parent_check
                && split_parent_path(&next_path).map(|(path, _)| path)
                    != split_parent_path(&current_path).map(|(path, _)| path)
            {
                break;
            }

            let actor_id = next.id().created_at().actor_id().as_str();
            if skip_actor_id.is_some_and(|skip| skip == actor_id) {
                break;
            }

            if version_vector.after_or_equal(next.id().created_at()) {
                break;
            }

            current_path = next_path;
        }

        Ok(current_path)
    }

    fn has_unknown_split_sibling(
        &self,
        path: &[usize],
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<bool> {
        let Some(version_vector) = version_vector else {
            return Ok(false);
        };
        let node = node_at_path(&self.root, path)?;
        let Some(ins_next_id) = node.ins_next_id() else {
            return Ok(false);
        };
        let Some(next_path) = self.floor_node_path(ins_next_id) else {
            return Ok(false);
        };
        let next = node_at_path(&self.root, &next_path)?;
        if next.is_text() {
            return Ok(false);
        }

        Ok(!version_vector.after_or_equal(next.id().created_at()))
    }

    fn unknown_split_sibling_paths(
        &self,
        path: &[usize],
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<Vec<Vec<usize>>> {
        let Some(version_vector) = version_vector else {
            return Ok(Vec::new());
        };

        let mut paths = Vec::new();
        let mut current_path = path.to_vec();
        loop {
            let current = node_at_path(&self.root, &current_path)?;
            let Some(ins_next_id) = current.ins_next_id() else {
                break;
            };
            let Some(next_path) = self.floor_node_path(ins_next_id) else {
                break;
            };
            let next = node_at_path(&self.root, &next_path)?;
            if next.is_text() || version_vector.after_or_equal(next.id().created_at()) {
                break;
            }
            paths.push(next_path.clone());
            current_path = next_path;
        }

        Ok(paths)
    }

    fn rebuild_node_map(&mut self) {
        let mut map = BTreeMap::new();
        collect_node_map(&self.root, &mut map);
        self.node_by_id = map;
    }

    fn rebuild_merge_state(&mut self) {
        let mut merged_pairs = Vec::new();
        collect_merge_pairs(&self.root, &mut merged_pairs);
        if merged_pairs.is_empty() {
            return;
        }

        rebuild_merge_state_in_node(&mut self.root, &merged_pairs);
        self.rebuild_node_map();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleTarget {
    path: Vec<usize>,
    from: usize,
    to: usize,
    kind: TreeTokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeTokenKind {
    Start,
    End,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeTokenTarget {
    path: Vec<usize>,
    kind: TreeTokenKind,
    ended: bool,
    index: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EditTargets {
    remove_paths: Vec<Vec<usize>>,
    merge_paths: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexTreePos {
    path: Vec<usize>,
    offset: usize,
}

impl IndexTreePos {
    fn new(path: Vec<usize>, offset: usize) -> Self {
        Self { path, offset }
    }
}

fn find_tree_pos(
    node: &TreeNode,
    path: Vec<usize>,
    index: usize,
    prefer_text: bool,
) -> Result<IndexTreePos> {
    if index > node.len() {
        return Err(YorkieError::InvalidTreePosition(format!(
            "index is out of range: {index} > {}",
            node.len()
        )));
    }

    if node.is_text() {
        return Ok(IndexTreePos::new(path, index));
    }

    let mut offset = 0;
    let mut pos = 0;
    for (visible_index, child) in node.children().enumerate() {
        let child_size = child.padded_size(false);
        let relative = index.saturating_sub(pos);

        if prefer_text && child.is_text() && child.len() >= relative {
            let mut child_path = path.clone();
            child_path.push(visible_index);
            return find_tree_pos(child, child_path, relative, prefer_text);
        }

        if index == pos {
            return Ok(IndexTreePos::new(path, offset));
        }

        if !prefer_text && child_size == relative {
            return Ok(IndexTreePos::new(path, offset + 1));
        }

        if child_size > relative {
            let mut child_path = path.clone();
            child_path.push(visible_index);
            return find_tree_pos(child, child_path, relative - 1, prefer_text);
        }

        pos += child_size;
        offset += 1;
    }

    Ok(IndexTreePos::new(path, offset))
}

fn tree_pos_to_path(root: &TreeNode, tree_pos: &IndexTreePos) -> Result<Vec<usize>> {
    let node = node_at_visible_path(root, &tree_pos.path)?;
    let mut path = Vec::new();
    let mut current_path = tree_pos.path.clone();

    if node.is_text() {
        let Some((parent_path, offset)) = split_parent_path(&current_path) else {
            return Err(YorkieError::InvalidTreePosition(
                "text node has no parent".to_owned(),
            ));
        };
        let parent = node_at_visible_path(root, &parent_path)?;
        path.push(left_siblings_size(parent, offset, false)? + tree_pos.offset);
        current_path = parent_path;
    } else if node.has_text_child() {
        path.push(left_siblings_size(node, tree_pos.offset, false)?);
    } else {
        path.push(tree_pos.offset);
    }

    while let Some((parent_path, offset)) = split_parent_path(&current_path) {
        path.push(offset);
        current_path = parent_path;
    }

    path.reverse();
    Ok(path)
}

fn path_to_tree_pos(root: &TreeNode, path: &[usize]) -> Result<IndexTreePos> {
    if path.is_empty() {
        return Err(YorkieError::InvalidTreePosition(
            "unacceptable path".to_owned(),
        ));
    }

    let mut node = root;
    let mut node_path = Vec::new();
    for path_element in &path[..path.len() - 1] {
        let child = visible_child(node, *path_element)
            .ok_or_else(|| YorkieError::InvalidTreePosition("unacceptable path".to_owned()))?;
        node_path.push(*path_element);
        node = child;
    }

    let last = path[path.len() - 1];
    if node.has_text_child() {
        return find_text_pos(node, node_path, last);
    }

    if visible_child_count(node) < last {
        return Err(YorkieError::InvalidTreePosition(
            "unacceptable path".to_owned(),
        ));
    }

    Ok(IndexTreePos::new(node_path, last))
}

fn find_text_pos(
    node: &TreeNode,
    path: Vec<usize>,
    mut path_element: usize,
) -> Result<IndexTreePos> {
    if node.len() < path_element {
        return Err(YorkieError::InvalidTreePosition(
            "unacceptable path".to_owned(),
        ));
    }

    for (visible_index, child) in node.children().enumerate() {
        let child_len = child.len();
        if child_len < path_element {
            path_element -= child_len;
        } else {
            let mut child_path = path;
            child_path.push(visible_index);
            return Ok(IndexTreePos::new(child_path, path_element));
        }
    }

    Ok(IndexTreePos::new(path, path_element))
}

fn index_of_tree_pos(
    root: &TreeNode,
    tree_pos: &IndexTreePos,
    include_removed: bool,
) -> Result<usize> {
    let node = node_at_visible_path(root, &tree_pos.path)?;
    let mut current_path = tree_pos.path.clone();
    let mut size = 0;
    let mut depth = 1;

    if node.is_text() {
        size += tree_pos.offset;
        let Some((parent_path, offset)) = split_parent_path(&current_path) else {
            return Err(YorkieError::InvalidTreePosition(
                "text node has no parent".to_owned(),
            ));
        };
        let parent = node_at_visible_path(root, &parent_path)?;
        size += left_siblings_size(parent, offset, include_removed)?;
        current_path = parent_path;
    } else {
        size += left_siblings_size(node, tree_pos.offset, include_removed)?;
    }

    while let Some((parent_path, offset)) = split_parent_path(&current_path) {
        let parent = node_at_visible_path(root, &parent_path)?;
        size += left_siblings_size(parent, offset, include_removed)?;
        depth += 1;
        current_path = parent_path;
    }

    Ok(size + depth - 1)
}

fn tree_pos_to_crdt_pos(root: &TreeNode, tree_pos: &IndexTreePos) -> Result<TreePos> {
    let node = node_at_visible_path(root, &tree_pos.path)?;

    let (parent, left_node) = if node.is_text() {
        let Some((parent_path, offset_in_parent)) = split_parent_path(&tree_pos.path) else {
            return Err(YorkieError::InvalidTreePosition(
                "text node has no parent".to_owned(),
            ));
        };
        let parent = node_at_visible_path(root, &parent_path)?;
        let left_node = if offset_in_parent == 0 && tree_pos.offset == 0 {
            parent
        } else {
            node
        };
        (parent, left_node)
    } else if tree_pos.offset == 0 {
        (node, node)
    } else {
        let left_node = visible_child(node, tree_pos.offset - 1)
            .ok_or_else(|| YorkieError::InvalidTreePosition("left sibling not found".to_owned()))?;
        (node, left_node)
    };

    Ok(TreePos::new(
        parent.id().clone(),
        left_node.id().split(tree_pos.offset),
    ))
}

fn left_siblings_size(parent: &TreeNode, offset: usize, include_removed: bool) -> Result<usize> {
    let children = if include_removed {
        parent.all_children().collect::<Vec<_>>()
    } else {
        parent.children().collect::<Vec<_>>()
    };

    if offset > children.len() {
        return Err(YorkieError::InvalidTreePosition(
            "offset is out of range".to_owned(),
        ));
    }

    Ok(children
        .into_iter()
        .take(offset)
        .map(|child| child.padded_size(include_removed))
        .sum())
}

fn node_at_visible_path<'a>(root: &'a TreeNode, path: &[usize]) -> Result<&'a TreeNode> {
    let mut node = root;
    for offset in path {
        node = visible_child(node, *offset)
            .ok_or_else(|| YorkieError::InvalidTreePosition("unacceptable path".to_owned()))?;
    }
    Ok(node)
}

fn node_at_visible_path_mut<'a>(
    root: &'a mut TreeNode,
    path: &[usize],
) -> Result<&'a mut TreeNode> {
    let mut node = root;
    for offset in path {
        node = visible_child_mut(node, *offset)
            .ok_or_else(|| YorkieError::InvalidTreePosition("unacceptable path".to_owned()))?;
    }
    Ok(node)
}

fn node_at_path<'a>(root: &'a TreeNode, path: &[usize]) -> Result<&'a TreeNode> {
    let mut node = root;
    for offset in path {
        node = node
            .children
            .get(*offset)
            .ok_or_else(|| YorkieError::InvalidTreePosition("unacceptable path".to_owned()))?;
    }
    Ok(node)
}

fn node_at_path_mut<'a>(root: &'a mut TreeNode, path: &[usize]) -> Result<&'a mut TreeNode> {
    let mut node = root;
    for offset in path {
        node = node
            .children
            .get_mut(*offset)
            .ok_or_else(|| YorkieError::InvalidTreePosition("unacceptable path".to_owned()))?;
    }
    Ok(node)
}

fn physical_to_visible_path(root: &TreeNode, path: &[usize]) -> Option<Vec<usize>> {
    let mut node = root;
    let mut visible_path = Vec::new();
    for offset in path {
        let child = node.children.get(*offset)?;
        if child.is_removed() {
            return None;
        }
        let visible_offset = node
            .children
            .iter()
            .take(*offset)
            .filter(|sibling| !sibling.is_removed())
            .count();
        visible_path.push(visible_offset);
        node = child;
    }
    Some(visible_path)
}

fn visible_child(node: &TreeNode, offset: usize) -> Option<&TreeNode> {
    node.children().nth(offset)
}

fn visible_child_mut(node: &mut TreeNode, offset: usize) -> Option<&mut TreeNode> {
    node.children
        .iter_mut()
        .filter(|child| !child.is_removed())
        .nth(offset)
}

fn visible_child_count(node: &TreeNode) -> usize {
    node.children().count()
}

fn split_parent_path(path: &[usize]) -> Option<(Vec<usize>, usize)> {
    let (&offset, parent_path) = path.split_last()?;
    Some((parent_path.to_vec(), offset))
}

fn collect_nodes<'a>(node: &'a TreeNode, nodes: &mut Vec<&'a TreeNode>) {
    nodes.push(node);
    for child in node.all_children() {
        collect_nodes(child, nodes);
    }
}

fn collect_node_map(node: &TreeNode, map: &mut BTreeMap<TreeNodeId, TreeNode>) {
    map.insert(node.id().clone(), node.clone());
    for child in node.all_children() {
        collect_node_map(child, map);
    }
}

fn collect_tokens_between(
    root: &TreeNode,
    from_idx: usize,
    to_idx: usize,
    include_removed: bool,
) -> Vec<TreeTokenTarget> {
    if from_idx == to_idx {
        return Vec::new();
    }

    let mut tokens = Vec::new();
    collect_tokens_between_node(
        root,
        Vec::new(),
        0,
        from_idx,
        to_idx,
        include_removed,
        &mut tokens,
    );
    tokens
}

fn collect_tokens_between_node(
    node: &TreeNode,
    path: Vec<usize>,
    base_index: usize,
    from_idx: usize,
    to_idx: usize,
    include_removed: bool,
    tokens: &mut Vec<TreeTokenTarget>,
) {
    let mut pos = 0;
    for (physical_index, child) in node.children.iter().enumerate() {
        if !include_removed && child.is_removed() {
            continue;
        }

        let child_size = child.padded_size(include_removed);
        if from_idx < pos + child_size && pos < to_idx {
            let padding = if child.is_text() { 0 } else { 1 };
            let from_child = from_idx as isize - pos as isize - padding as isize;
            let to_child = to_idx as isize - pos as isize - padding as isize;
            let child_len = if include_removed {
                child.total_len()
            } else {
                child.len()
            };
            let start_contained = !child.is_text() && from_child < 0;
            let end_contained = !child.is_text() && to_child > child_len as isize;

            let mut child_path = path.clone();
            child_path.push(physical_index);

            if child.is_text() || start_contained {
                tokens.push(TreeTokenTarget {
                    path: child_path.clone(),
                    kind: if child.is_text() {
                        TreeTokenKind::Text
                    } else {
                        TreeTokenKind::Start
                    },
                    ended: end_contained,
                    index: base_index + pos,
                });
            }

            let nested_from = from_child.max(0) as usize;
            let nested_to = to_child.min(child_len as isize).max(0) as usize;
            collect_tokens_between_node(
                child,
                child_path.clone(),
                base_index + pos + padding,
                nested_from,
                nested_to,
                include_removed,
                tokens,
            );

            if end_contained {
                tokens.push(TreeTokenTarget {
                    path: child_path,
                    kind: TreeTokenKind::End,
                    ended: end_contained,
                    index: base_index + pos + child_size - 1,
                });
            }
        }

        pos += child_size;
    }
}

fn node_can_delete(
    node: &TreeNode,
    edited_at: &TimeTicket,
    version_vector: Option<&crate::VersionVector>,
) -> bool {
    if !ticket_known(version_vector, node.id().created_at()) {
        return false;
    }

    match node.removed_at() {
        None => true,
        Some(removed_at) => {
            !ticket_known(version_vector, removed_at) && edited_at.after(removed_at)
        }
    }
}

fn ticket_known(version_vector: Option<&crate::VersionVector>, ticket: &TimeTicket) -> bool {
    version_vector
        .map(|vector| vector.after_or_equal(ticket))
        .unwrap_or(true)
}

fn find_visible_path_by_node_id(
    node: &TreeNode,
    id: &TreeNodeId,
    current: &mut Vec<usize>,
    result: &mut Vec<usize>,
) -> bool {
    if node.id() == id {
        *result = current.clone();
        return true;
    }

    for (visible_index, child) in node.children().enumerate() {
        current.push(visible_index);
        if find_visible_path_by_node_id(child, id, current, result) {
            return true;
        }
        current.pop();
    }

    false
}

fn find_path_by_node_id(
    node: &TreeNode,
    id: &TreeNodeId,
    current: &mut Vec<usize>,
    result: &mut Vec<usize>,
) -> bool {
    if node.id() == id {
        *result = current.clone();
        return true;
    }

    for (index, child) in node.all_children().enumerate() {
        current.push(index);
        if find_path_by_node_id(child, id, current, result) {
            return true;
        }
        current.pop();
    }

    false
}

fn subtree_data_size(node: &TreeNode) -> DataSize {
    let mut size = node.data_size();
    for child in node.all_children() {
        add_data_size(&mut size, subtree_data_size(child));
    }
    size
}

fn descendant_paths(root: &TreeNode, path: &[usize]) -> Vec<Vec<usize>> {
    let Ok(node) = node_at_path(root, path) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    collect_descendant_paths(node, path.to_vec(), &mut paths);
    paths
}

fn collect_descendant_paths(node: &TreeNode, path: Vec<usize>, paths: &mut Vec<Vec<usize>>) {
    for (index, child) in node.all_children().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        paths.push(child_path.clone());
        collect_descendant_paths(child, child_path, paths);
    }
}

fn client_lamport(version_vector: Option<&crate::VersionVector>, created_at: &TimeTicket) -> i64 {
    version_vector
        .and_then(|vector| vector.get(created_at.actor_id().as_str()))
        .unwrap_or(if version_vector.is_some() {
            0
        } else {
            i64::MAX
        })
}

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

fn collect_merge_pairs(node: &TreeNode, pairs: &mut Vec<(TreeNodeId, TreeNodeId)>) {
    for child in node.all_children() {
        if let Some(merged_from) = child.merged_from() {
            pairs.push((merged_from.clone(), node.id().clone()));
        }
        collect_merge_pairs(child, pairs);
    }
}

fn rebuild_merge_state_in_node(node: &mut TreeNode, pairs: &[(TreeNodeId, TreeNodeId)]) {
    for (source, target) in pairs {
        if node.id() == source && node.merged_into().is_none() {
            node.set_merged_into(Some(target.clone()));
        }
    }

    for child in node.all_children_mut() {
        rebuild_merge_state_in_node(child, pairs);
    }
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn split_utf16(value: &str, offset: usize) -> (String, String) {
    let units = value.encode_utf16().collect::<Vec<_>>();
    let offset = offset.min(units.len());
    (
        String::from_utf16_lossy(&units[..offset]),
        String::from_utf16_lossy(&units[offset..]),
    )
}

fn split_issue_ticket(
    edited_at: &TimeTicket,
    contents_len: usize,
    split_index: usize,
) -> TimeTicket {
    TimeTicket::new(
        edited_at.lamport(),
        edited_at
            .delimiter()
            .saturating_add(contents_len as u32)
            .saturating_add(split_index as u32)
            .saturating_add(1),
        edited_at.actor_id().clone(),
    )
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attribute(value: &str) -> String {
    escape_xml_text(value).replace('"', "&quot;")
}

fn attributes_to_json(attrs: &Rht) -> String {
    let items = attrs
        .to_object()
        .into_iter()
        .map(|(key, value)| {
            format!(
                "\"{}\":{}",
                escape_json_string(&key),
                attribute_value_to_json(&value)
            )
        })
        .collect::<Vec<_>>();

    format!("{{{}}}", items.join(","))
}

fn attribute_value_to_json(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        return trimmed.to_owned();
    }

    match trimmed {
        "true" | "false" | "null" => trimmed.to_owned(),
        _ if trimmed.parse::<f64>().is_ok() => trimmed.to_owned(),
        _ => format!("\"{}\"", escape_json_string(value)),
    }
}

pub(crate) fn attribute_value_to_json_value(value: &str) -> JsonValue {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return JsonValue::String(unescape_json_string(&trimmed[1..trimmed.len() - 1]));
    }

    match trimmed {
        "true" => JsonValue::Bool(true),
        "false" => JsonValue::Bool(false),
        "null" => JsonValue::Null,
        _ => trimmed
            .parse::<i32>()
            .map(JsonValue::Integer)
            .or_else(|_| trimmed.parse::<i64>().map(JsonValue::Long))
            .or_else(|_| trimmed.parse::<f64>().map(JsonValue::Double))
            .unwrap_or_else(|_| JsonValue::String(value.to_owned())),
    }
}

fn attribute_value_to_xml_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return unescape_json_string(&trimmed[1..trimmed.len() - 1]);
    }

    value.to_owned()
}

fn unescape_json_string(value: &str) -> String {
    let mut decoded = String::new();
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some('"') => decoded.push('"'),
            Some('\\') => decoded.push('\\'),
            Some('/') => decoded.push('/'),
            Some('b') => decoded.push('\u{08}'),
            Some('f') => decoded.push('\u{0c}'),
            Some('n') => decoded.push('\n'),
            Some('r') => decoded.push('\r'),
            Some('t') => decoded.push('\t'),
            Some('u') => {
                let hex = chars.by_ref().take(4).collect::<String>();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        decoded.push(ch);
                    }
                }
            }
            Some(ch) => decoded.push(ch),
            None => decoded.push('\\'),
        }
    }

    decoded
}

#[cfg(test)]
mod tests {
    use super::{CrdtTree, TreeNode, TreeNodeId};
    use crate::crdt::rht::Rht;
    use crate::{TimeTicket, TIME_TICKET_SIZE};
    use std::collections::BTreeMap;

    #[test]
    fn creates_tree_nodes_and_serializes_json_and_xml() {
        let mut attrs = Rht::new();
        attrs.set("bold", "\"true\"", ticket(4, "a"));
        let mut paragraph = TreeNode::create_element(
            node_id(2, 0),
            "p",
            Some(attrs),
            vec![TreeNode::create_text(node_id(3, 0), "hello")],
        );
        paragraph.append(TreeNode::create_element(
            node_id(5, 0),
            "br",
            None,
            Vec::new(),
        ));

        let tree = CrdtTree::create(
            TreeNode::create_element(node_id(1, 0), "root", None, vec![paragraph]),
            ticket(10, "a"),
        );

        assert_eq!(
            r#"{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"hello"},{"type":"br","children":[]}],"attributes":{"bold":"true"}}]}"#,
            tree.to_json()
        );
        assert_eq!(
            r#"<root><p bold="true">hello<br></br></p></root>"#,
            tree.to_xml()
        );
        assert_eq!(4, tree.node_map_len());
    }

    #[test]
    fn finds_floor_node_by_split_id() {
        let tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![
                    TreeNode::create_text(node_id(2, 0), "hel"),
                    TreeNode::create_text(node_id(2, 3), "lo"),
                ],
            ),
            ticket(10, "a"),
        );

        assert_eq!(
            &node_id(2, 3),
            tree.find_floor_node(&node_id(2, 4)).unwrap().id()
        );
        assert!(tree.find_floor_node(&node_id(3, 0)).is_none());
    }

    #[test]
    fn converts_tree_indexes_paths_and_positions() -> crate::Result<()> {
        let tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(node_id(3, 0), "ABC")],
                )],
            ),
            ticket(10, "a"),
        );

        assert_eq!(5, tree.root().len());
        assert_eq!(vec![0, 2], tree.index_to_path(3)?);
        assert_eq!(3, tree.path_to_index(&[0, 2])?);
        assert_eq!(
            "2:a:0/0:3:a:0/2",
            tree.path_to_pos(&[0, 2])?.to_test_string()
        );
        assert_eq!("1:a:0/0:1:a:0/0", tree.find_pos(0, true)?.to_test_string());
        assert_eq!("2:a:0/0:2:a:0/0", tree.find_pos(1, true)?.to_test_string());
        assert_eq!("2:a:0/0:3:a:0/3", tree.find_pos(4, true)?.to_test_string());
        Ok(())
    }

    #[test]
    fn deepcopies_tree_nodes_and_rebuilds_node_map() {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_text(node_id(2, 0), "hello")],
            ),
            ticket(10, "a"),
        );
        tree.root_mut()
            .append(TreeNode::create_text(node_id(3, 0), "world"));
        let moved_at = ticket(5, "a");
        let removed_at = ticket(6, "a");
        tree.set_moved_at(Some(moved_at.clone()));
        tree.set_removed_at(Some(removed_at.clone()));

        let copy = tree.deepcopy();
        tree.root_mut()
            .append(TreeNode::create_text(node_id(4, 0), "!"));

        assert_eq!(3, copy.node_map_len());
        assert_eq!(Some(&moved_at), copy.moved_at());
        assert_eq!(Some(&removed_at), copy.removed_at());
        assert_eq!(
            r#"{"type":"root","children":[{"type":"text","value":"hello"},{"type":"text","value":"world"}]}"#,
            copy.to_json()
        );
    }

    #[test]
    fn reports_tree_data_size_and_gc_pairs() {
        let mut root = TreeNode::create_element(
            node_id(1, 0),
            "root",
            None,
            vec![TreeNode::create_text(node_id(2, 0), "ab")],
        );
        let removed_at = ticket(3, "a");
        root.all_children_mut()
            .next()
            .unwrap()
            .remove(removed_at.clone());
        let tree = CrdtTree::create(root, ticket(10, "a"));

        assert_eq!(TIME_TICKET_SIZE * 2, tree.data_size().meta);
        assert_eq!(0, tree.data_size().data);
        assert_eq!(1, tree.gc_pair_entries().len());
        assert_eq!(removed_at, tree.gc_pair_entries()[0].2);
    }

    #[test]
    fn styles_element_tokens_and_ignores_text_only_ranges() -> crate::Result<()> {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![
                    TreeNode::create_element(
                        node_id(2, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(node_id(3, 0), "ab")],
                    ),
                    TreeNode::create_element(
                        node_id(4, 0),
                        "p",
                        None,
                        vec![TreeNode::create_text(node_id(5, 0), "cd")],
                    ),
                ],
            ),
            ticket(1, "a"),
        );

        let opening = (tree.find_pos(0, true)?, tree.find_pos(1, true)?);
        tree.style_by_range_with_changes(
            opening,
            BTreeMap::from([("weight".to_owned(), "bold".to_owned())]),
            ticket(10, "a"),
            None,
        )?;
        assert_eq!(
            r#"<root><p weight="bold">ab</p><p>cd</p></root>"#,
            tree.to_xml()
        );

        let closing = (tree.find_pos(3, true)?, tree.find_pos(4, true)?);
        tree.style_by_range_with_changes(
            closing,
            BTreeMap::from([("color".to_owned(), "red".to_owned())]),
            ticket(11, "a"),
            None,
        )?;
        assert_eq!(
            r#"<root><p color="red" weight="bold">ab</p><p>cd</p></root>"#,
            tree.to_xml()
        );

        let text_only = (tree.find_pos(1, true)?, tree.find_pos(3, true)?);
        let (_, _, changes, _, _) = tree.style_by_range_with_changes(
            text_only,
            BTreeMap::from([("ignored".to_owned(), "true".to_owned())]),
            ticket(12, "a"),
            None,
        )?;
        assert!(changes.is_empty());
        assert_eq!(
            r#"<root><p color="red" weight="bold">ab</p><p>cd</p></root>"#,
            tree.to_xml()
        );

        Ok(())
    }

    #[test]
    fn styles_text_only_ranges_by_splitting_boundaries() -> crate::Result<()> {
        let mut tree = text_tree("hello");
        let range = (tree.find_pos(2, true)?, tree.find_pos(4, true)?);

        let (_, diff, changes, previous, attributes_to_remove) = tree.style_by_range_with_changes(
            range,
            BTreeMap::from([("bold".to_owned(), "true".to_owned())]),
            ticket(10, "a"),
            None,
        )?;

        assert!(changes.is_empty());
        assert!(previous.is_empty());
        assert!(attributes_to_remove.is_empty());
        assert_eq!(0, diff.data);
        assert_eq!(TIME_TICKET_SIZE * 2, diff.meta);
        assert_eq!(5, tree.node_map_len());
        assert_eq!(r#"<root><p>hello</p></root>"#, tree.to_xml());
        assert_eq!(
            r#"{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"h"},{"type":"text","value":"el"},{"type":"text","value":"lo"}]}]}"#,
            tree.to_json()
        );
        Ok(())
    }

    #[test]
    fn removes_tree_style_attributes() -> crate::Result<()> {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    {
                        let mut attrs = Rht::new();
                        attrs.set("bold", "true", ticket(3, "a"));
                        Some(attrs)
                    },
                    vec![TreeNode::create_text(node_id(4, 0), "hello")],
                )],
            ),
            ticket(1, "a"),
        );

        let range = (tree.find_pos(0, true)?, tree.find_pos(1, true)?);
        let (removed, _, changes, previous) = tree.remove_style_by_range_with_changes(
            range,
            &["bold".to_owned(), "missing".to_owned()],
            ticket(10, "a"),
            None,
        )?;

        assert_eq!(2, removed.len());
        assert_eq!(
            BTreeMap::from([("bold".to_owned(), "true".to_owned())]),
            previous
        );
        assert_eq!(1, changes.len());
        assert_eq!(r#"<root><p>hello</p></root>"#, tree.to_xml());
        Ok(())
    }

    #[test]
    fn removes_style_from_text_only_ranges_by_splitting_boundaries() -> crate::Result<()> {
        let mut tree = text_tree("hello");
        let range = (tree.find_pos(2, true)?, tree.find_pos(4, true)?);

        let (removed, diff, changes, previous) = tree.remove_style_by_range_with_changes(
            range,
            &["bold".to_owned()],
            ticket(10, "a"),
            None,
        )?;

        assert!(removed.is_empty());
        assert!(changes.is_empty());
        assert!(previous.is_empty());
        assert_eq!(0, diff.data);
        assert_eq!(TIME_TICKET_SIZE * 2, diff.meta);
        assert_eq!(5, tree.node_map_len());
        assert_eq!(r#"<root><p>hello</p></root>"#, tree.to_xml());
        Ok(())
    }

    #[test]
    fn edits_tree_by_inserting_element_nodes() -> crate::Result<()> {
        let mut tree = CrdtTree::create(TreeNode::create_root(node_id(1, 0)), ticket(1, "a"));
        let pos = tree.find_pos(0, true)?;
        let result = tree.edit_by_range_with_changes(
            (pos.clone(), pos),
            Some(vec![TreeNode::create_element(
                node_id(2, 0),
                "p",
                None,
                vec![TreeNode::create_text(node_id(3, 0), "hello")],
            )]),
            0,
            ticket(10, "a"),
            None,
        )?;

        assert_eq!(r#"<root><p>hello</p></root>"#, tree.to_xml());
        assert_eq!(0, result.from_idx);
        assert_eq!(0, result.to_idx);
        assert_eq!(7, result.inserted_size);
        assert_eq!(vec![0], result.changes[0].from_path);
        assert_eq!(vec![0], result.changes[0].to_path);
        assert_eq!(
            Some(vec![
                r#"{"type":"p","children":[{"type":"text","value":"hello"}]}"#.to_owned()
            ]),
            result.changes[0].value
        );
        Ok(())
    }

    #[test]
    fn edits_tree_by_removing_element_nodes() -> crate::Result<()> {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(node_id(3, 0), "hello")],
                )],
            ),
            ticket(1, "a"),
        );
        let range = (tree.path_to_pos(&[0])?, tree.path_to_pos(&[1])?);

        let result = tree.edit_by_range_with_changes(range, None, 0, ticket(10, "a"), None)?;

        assert_eq!(r#"<root></root>"#, tree.to_xml());
        assert_eq!(0, result.from_idx);
        assert_eq!(7, result.to_idx);
        assert_eq!(2, result.removed_nodes.len());
        assert_eq!(2, result.gc_pairs.len());
        assert_eq!(None, result.changes[0].value);
        Ok(())
    }

    #[test]
    fn edits_tree_by_inserting_inside_text_nodes() -> crate::Result<()> {
        let mut tree = text_tree("hello");
        let pos = tree.find_pos(3, true)?;

        let result = tree.edit_by_range_with_changes(
            (pos.clone(), pos),
            Some(vec![TreeNode::create_text(node_id(4, 0), "X")]),
            0,
            ticket(10, "a"),
            None,
        )?;

        assert_eq!(r#"<root><p>heXllo</p></root>"#, tree.to_xml());
        assert_eq!(3, result.from_idx);
        assert_eq!(3, result.to_idx);
        assert_eq!(1, result.inserted_size);
        assert_eq!(5, tree.node_map_len());
        Ok(())
    }

    #[test]
    fn edits_tree_by_deleting_inside_text_nodes() -> crate::Result<()> {
        let mut tree = text_tree("hello");
        let range = (tree.find_pos(2, true)?, tree.find_pos(5, true)?);

        let result = tree.edit_by_range_with_changes(range, None, 0, ticket(10, "a"), None)?;

        assert_eq!(r#"<root><p>ho</p></root>"#, tree.to_xml());
        assert_eq!(2, result.from_idx);
        assert_eq!(5, result.to_idx);
        assert_eq!(1, result.removed_nodes.len());
        assert_eq!(1, result.gc_pairs.len());
        assert_eq!(5, tree.node_map_len());
        Ok(())
    }

    #[test]
    fn edits_tree_by_splitting_element_nodes() -> crate::Result<()> {
        let mut attrs = Rht::new();
        attrs.set("bold", "true", ticket(4, "a"));
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    Some(attrs),
                    vec![TreeNode::create_text(node_id(3, 0), "helloworld")],
                )],
            ),
            ticket(1, "a"),
        );
        let pos = tree.find_pos(6, true)?;

        let result =
            tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(10, "a"), None)?;

        assert_eq!(
            r#"<root><p bold="true">hello</p><p bold="true">world</p></root>"#,
            tree.to_xml()
        );
        assert_eq!(6, result.from_idx);
        assert_eq!(6, result.to_idx);
        assert_eq!(Some(1), result.changes[0].split_level);
        assert_eq!(5, tree.node_map_len());
        Ok(())
    }

    #[test]
    fn splits_tree_content_by_path() -> crate::Result<()> {
        let mut tree = nested_tree();

        let pos = tree.path_to_pos(&[0, 0, 0, 2])?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(20, "a"), None)?;
        assert_eq!(
            r#"<doc><tc><p><tn>12</tn><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>"#,
            tree.to_xml()
        );

        let pos = tree.path_to_pos(&[0, 0, 1])?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(21, "a"), None)?;
        assert_eq!(
            r#"<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>"#,
            tree.to_xml()
        );

        let pos = tree.path_to_pos(&[0, 2, 0, 4])?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(22, "a"), None)?;
        assert_eq!(
            r#"<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn><tn></tn></p></tc></doc>"#,
            tree.to_xml()
        );

        let pos = tree.path_to_pos(&[0, 2, 1])?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(23, "a"), None)?;
        assert_eq!(
            r#"<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p><p><tn></tn></p></tc></doc>"#,
            tree.to_xml()
        );
        Ok(())
    }

    #[test]
    fn merges_tree_content_by_path() -> crate::Result<()> {
        let mut tree = nested_tree();

        let range = (tree.path_to_pos(&[0, 0, 1])?, tree.path_to_pos(&[0, 1, 0])?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(20, "a"), None)?;
        assert_eq!(
            r#"<doc><tc><p><tn>1234</tn><tn>5678</tn></p></tc></doc>"#,
            tree.to_xml()
        );

        let range = (
            tree.path_to_pos(&[0, 0, 0, 4])?,
            tree.path_to_pos(&[0, 0, 1, 0])?,
        );
        tree.edit_by_range_with_changes(range, None, 0, ticket(21, "a"), None)?;
        assert_eq!(
            r#"<doc><tc><p><tn>12345678</tn></p></tc></doc>"#,
            tree.to_xml()
        );
        Ok(())
    }

    fn nested_tree() -> CrdtTree {
        CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "doc",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "tc",
                    None,
                    vec![
                        TreeNode::create_element(
                            node_id(3, 0),
                            "p",
                            None,
                            vec![TreeNode::create_element(
                                node_id(4, 0),
                                "tn",
                                None,
                                vec![TreeNode::create_text(node_id(5, 0), "1234")],
                            )],
                        ),
                        TreeNode::create_element(
                            node_id(6, 0),
                            "p",
                            None,
                            vec![TreeNode::create_element(
                                node_id(7, 0),
                                "tn",
                                None,
                                vec![TreeNode::create_text(node_id(8, 0), "5678")],
                            )],
                        ),
                    ],
                )],
            ),
            ticket(1, "a"),
        )
    }

    fn text_tree(value: &str) -> CrdtTree {
        CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(node_id(3, 0), value)],
                )],
            ),
            ticket(1, "a"),
        )
    }

    fn node_id(lamport: i64, offset: usize) -> TreeNodeId {
        TreeNodeId::new(ticket(lamport, "a"), offset)
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
