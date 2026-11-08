use super::element::{CrdtElementMeta, DataSize};
use super::rht::{Rht, RhtNode};
use crate::json::escape_json_string;
use crate::{Result, TimeTicket, TreeNodeInput, YorkieError, TIME_TICKET_SIZE};
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

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.removed_at = removed_at;
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

    /// Reports whether an edit at `edited_at` removes this node (JS
    /// `canDelete`): its creation must be known to the edit, and a tombstone
    /// it already has must be one the edit did not know and older, so
    /// concurrent deletions resolve last writer wins.
    pub(crate) fn can_delete(
        &self,
        edited_at: &TimeTicket,
        creation_known: bool,
        tombstone_known: bool,
    ) -> bool {
        if !creation_known {
            return false;
        }
        match &self.removed_at {
            None => true,
            Some(removed_at) => !tombstone_known && edited_at.after(removed_at),
        }
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
    /// The IDs of every node, removed ones too, for floor lookups (JS
    /// `nodeMapByID`). The nodes themselves are found by walking the tree.
    node_ids: BTreeSet<TreeNodeId>,
    /// GC pairs of text pieces split off removed nodes, drained by the edit
    /// or style that split them (JS `pendingGCPairs`).
    pending_gc_pairs: Vec<(String, DataSize, TimeTicket)>,
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeEditChange {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) from_path: Vec<usize>,
    pub(crate) to_path: Vec<usize>,
    pub(crate) value: Option<Vec<TreeNodeInput>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeEditResult {
    pub(crate) changes: Vec<TreeEditChange>,
    pub(crate) gc_pairs: Vec<(String, DataSize, TimeTicket)>,
    pub(crate) diff: DataSize,
    pub(crate) removed_nodes: Vec<TreeNode>,
    pub(crate) merge_level: usize,
    pub(crate) from_idx: usize,
    pub(crate) to_idx: usize,
    pub(crate) inserted_size: usize,
}

type TreeStyleResult = (
    Vec<RhtNode>,
    DataSize,
    Vec<TreeStyleChange>,
    BTreeMap<String, String>,
    Vec<String>,
);
type TreeRemoveStyleResult = (
    Vec<RhtNode>,
    DataSize,
    Vec<TreeStyleChange>,
    BTreeMap<String, String>,
);

impl CrdtTree {
    pub(crate) fn new(root: TreeNode, created_at: TimeTicket) -> Self {
        let mut tree = Self {
            meta: CrdtElementMeta::new(created_at),
            root,
            node_ids: BTreeSet::new(),
            pending_gc_pairs: Vec::new(),
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
        let floor = self.floor_id(id)?;
        let path = self.physical_path_by_node_id(&floor)?;
        node_at_path(&self.root, &path).ok()
    }

    pub(crate) fn node_map_len(&self) -> usize {
        self.node_ids.len()
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
    ) -> Result<TreeStyleResult> {
        let (from_idx, to_idx, mut diff) = self.style_range(&range, &edited_at)?;
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

                // A removed node is not part of the rendered document.
                if !affected_attrs.is_empty() && !node.is_removed() {
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
    ) -> Result<TreeRemoveStyleResult> {
        let (from_idx, to_idx, mut diff) = self.style_range(&range, &edited_at)?;
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

                // As for `style`, a removed node reports no change. The
                // attribute store records a tombstone even for a key it did
                // not hold, so such a removal still reports one, as in JS.
                if removed_any && !node.is_removed() {
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

    /// Edits the range with `contents`, issuing the tickets of the elements a
    /// split creates the way a replica does when the operation carries none
    /// (JS `TreeEditOperation.execute` fallback): one delimiter after
    /// another, starting past the top-level contents.
    pub(crate) fn edit_by_range_with_changes(
        &mut self,
        range: (TreePos, TreePos),
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
        edited_at: TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<TreeEditResult> {
        let mut issue_split_ticket =
            fallback_split_tickets(&edited_at, contents.as_ref().map(Vec::len));
        self.edit(
            range,
            contents,
            split_level,
            edited_at,
            &mut issue_split_ticket,
            version_vector,
        )
    }

    /// Edits the range with `contents` and splits `split_level` levels at
    /// its start (JS `CRDTTree.edit`). `issue_split_ticket` gives the ticket
    /// of each element the split creates.
    pub(crate) fn edit(
        &mut self,
        range: (TreePos, TreePos),
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
        edited_at: TimeTicket,
        issue_split_ticket: &mut dyn FnMut() -> TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<TreeEditResult> {
        let mut diff = DataSize::default();
        let (from_parent_path, from_left_raw_path) =
            self.find_nodes_and_split_text(&range.0, &edited_at, false, &mut diff)?;
        let from_parent = self.id_at(&from_parent_path)?;
        let from_left_raw = self.id_at(&from_left_raw_path)?;
        let (to_parent_path, to_left_raw_path) =
            self.find_nodes_and_split_text(&range.1, &edited_at, false, &mut diff)?;
        let to_parent = self.id_at(&to_parent_path)?;
        let to_left_raw = self.id_at(&to_left_raw_path)?;
        let mut gc_pairs = Vec::new();
        let mut removed_nodes = Vec::new();

        // 01-1. Advance past the split siblings the editor did not know. The
        // editing actor is passed so a same-boundary empty run resolves here
        // as the split step resolves it (JS §7.7).
        let actor_id = edited_at.actor_id().as_str().to_owned();
        let from_left = if from_left_raw != from_parent {
            self.advance_past_unknown_split_siblings(
                &from_left_raw,
                version_vector,
                false,
                Some(&actor_id),
            )?
        } else {
            from_left_raw
        };
        let to_left = if to_left_raw != to_parent {
            self.advance_past_unknown_split_siblings(
                &to_left_raw,
                version_vector,
                false,
                Some(&actor_id),
            )?
        } else {
            to_left_raw
        };
        let from_idx = self.to_index(&from_parent, &from_left)?;
        let to_idx = self.to_index(&to_parent, &to_left)?;
        let from_path = self.index_to_path(from_idx)?;

        // Range narrowing: when a concurrent split put the two ends in
        // different parents, start the traversal at the split sibling of
        // `from_left` in `to_parent`. Merge, split and insert still use the
        // original `from_parent` and `from_left`.
        let mut collect_from_parent = from_parent.clone();
        let mut collect_from_left = from_left.clone();
        if from_left != from_parent && from_parent != to_parent {
            let mut walker = InsNextWalker::default();
            walker.visit(&from_left);
            let mut current = from_left.clone();
            while let Some(ins_next_id) = self.node(&current)?.ins_next_id().cloned() {
                let Some(next) = self.floor_id(&ins_next_id) else {
                    break;
                };
                if self.node(&next)?.is_text() || !walker.visit(&next) {
                    break;
                }
                if self.parent_id(&next).as_ref() == Some(&to_parent) {
                    // At the start of `to_parent` the narrowed range would run
                    // backwards and drop the merge the editor asked for.
                    if to_left != to_parent {
                        collect_from_left = next;
                        collect_from_parent = to_parent.clone();
                    }
                    break;
                }
                current = next;
            }
        }

        // The traversal covers tombstones too (JS `traverseInPosRange`), so it
        // runs over indexes that count removed nodes, not the visible ones.
        let collect_from_idx =
            self.physical_index_after(&collect_from_parent, &collect_from_left)?;
        let collect_to_idx = self.physical_index_after(&to_parent, &to_left)?;
        let targets = if collect_from_idx < collect_to_idx {
            self.edit_targets(collect_from_idx, collect_to_idx, &edited_at, version_vector)?
        } else {
            EditTargets::default()
        };
        // A concurrent deletion splits the range into several changes.
        let mut changes = self.make_deletion_changes(&targets.removal_tokens)?;
        let merge_level = targets.merged.len();

        // 02. Delete the nodes marked as removed.
        for id in &targets.removed {
            let node = self.node_mut(id)?;
            if node.remove(edited_at.clone()) {
                gc_pairs.push((node.id_string(), node.data_size(), edited_at.clone()));
                removed_nodes.push(node.clone());
            }
        }

        // 03. Merge: move the children of the merged elements to the merge
        // target, and 03-1, delete what earlier merges moved out of the
        // elements this edit deleted whole.
        let dest = self.resolve_merge_target(&from_parent)?;
        for (id, size) in self.move_merged_children(&targets.moved, &dest, &edited_at)? {
            gc_pairs.push((id, size, edited_at.clone()));
        }
        for (id, size) in self.propagate_merge_deletes(&targets, &dest, &range, &edited_at)? {
            gc_pairs.push((id, size, edited_at.clone()));
        }

        // 04. Split the elements for the split level.
        if split_level > 0 {
            let (split_idx, split_path) = self
                .split_levels(
                    &from_parent,
                    &from_left,
                    split_level,
                    &edited_at,
                    issue_split_ticket,
                    version_vector,
                    &mut diff,
                )?
                .unwrap_or_else(|| (from_idx, from_path.clone()));
            changes.push(TreeEditChange {
                from: split_idx,
                to: split_idx,
                from_path: split_path.clone(),
                to_path: split_path,
                value: None,
            });
        }
        let from_parent_path = self.path_of(&from_parent)?;
        let from_left_path = self.path_of(&from_left)?;

        let mut inserted_size = 0;
        let mut inserted_value = contents.as_ref().map(|nodes| {
            nodes
                .iter()
                .map(TreeNodeInput::from_crdt)
                .collect::<Vec<_>>()
        });
        if let Some(contents) = contents {
            // Insert right after the left node, or first in the parent (JS
            // `CRDTTree.edit` step 05).
            debug_assert!(
                from_left_path == from_parent_path
                    || split_parent_path(&from_left_path).map(|(path, _)| path)
                        == Some(from_parent_path.clone()),
                "the left sibling must be a child of the insertion parent"
            );
            let first_index = if from_left_path == from_parent_path {
                0
            } else {
                from_left_path.last().map_or(0, |index| index + 1)
            };
            let intended = self.intended_merge_parent(&range.0, &from_parent)?;
            let parent = node_at_path_mut(&mut self.root, &from_parent_path)?;
            // Content inserted into a parent removed concurrently is born
            // removed: it never counts as live and shows in no change.
            let parent_removed = parent.is_removed();
            for (insert_index, mut content) in (first_index..).zip(contents) {
                content.clear_removed_recursively();
                if let Some((merged_from, merged_at)) = &intended {
                    content.merged_from = Some(merged_from.clone());
                    content.merged_at = merged_at.clone();
                }
                if parent_removed {
                    content.remove_recursively(edited_at.clone(), &mut gc_pairs, &mut Vec::new());
                } else {
                    inserted_size += content.padded_size(false);
                    add_data_size(&mut diff, subtree_data_size(&content));
                }
                parent.insert_at(insert_index, content);
            }
            if parent_removed {
                inserted_value = None;
            }
        }

        self.rebuild_node_map();
        gc_pairs.extend(self.drain_pending_gc_pairs());

        let changes = attach_insertion_change(changes, from_idx, from_path, inserted_value);

        Ok(TreeEditResult {
            changes,
            gc_pairs,
            diff,
            removed_nodes,
            merge_level,
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
            node_ids: BTreeSet::new(),
            pending_gc_pairs: Vec::new(),
        };
        tree.rebuild_node_map();
        tree.rebuild_merge_state();
        tree
    }

    pub(crate) fn gc_pair_entries(&self) -> Vec<(String, DataSize, TimeTicket)> {
        self.root.gc_pair_entries()
    }

    /// Purges the removed node or attribute `child_id`. A purged node's split
    /// siblings on either side are linked to each other, so the split chain
    /// does not stop at it (JS `CRDTTree.purge`).
    pub(crate) fn purge_gc_pair_by_id(&mut self, child_id: &str) -> bool {
        let links = self
            .nodes()
            .find(|node| node.id_string() == child_id)
            .map(|node| (node.ins_prev_id().cloned(), node.ins_next_id().cloned()));
        let purged = self.root.purge_gc_pair_by_id(child_id);
        if !purged {
            return false;
        }
        self.rebuild_node_map();

        if let Some((prev_id, next_id)) = links {
            if let Some(prev) = prev_id.as_ref().and_then(|id| self.floor_id(id)) {
                if let Ok(prev) = self.node_mut(&prev) {
                    prev.set_ins_next_id(next_id.clone());
                }
            }
            if let Some(next) = next_id.as_ref().and_then(|id| self.floor_id(id)) {
                if let Ok(next) = self.node_mut(&next) {
                    next.set_ins_prev_id(prev_id);
                }
            }
        }
        true
    }

    pub(crate) fn nodes(&self) -> impl Iterator<Item = &TreeNode> {
        let mut nodes = Vec::new();
        collect_nodes(&self.root, &mut nodes);
        nodes.into_iter()
    }

    /// Finds the parent and left sibling of `pos` as physical paths, splitting
    /// the text node the position falls in (JS `findNodesAndSplitText`). The
    /// nodes are found by ID, removed ones too, and the left sibling moves
    /// past nodes inserted at the same position after `edited_at`, as in RGA.
    /// `range_boundary` resolves a position in a parent merged away to right
    /// after it rather than into the merge target, as JS does for ranges.
    fn find_nodes_and_split_text(
        &mut self,
        pos: &TreePos,
        edited_at: &TimeTicket,
        range_boundary: bool,
        diff: &mut DataSize,
    ) -> Result<(Vec<usize>, Vec<usize>)> {
        let not_found = || {
            YorkieError::InvalidTreePosition(format!(
                "cannot find node of TreePos({})",
                pos.to_test_string()
            ))
        };
        let parent_path = self
            .floor_node_path(pos.parent_id())
            .ok_or_else(not_found)?;
        let mut left_path = self
            .floor_node_path(pos.left_sibling_id())
            .ok_or_else(not_found)?;

        // A left sibling ID at the start of a split node means the end of the
        // node it was split from.
        let left = node_at_path(&self.root, &left_path)?;
        if pos.left_sibling_id() != pos.parent_id()
            && pos.left_sibling_id().offset() > 0
            && pos.left_sibling_id().offset() == left.id().offset()
        {
            if let Some(prev_id) = left.ins_prev_id().cloned() {
                left_path = self.floor_node_path(&prev_id).ok_or_else(not_found)?;
            }
        }

        let is_left_most = parent_path == left_path;
        let real_parent_path = match split_parent_path(&left_path) {
            Some((path, _)) if !is_left_most => path,
            _ => parent_path,
        };

        let real_parent = node_at_path(&self.root, &real_parent_path)?;
        if real_parent.is_removed() && is_left_most {
            if let Some(merged_into) = real_parent.merged_into().cloned() {
                if let Some(paths) =
                    self.merged_parent_position(&real_parent_path, &merged_into, range_boundary)?
                {
                    return Ok(paths);
                }
            }
        }

        let left = node_at_path(&self.root, &left_path)?;
        if left.is_text() {
            if let Some((text_parent_path, index)) = split_parent_path(&left_path) {
                let split_diff = node_at_path_mut(&mut self.root, &text_parent_path)?
                    .split_text_child_at(index, pos.left_sibling_id().offset())?;
                if split_diff != DataSize::default() {
                    let mut split_path = text_parent_path;
                    split_path.push(index + 1);
                    let split = node_at_path(&self.root, &split_path)?;
                    // A piece split off a removed node is born removed: it
                    // was never live, so it goes to GC with the size the
                    // split added rather than to the live diff (JS
                    // `CRDTTreeNode.split`).
                    match split.removed_at() {
                        Some(removed_at) => self.pending_gc_pairs.push((
                            split.id_string(),
                            split_diff,
                            removed_at.clone(),
                        )),
                        None => add_data_size(diff, split_diff),
                    }
                    self.rebuild_node_map();
                    self.link_split_to_next(&split_path)?;
                }
            }
        }

        let parent = node_at_path(&self.root, &real_parent_path)?;
        let start = if is_left_most {
            0
        } else {
            left_path.last().map_or(0, |index| index + 1)
        };
        for (index, next) in parent.children.iter().enumerate().skip(start) {
            if !next.id().created_at().after(edited_at) {
                break;
            }
            left_path = real_parent_path.clone();
            left_path.push(index);
        }

        Ok((real_parent_path, left_path))
    }

    /// Returns the parent an insert at `from` was declared in when a merge
    /// removed it and the content lands in its merge target `parent`
    /// instead, with the merge ticket (JS `CRDTTree.edit` step 05, §9.4).
    /// The content is stamped as merged from that parent, so it stays apart
    /// from nodes that were never inside it. The ticket comes from a sibling
    /// the merge moved: the parent's own removal may have been overwritten
    /// by a later deletion.
    fn intended_merge_parent(
        &self,
        from: &TreePos,
        parent: &TreeNodeId,
    ) -> Result<Option<(TreeNodeId, Option<TimeTicket>)>> {
        let Some(declared) = self.floor_id(from.parent_id()) else {
            return Ok(None);
        };
        let declared_node = self.node(&declared)?;
        if &declared == parent
            || !declared_node.is_removed()
            || declared_node.merged_into().is_none()
            || &self.resolve_merge_target(&declared)? != parent
        {
            return Ok(None);
        }

        let merged_at = self
            .node(parent)?
            .all_children()
            .find(|child| child.merged_from() == Some(&declared) && child.merged_at().is_some())
            .and_then(|child| child.merged_at().cloned())
            .or_else(|| declared_node.removed_at().cloned());
        Ok(Some((declared, merged_at)))
    }

    /// Returns and clears the GC pairs buffered by text splits (JS
    /// `drainPendingGCPairs`).
    pub(crate) fn drain_pending_gc_pairs(&mut self) -> Vec<(String, DataSize, TimeTicket)> {
        std::mem::take(&mut self.pending_gc_pairs)
    }

    /// Points the node after a newly split node back at it (JS
    /// `CRDTTreeNode.split`), so a position at the start of that next node
    /// still resolves to the end of the split node.
    fn link_split_to_next(&mut self, split_path: &[usize]) -> Result<()> {
        let split = node_at_path(&self.root, split_path)?;
        let split_id = split.id().clone();
        let Some(next_id) = split.ins_next_id().cloned() else {
            return Ok(());
        };
        let Some(next_path) = self.floor_node_path(&next_id) else {
            return Ok(());
        };
        node_at_path_mut(&mut self.root, &next_path)?.set_ins_prev_id(Some(split_id));
        self.rebuild_node_map();
        Ok(())
    }

    /// Resolves the leftmost position of a parent a merge removed (JS
    /// `findNodesAndSplitText` step 02-1): a range boundary goes right after
    /// the parent, and an insertion goes in front of the first child the
    /// merge moved into the target.
    fn merged_parent_position(
        &self,
        parent_path: &[usize],
        merged_into: &TreeNodeId,
        range_boundary: bool,
    ) -> Result<Option<(Vec<usize>, Vec<usize>)>> {
        if range_boundary {
            return Ok(split_parent_path(parent_path)
                .map(|(grand_parent_path, _)| (grand_parent_path, parent_path.to_vec())));
        }

        let Some(target_path) = self.physical_path_by_node_id(merged_into) else {
            return Ok(None);
        };
        let target = node_at_path(&self.root, &target_path)?;
        if target.is_text() || target.is_removed() {
            return Ok(None);
        }

        let parent_id = node_at_path(&self.root, parent_path)?.id();
        let moved = target
            .all_children()
            .position(|child| child.merged_from() == Some(parent_id));
        let left_path = match moved {
            Some(index) if index > 0 => {
                let mut path = target_path.clone();
                path.push(index - 1);
                path
            }
            _ => target_path.clone(),
        };
        Ok(Some((target_path, left_path)))
    }

    /// Resolves a style range to indexes that count removed nodes, splitting
    /// text at its ends (JS `styleTargets`).
    fn style_range(
        &mut self,
        range: &(TreePos, TreePos),
        edited_at: &TimeTicket,
    ) -> Result<(usize, usize, DataSize)> {
        let mut diff = DataSize::default();
        let (from_parent, from_left) =
            self.find_nodes_and_split_text(&range.0, edited_at, true, &mut diff)?;
        let (to_parent, to_left) =
            self.find_nodes_and_split_text(&range.1, edited_at, true, &mut diff)?;
        Ok((
            physical_index_after_left_path(&self.root, &from_parent, &from_left)?,
            physical_index_after_left_path(&self.root, &to_parent, &to_left)?,
            diff,
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
        for token in collect_tokens_between(&self.root, from_idx, to_idx, true) {
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
                // JS `styleChangeRange`: the range of the node's start token,
                // whichever token reached it.
                let Ok(start) = physical_node_start(&self.root, &token.path) else {
                    continue;
                };
                let from = visible_index_of_physical(&self.root, start);
                targets.push(StyleTarget {
                    path: token.path,
                    from,
                    to: from + 1,
                    kind: token.kind,
                });
            }
        }
        targets
    }

    /// Converts the removal tokens into one deletion change per run of
    /// adjacent tokens, last run first (JS `makeDeletionChanges`). It runs
    /// before the nodes are removed, so the indexes are those the editor saw.
    fn make_deletion_changes(&self, tokens: &[TreeTokenTarget]) -> Result<Vec<TreeEditChange>> {
        let mut changes: Vec<TreeEditChange> = Vec::new();
        let mut run_start: Option<usize> = None;
        for (index, token) in tokens.iter().enumerate() {
            let start = *run_start.get_or_insert(token.index);
            let end = token.index + self.token_width(token)?;
            if tokens.get(index + 1).is_some_and(|next| next.index == end) {
                continue;
            }
            run_start = None;

            let from = visible_index_of_physical(&self.root, start);
            let to = visible_index_of_physical(&self.root, end);
            if from >= to {
                continue;
            }
            // A run that starts where the previous one ended continues it.
            if let Some(last) = changes.last_mut().filter(|last| last.to == from) {
                last.to = to;
                last.to_path = self.index_to_path(to)?;
                continue;
            }
            changes.push(TreeEditChange {
                from,
                to,
                from_path: self.index_to_path(from)?,
                to_path: self.index_to_path(to)?,
                value: None,
            });
        }
        changes.reverse();
        Ok(changes)
    }

    /// Returns how many indexes, counting removed nodes, the token spans.
    fn token_width(&self, token: &TreeTokenTarget) -> Result<usize> {
        if token.kind == TreeTokenKind::Text {
            return Ok(node_at_path(&self.root, &token.path)?.total_len());
        }
        Ok(1)
    }

    /// Collects what an edit of the range between the two indexes, which
    /// count removed nodes, removes and merges (JS `CRDTTree.edit` step 01's
    /// traversal).
    fn edit_targets(
        &self,
        from_idx: usize,
        to_idx: usize,
        edited_at: &TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<EditTargets> {
        let tokens = collect_tokens_between(&self.root, from_idx, to_idx, true);
        // The elements the range covers whole, start and end tokens both.
        let enclosed = tokens
            .iter()
            .filter(|token| token.kind == TreeTokenKind::Start && token.ended)
            .filter_map(|token| node_at_path(&self.root, &token.path).ok())
            .map(|node| node.id().clone())
            .collect::<BTreeSet<_>>();

        let mut targets = EditTargets::default();
        let mut removed = BTreeSet::new();
        let mut merged = BTreeSet::new();
        for token in tokens {
            let node = node_at_path(&self.root, &token.path)?;
            let id = node.id().clone();

            // An element whose start token alone is in the range is merged:
            // its children move to the merge target, removed ones too, so
            // they stay anchors there. An element created concurrently is
            // not, as crossing into it is an artifact of a concurrent split
            // (Fix 9).
            if token.kind == TreeTokenKind::Start
                && !token.ended
                && ticket_known(version_vector, node.id().created_at())
            {
                targets.merged.push(id.clone());
                merged.insert(id.clone());
                targets
                    .moved
                    .extend(node.all_children().map(|child| child.id().clone()));
            }

            let creation_known = ticket_known(version_vector, node.id().created_at());
            let tombstone_known = node
                .removed_at()
                .is_some_and(|removed_at| ticket_known(version_vector, removed_at));
            let parent_removed = split_parent_path(&token.path)
                .and_then(|(parent_path, _)| node_at_path(&self.root, &parent_path).ok())
                .is_some_and(|parent| {
                    removed.contains(parent.id()) && !merged.contains(parent.id())
                });
            if !(node.can_delete(edited_at, creation_known, tombstone_known) || parent_removed) {
                continue;
            }

            // An element whose end token alone is in the range stays.
            if matches!(token.kind, TreeTokenKind::Text | TreeTokenKind::Start) {
                targets.removed.push(id.clone());
                removed.insert(id.clone());

                // §4.1: the delete reaches the split siblings of the element a
                // concurrent split created.
                if !node.is_text() && node.ins_next_id().is_some() && !merged.contains(&id) {
                    for sibling in self.unknown_split_siblings_of(&id, version_vector, &enclosed)? {
                        removed.insert(sibling.clone());
                        targets.removed.push(sibling);
                    }
                }
            }
            targets.removal_tokens.push(token);
        }

        Ok(targets)
    }

    /// Returns the split siblings of the deleted element `id` the editor did
    /// not know, with their subtrees (JS `collectUnknownSplitSiblings`). The
    /// walk passes a known sibling only when the editor saw it gone: removed
    /// by a change it knew, or enclosed whole by this delete.
    fn unknown_split_siblings_of(
        &self,
        id: &TreeNodeId,
        version_vector: Option<&crate::VersionVector>,
        enclosed: &BTreeSet<TreeNodeId>,
    ) -> Result<Vec<TreeNodeId>> {
        let mut siblings = Vec::new();
        let mut walker = InsNextWalker::default();
        walker.visit(id);
        let mut next = self
            .node(id)?
            .ins_next_id()
            .and_then(|next_id| self.floor_id(next_id));
        while let Some(next_id) = next {
            if !walker.visit(&next_id) {
                break;
            }
            let node = self.node(&next_id)?;
            if ticket_known(version_vector, node.id().created_at()) {
                let seen_gone = node
                    .removed_at()
                    .is_some_and(|removed_at| ticket_known(version_vector, removed_at));
                if !seen_gone && !enclosed.contains(&next_id) {
                    break;
                }
            } else {
                siblings.push(next_id.clone());
                let mut descendants = Vec::new();
                collect_post_order_ids(node, &mut descendants);
                descendants.pop();
                siblings.extend(descendants);
            }
            next = node
                .ins_next_id()
                .and_then(|next_id| self.floor_id(next_id));
        }
        Ok(siblings)
    }

    /// Follows the merge forwarding chain from `id` while the node is a
    /// merged-away tombstone, and returns the live target, so a chained merge
    /// stays flat (JS `resolveMergeTarget`).
    fn resolve_merge_target(&self, id: &TreeNodeId) -> Result<TreeNodeId> {
        let mut target = id.clone();
        let mut seen = BTreeSet::from([target.clone()]);
        loop {
            let node = self.node(&target)?;
            if !node.is_removed() {
                break;
            }
            let Some(next) = self.merge_node_id(node.merged_into()) else {
                break;
            };
            if !seen.insert(next.clone()) {
                break;
            }
            target = next;
        }
        Ok(target)
    }

    /// Moves the children of the merged elements to the end of `dest` (JS
    /// `CRDTTree.edit` step 03), and returns the GC pairs of the subtrees the
    /// cycle guard tombstones in place instead.
    fn move_merged_children(
        &mut self,
        moved: &[TreeNodeId],
        dest: &TreeNodeId,
        edited_at: &TimeTicket,
    ) -> Result<Vec<(String, DataSize)>> {
        let mut stranded = Vec::new();
        let mut dest_tombstoned = false;
        for id in moved {
            let Some(path) = self.physical_path_by_node_id(id) else {
                continue;
            };
            let Some((parent_path, index)) = split_parent_path(&path) else {
                continue;
            };
            // A destination that is the node or lies inside it can only come
            // from a forged lineage, and moving there would close a cycle. The
            // node is tombstoned where it stands, and so is the rest once the
            // destination itself is tombstoned.
            let dest_path = self.path_of(dest)?;
            if dest_path.starts_with(&path) {
                dest_tombstoned = true;
            }
            if dest_tombstoned {
                tombstone_subtree(
                    node_at_path_mut(&mut self.root, &path)?,
                    edited_at,
                    &mut stranded,
                );
                continue;
            }

            let parent = node_at_path_mut(&mut self.root, &parent_path)?;
            let parent_id = parent.id().clone();
            let mut child = parent.children.remove(index);
            // Stamped on the first move only, so a child carried through a
            // chained merge keeps its original source and merge ticket.
            if child.merged_from.is_none() {
                child.merged_from = Some(parent_id);
                child.merged_at = Some(edited_at.clone());
            }
            let merged_from = child.merged_from.clone();
            let dest_path = self.path_of(dest)?;
            node_at_path_mut(&mut self.root, &dest_path)?.append(child);

            // Point the source at the resolved destination, as
            // `rebuild_merge_state` derives it from a snapshot.
            if let Some(source) = self.merge_node_id(merged_from.as_ref()) {
                let source = self.node_mut(&source)?;
                if source.is_removed() {
                    source.set_merged_into(Some(dest.clone()));
                }
            }
        }
        Ok(stranded)
    }

    /// Tombstones the children an earlier merge moved out of each element
    /// this edit deleted, with their descendants (JS `CRDTTree.edit` step
    /// 03-1), and returns their GC pairs.
    fn propagate_merge_deletes(
        &mut self,
        targets: &EditTargets,
        dest: &TreeNodeId,
        range: &(TreePos, TreePos),
        edited_at: &TimeTicket,
    ) -> Result<Vec<(String, DataSize)>> {
        let mut pairs = Vec::new();
        let mut declared: Option<BTreeSet<TreeNodeId>> = None;
        for id in &targets.removed {
            let Some(merged_into) = self.node(id)?.merged_into().cloned() else {
                continue;
            };
            if targets.merged.contains(id) {
                continue;
            }
            // A source whose children already sit in this edit's destination
            // stands for the merge this edit asks for only when one of its
            // positions named that source.
            if &merged_into == dest {
                let declared =
                    declared.get_or_insert_with(|| self.declared_boundaries(&[&range.0, &range.1]));
                if declared.contains(id) {
                    continue;
                }
            }
            let Some(target) = self.merge_node_id(Some(&merged_into)) else {
                continue;
            };
            for child in self.node_mut(&target)?.all_children_mut() {
                if child.merged_from() != Some(id) || child.is_removed() {
                    continue;
                }
                tombstone_subtree(child, edited_at, &mut pairs);
            }
        }
        Ok(pairs)
    }

    /// Returns the elements the positions name as their parents, with every
    /// ancestor of them, preferring the element a node was merged out of
    /// over its current parent (JS `declaredBoundaries`).
    fn declared_boundaries(&self, positions: &[&TreePos]) -> BTreeSet<TreeNodeId> {
        let mut boundaries = BTreeSet::new();
        for pos in positions {
            let mut current = self.merge_node_id(Some(pos.parent_id()));
            while let Some(id) = current {
                if !boundaries.insert(id.clone()) {
                    break;
                }
                let merged_from = self
                    .node(&id)
                    .ok()
                    .and_then(|node| self.merge_node_id(node.merged_from()));
                current = merged_from.or_else(|| self.parent_id(&id));
            }
        }
        boundaries
    }

    /// Splits `split_level` elements from the position after `left` in
    /// `parent` upward (JS `CRDTTree.edit` step 04), and returns the index
    /// and path where the first level opened, measured once it has split.
    #[allow(clippy::too_many_arguments)]
    fn split_levels(
        &mut self,
        parent: &TreeNodeId,
        left: &TreeNodeId,
        split_level: usize,
        edited_at: &TimeTicket,
        issue_split_ticket: &mut dyn FnMut() -> TimeTicket,
        version_vector: Option<&crate::VersionVector>,
        diff: &mut DataSize,
    ) -> Result<Option<(usize, Vec<usize>)>> {
        let actor_id = edited_at.actor_id().as_str().to_owned();
        let mut parent = parent.clone();
        let mut left = left.clone();
        let mut first_level = None;
        for split_count in 0..split_level {
            // §7.5: advance past the split siblings of this level the editor
            // did not know, but not past its own split products (§7.7).
            if left != parent {
                left = self.advance_past_unknown_split_siblings(
                    &left,
                    version_vector,
                    true,
                    Some(&actor_id),
                )?;
                if let Some(left_parent) = self.parent_id(&left) {
                    parent = left_parent;
                }
            }

            let Some(grand_parent) = self.parent_id(&parent) else {
                break;
            };

            let offset = if left == parent {
                0
            } else {
                self.node(&parent)?
                    .all_children()
                    .position(|child| child.id() == &left)
                    .map_or(0, |index| index + 1)
            };
            let (target, offset) =
                self.order_same_boundary_split(&parent, offset, edited_at, version_vector)?;
            let split_diff =
                self.split_element(&target, offset, issue_split_ticket, version_vector)?;
            add_data_size(diff, split_diff);

            // The split is reported at the end of what stayed in `parent`,
            // where it opened on this replica: concurrent inserts at the
            // boundary stay on its left (§7.3). A tombstone takes no room in
            // the index, so the last live child measures the same place.
            if split_count == 0 {
                let last = self
                    .node(&parent)?
                    .children()
                    .last()
                    .map_or_else(|| parent.clone(), |child| child.id().clone());
                if let Ok(index) = self.to_index(&parent, &last) {
                    if let Ok(path) = self.index_to_path(index) {
                        first_level = Some((index, path));
                    }
                }
            }

            left = parent;
            parent = grand_parent;
        }
        Ok(first_level)
    }

    /// Decides which node a split at `offset` of `parent` splits, so that
    /// concurrent splits of one node at one boundary order their products by
    /// ticket, newest first, as RGA orders concurrent inserts (JS
    /// `orderSameBoundarySplit`).
    fn order_same_boundary_split(
        &self,
        parent: &TreeNodeId,
        offset: usize,
        edited_at: &TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<(TreeNodeId, usize)> {
        // A concurrent split of the same boundary took everything right of
        // it, so only a split at the end of `parent` can be one.
        let Some(version_vector) = version_vector else {
            return Ok((parent.clone(), offset));
        };
        if offset != self.node(parent)?.all_children().count() {
            return Ok((parent.clone(), offset));
        }

        let mut target = parent.clone();
        let mut walker = InsNextWalker::default();
        walker.visit(&target);
        while let Some(ins_next_id) = self.node(&target)?.ins_next_id().cloned() {
            let Some(next_id) = self.floor_id(&ins_next_id) else {
                break;
            };
            let next = self.node(&next_id)?;
            if next.is_text() || self.parent_id(&next_id).is_none() || !walker.visit(&next_id) {
                break;
            }
            if !self.shares_split_family_parent(&target, &next_id)? {
                break;
            }
            // Splitting a tombstoned sibling would give our product a
            // tombstone a replica that applied us first never gives it.
            if next.is_removed() {
                break;
            }

            let created_at = next.id().created_at();
            if created_at.actor_id() == edited_at.actor_id()
                || version_vector
                    .get(created_at.actor_id().as_str())
                    .is_some_and(|known| known >= created_at.lamport())
                || !created_at.after(edited_at)
            {
                break;
            }
            target = next_id;
        }

        Ok(if &target == parent {
            (target, offset)
        } else {
            (target, 0)
        })
    }

    /// Reports whether `next` sits under the parent of `node`, or under a
    /// split product of that parent (JS `sharesSplitFamilyParent`).
    fn shares_split_family_parent(&self, node: &TreeNodeId, next: &TreeNodeId) -> Result<bool> {
        let (Some(node_parent), Some(next_parent)) = (self.parent_id(node), self.parent_id(next))
        else {
            return Ok(false);
        };
        if node_parent == next_parent {
            return Ok(true);
        }

        let mut walker = InsNextWalker::default();
        let mut current = Some(node_parent);
        while let Some(current_id) = current {
            if !walker.visit(&current_id) {
                break;
            }
            if current_id == next_parent {
                return Ok(true);
            }
            let Some(ins_next_id) = self.node(&current_id)?.ins_next_id() else {
                return Ok(false);
            };
            current = self.floor_id(ins_next_id);
        }
        Ok(false)
    }

    /// Splits the element `id` at the physical child `offset` into itself and
    /// a new element right after it (JS `CRDTTreeNode.split` of an element,
    /// with `splitElement`), and returns the size the split added.
    fn split_element(
        &mut self,
        id: &TreeNodeId,
        offset: usize,
        issue_split_ticket: &mut dyn FnMut() -> TimeTicket,
        version_vector: Option<&crate::VersionVector>,
    ) -> Result<DataSize> {
        let path = self.path_of(id)?;
        let Some((parent_path, index)) = split_parent_path(&path) else {
            return Err(YorkieError::InvalidTreePosition(
                "root node cannot be split".to_owned(),
            ));
        };
        let split_id = TreeNodeId::new(issue_split_ticket(), 0);
        let node = node_at_path_mut(&mut self.root, &path)?;
        let prev_size = node.data_size();
        let mut split = TreeNode::create_element(
            split_id.clone(),
            node.node_type.clone(),
            node.attrs.clone(),
            Vec::new(),
        );
        split.removed_at = node.removed_at.clone();
        // The product holds the other half of the same moved node, so it
        // carries the same merge stamp.
        split.merged_from = node.merged_from.clone();
        split.merged_at = node.merged_at.clone();

        let mut right = node.children.split_off(offset.min(node.children.len()));
        let mut left = std::mem::take(&mut node.children);

        // Fix 8: a child a merge the editor did not know moved here from a
        // sibling of this node was local to this level, so it stays left.
        if let Some(version_vector) = version_vector {
            let siblings = left
                .iter()
                .chain(right.iter())
                .map(|child| child.id().clone())
                .collect::<BTreeSet<_>>();
            let (stay, moved): (Vec<_>, Vec<_>) = right.into_iter().partition(|child| {
                match (child.merged_from(), child.merged_at()) {
                    (Some(merged_from), Some(merged_at)) => {
                        !version_vector.after_or_equal(merged_at) && siblings.contains(merged_from)
                    }
                    _ => false,
                }
            });
            left.extend(stay);
            right = moved;

            // §7.3: inserts the editor did not know, at the boundary, stay
            // left. Element split products are not inserts and are passed.
            let mut moved_to_left = Vec::new();
            let mut remaining = Vec::new();
            let mut boundary_reached = false;
            for child in right {
                if !boundary_reached {
                    if child.ins_prev_id().is_some() && !child.is_text() {
                        remaining.push(child);
                        continue;
                    }
                    let created_at = child.id().created_at();
                    let known = version_vector.get(created_at.actor_id().as_str());
                    if known.is_none_or(|known| known < created_at.lamport()) {
                        moved_to_left.push(child);
                        continue;
                    }
                }
                boundary_reached = true;
                remaining.push(child);
            }
            left.extend(moved_to_left);
            right = remaining;
        }

        node.children = left;
        split.children = right;
        let next_id = node.ins_next_id.replace(split_id.clone());
        split.ins_prev_id = Some(node.id.clone());
        split.ins_next_id = next_id.clone();
        let size = node.data_size();
        let split_size = split.data_size();
        let diff = DataSize {
            data: size.data + split_size.data - prev_size.data,
            meta: size.meta + split_size.meta - prev_size.meta,
        };
        let split_removed_at = split.removed_at.clone();
        let split_is_empty = split.children.is_empty();
        node_at_path_mut(&mut self.root, &parent_path)?.insert_at(index + 1, split);
        self.rebuild_node_map();

        // The former next split sibling now follows the product.
        if let Some(next_id) = next_id.and_then(|next_id| self.floor_id(&next_id)) {
            self.node_mut(&next_id)?
                .set_ins_prev_id(Some(split_id.clone()));

            // §7.4: an empty product goes in front of that sibling when a
            // split of the level above has already moved it to another
            // parent, unless the sibling is removed.
            let next_parent = self.parent_id(&next_id);
            if split_is_empty
                && !self.node(&next_id)?.is_removed()
                && next_parent.is_some()
                && next_parent != self.parent_id(&split_id)
            {
                let split_path = self.path_of(&split_id)?;
                let (split_parent, split_index) =
                    split_parent_path(&split_path).expect("a split product has a parent");
                let split = node_at_path_mut(&mut self.root, &split_parent)?
                    .children
                    .remove(split_index);
                let next_path = self.path_of(&next_id)?;
                let (next_parent_path, next_index) =
                    split_parent_path(&next_path).expect("checked above");
                node_at_path_mut(&mut self.root, &next_parent_path)?.insert_at(next_index, split);
            }
        }

        // A piece split off a removed node was never live, so it goes to GC
        // with the size the split added rather than to the live diff.
        if let Some(removed_at) = split_removed_at {
            self.pending_gc_pairs
                .push((split_id.to_id_string(), diff, removed_at));
            return Ok(DataSize::default());
        }
        Ok(diff)
    }

    fn physical_path_by_node_id(&self, id: &TreeNodeId) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        find_path_by_node_id(&self.root, id, &mut Vec::new(), &mut path).then_some(path)
    }

    fn floor_node_path(&self, id: &TreeNodeId) -> Option<Vec<usize>> {
        self.physical_path_by_node_id(&self.floor_id(id)?)
    }

    /// Returns the ID of the node holding `id`: the one with the greatest ID
    /// not after it from the same change (JS `findFloorNode`).
    fn floor_id(&self, id: &TreeNodeId) -> Option<TreeNodeId> {
        let floor = self.node_ids.range(..=id.clone()).next_back()?;
        floor.has_same_created_at(id).then(|| floor.clone())
    }

    /// Returns `id` when it names an element of this tree exactly (JS
    /// `findMergeNode`). Merge pointers name elements, so a floor-only match
    /// or a text node is no match.
    fn merge_node_id(&self, id: Option<&TreeNodeId>) -> Option<TreeNodeId> {
        let id = id?;
        if self.floor_id(id).as_ref() != Some(id) {
            return None;
        }
        let node = self.node(id).ok()?;
        (!node.is_text()).then(|| id.clone())
    }

    /// Returns the physical path of the node `id`.
    fn path_of(&self, id: &TreeNodeId) -> Result<Vec<usize>> {
        self.physical_path_by_node_id(id).ok_or_else(|| {
            YorkieError::InvalidTreePosition(format!("node {} not found", id.to_test_string()))
        })
    }

    fn node(&self, id: &TreeNodeId) -> Result<&TreeNode> {
        node_at_path(&self.root, &self.path_of(id)?)
    }

    fn node_mut(&mut self, id: &TreeNodeId) -> Result<&mut TreeNode> {
        let path = self.path_of(id)?;
        node_at_path_mut(&mut self.root, &path)
    }

    /// Returns the ID of the parent of `id`, or `None` for the root.
    fn parent_id(&self, id: &TreeNodeId) -> Option<TreeNodeId> {
        let path = self.physical_path_by_node_id(id)?;
        let (parent_path, _) = split_parent_path(&path)?;
        node_at_path(&self.root, &parent_path)
            .ok()
            .map(|parent| parent.id().clone())
    }

    fn id_at(&self, path: &[usize]) -> Result<TreeNodeId> {
        Ok(node_at_path(&self.root, path)?.id().clone())
    }

    /// Returns the visible index right after `left`, or at the start of the
    /// content of `parent` when the two are the same (JS `toIndex`).
    fn to_index(&self, parent: &TreeNodeId, left: &TreeNodeId) -> Result<usize> {
        Ok(visible_index_of_physical(
            &self.root,
            self.physical_index_after(parent, left)?,
        ))
    }

    /// Returns the index, counting removed nodes, right after `left`, or at
    /// the start of the content of `parent` when the two are the same.
    fn physical_index_after(&self, parent: &TreeNodeId, left: &TreeNodeId) -> Result<usize> {
        physical_index_after_left_path(&self.root, &self.path_of(parent)?, &self.path_of(left)?)
    }

    /// Follows the split chain of `node` past the element split siblings the
    /// editor did not know (JS `advancePastUnknownSplitSiblings`).
    fn advance_past_unknown_split_siblings(
        &self,
        node: &TreeNodeId,
        version_vector: Option<&crate::VersionVector>,
        relax_parent_check: bool,
        skip_actor_id: Option<&str>,
    ) -> Result<TreeNodeId> {
        let Some(version_vector) = version_vector else {
            return Ok(node.clone());
        };

        let mut current = node.clone();
        let mut walker = InsNextWalker::default();
        walker.visit(&current);
        while let Some(ins_next_id) = self.node(&current)?.ins_next_id().cloned() {
            let Some(next_id) = self.floor_id(&ins_next_id) else {
                break;
            };
            let next = self.node(&next_id)?;
            if next.is_text() || !walker.visit(&next_id) {
                break;
            }

            // §7.5: a split of the level above may have moved the sibling to
            // another parent.
            if !relax_parent_check && self.parent_id(&next_id) != self.parent_id(&current) {
                break;
            }

            // §7.7: our own split products are not concurrent ones.
            let created_at = next.id().created_at();
            if skip_actor_id.is_some_and(|skip| skip == created_at.actor_id().as_str()) {
                break;
            }
            if version_vector
                .get(created_at.actor_id().as_str())
                .is_some_and(|known| known >= created_at.lamport())
            {
                break;
            }

            // Empty unknown siblings right before our own product are
            // concurrent splits of the same boundary ordered ahead of ours.
            if let Some(skip) = skip_actor_id {
                if self.empty_run_reaches_actor(&next_id, skip, version_vector)? {
                    break;
                }
            }

            current = next_id;
        }

        Ok(current)
    }

    /// Reports whether the split chain from `node` runs through empty element
    /// split siblings the editor did not know only, and then reaches a node
    /// `actor_id` created (JS `emptyRunReachesActor`).
    fn empty_run_reaches_actor(
        &self,
        node: &TreeNodeId,
        actor_id: &str,
        version_vector: &crate::VersionVector,
    ) -> Result<bool> {
        let mut walker = InsNextWalker::default();
        let mut current = Some(node.clone());
        while let Some(current_id) = current {
            let current_node = self.node(&current_id)?;
            if current_node.is_text() || !walker.visit(&current_id) {
                break;
            }
            let created_at = current_node.id().created_at();
            if created_at.actor_id().as_str() == actor_id {
                return Ok(&current_id != node);
            }
            if current_node.all_children().next().is_some()
                || version_vector
                    .get(created_at.actor_id().as_str())
                    .is_some_and(|known| known >= created_at.lamport())
            {
                return Ok(false);
            }
            current = current_node
                .ins_next_id()
                .and_then(|next_id| self.floor_id(next_id));
        }
        Ok(false)
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
        let mut ids = BTreeSet::new();
        collect_node_ids(&self.root, &mut ids);
        self.node_ids = ids;
    }

    /// Rebuilds the `merged_into` cache of merge sources from the
    /// `merged_from` their moved children persist, for a tree loaded from a
    /// snapshot (JS `rebuildMergeState`). A child under a text node, or one
    /// naming a live element, is no merge's: a merge removes its source.
    fn rebuild_merge_state(&mut self) {
        let mut moved = Vec::new();
        collect_merged_children(&self.root, &mut Vec::new(), &mut moved);
        for (child_path, merged_from) in moved {
            let Some(source) = self.merge_node_id(Some(&merged_from)) else {
                continue;
            };
            let Some((parent_path, _)) = split_parent_path(&child_path) else {
                continue;
            };
            let Ok(parent) = node_at_path(&self.root, &parent_path) else {
                continue;
            };
            if parent.is_text() {
                continue;
            }
            let parent_id = parent.id().clone();
            let Some(removed_at) = self
                .node(&source)
                .ok()
                .and_then(|node| node.removed_at().cloned())
            else {
                continue;
            };

            // Snapshots written before `merged_at` existed fall back to the
            // removal of the source.
            if let Ok(child) = node_at_path_mut(&mut self.root, &child_path) {
                if child.merged_at.is_none() {
                    child.merged_at = Some(removed_at);
                }
            }
            if let Ok(source) = self.node_mut(&source) {
                if source.merged_into.is_none() {
                    source.merged_into = Some(parent_id);
                }
            }
        }
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
    /// The nodes the edit removes, in traversal order (JS
    /// `nodesToBeRemoved`).
    removed: Vec<TreeNodeId>,
    /// The elements the edit merges (JS `toBeMergedNodes`).
    merged: Vec<TreeNodeId>,
    /// The children of `merged`, removed ones too (JS
    /// `toBeMovedToFromParents`).
    moved: Vec<TreeNodeId>,
    /// The tokens of the nodes the edit removes, start and end tokens both,
    /// in document order (JS `tokensToBeRemoved`).
    removal_tokens: Vec<TreeTokenTarget>,
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

fn collect_node_ids(node: &TreeNode, ids: &mut BTreeSet<TreeNodeId>) {
    ids.insert(node.id().clone());
    for child in node.all_children() {
        collect_node_ids(child, ids);
    }
}

/// Adds the inserted nodes to the changes of an edit (JS `CRDTTree.edit`
/// step 05): onto the deletion change that starts at the insertion index, or
/// as a change of its own.
fn attach_insertion_change(
    mut changes: Vec<TreeEditChange>,
    from: usize,
    from_path: Vec<usize>,
    value: Option<Vec<TreeNodeInput>>,
) -> Vec<TreeEditChange> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return changes;
    };

    match changes.last_mut().filter(|last| last.from == from) {
        Some(last) => last.value = Some(value),
        None => changes.push(TreeEditChange {
            from,
            to: from,
            from_path: from_path.clone(),
            to_path: from_path,
            value: Some(value),
        }),
    }
    changes
}

/// Converts an index that counts removed nodes to the visible index at the
/// same place. A place inside a removed node maps to where the node starts.
fn visible_index_of_physical(node: &TreeNode, physical: usize) -> usize {
    let mut pos = 0;
    let mut visible = 0;
    for child in node.all_children() {
        if physical <= pos {
            break;
        }
        let size = child.padded_size(true);
        if physical >= pos + size {
            if !child.is_removed() {
                visible += child.padded_size(false);
            }
            pos += size;
            continue;
        }

        if child.is_removed() {
            return visible;
        }
        if child.is_text() {
            return visible + physical - pos;
        }
        // Past the start token of the element.
        return visible + 1 + visible_index_of_physical(child, physical - pos - 1);
    }
    visible
}

/// Returns the index, counting removed nodes, of the boundary right after
/// `left_path`, or at the start of the content of `parent_path` when the two
/// are the same. Both are physical paths.
fn physical_index_after_left_path(
    root: &TreeNode,
    parent_path: &[usize],
    left_path: &[usize],
) -> Result<usize> {
    if parent_path == left_path {
        if parent_path.is_empty() {
            return Ok(0);
        }
        // Past the start token of the parent.
        return Ok(physical_node_start(root, parent_path)? + 1);
    }

    let left = node_at_path(root, left_path)?;
    Ok(physical_node_start(root, left_path)? + left.padded_size(true))
}

/// Returns the index, counting removed nodes, of the first token of the node
/// at the physical `path`.
fn physical_node_start(root: &TreeNode, path: &[usize]) -> Result<usize> {
    let mut index = 0;
    let mut node = root;
    for (depth, &offset) in path.iter().enumerate() {
        if depth > 0 {
            // The start token of `node`, which the root does not have.
            index += 1;
        }
        let Some(siblings) = node.children.get(..offset) else {
            return Err(YorkieError::InvalidTreePosition(
                "path is out of range".to_owned(),
            ));
        };
        index += siblings
            .iter()
            .map(|sibling| sibling.padded_size(true))
            .sum::<usize>();
        node = &node.children[offset];
    }
    Ok(index)
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

/// Reports whether the change holding `version_vector` knew `ticket` (JS
/// `ticketKnown`). No vector, or an empty one, is a local change.
fn ticket_known(version_vector: Option<&crate::VersionVector>, ticket: &TimeTicket) -> bool {
    match version_vector {
        Some(vector) if !vector.is_empty() => vector.after_or_equal(ticket),
        _ => true,
    }
}

/// Tombstones `node` and every live node below it with `edited_at`, and
/// pushes the GC pair of each node it tombstoned, in post-order (JS
/// `tombstoneSubtree`).
fn tombstone_subtree(
    node: &mut TreeNode,
    edited_at: &TimeTicket,
    pairs: &mut Vec<(String, DataSize)>,
) {
    for child in node.all_children_mut() {
        tombstone_subtree(child, edited_at, pairs);
    }
    if node.removed_at.is_none() && node.remove(edited_at.clone()) {
        pairs.push((node.id_string(), node.data_size()));
    }
}

/// Collects the IDs of the subtree of `node` in post-order, `node` last.
fn collect_post_order_ids(node: &TreeNode, ids: &mut Vec<TreeNodeId>) {
    for child in node.all_children() {
        collect_post_order_ids(child, ids);
    }
    ids.push(node.id().clone());
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

/// Collects the physical path and `merged_from` of every node a merge moved,
/// in post-order (JS `traverseAll`).
fn collect_merged_children(
    node: &TreeNode,
    path: &mut Vec<usize>,
    moved: &mut Vec<(Vec<usize>, TreeNodeId)>,
) {
    for (index, child) in node.all_children().enumerate() {
        path.push(index);
        collect_merged_children(child, path, moved);
        path.pop();
    }
    if let (Some(merged_from), false) = (node.merged_from(), path.is_empty()) {
        moved.push((path.clone(), merged_from.clone()));
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

/// Issues the tickets of the elements a split creates when the operation
/// carries none (JS `TreeEditOperation.execute` fallback): the tickets of the
/// same change that follow the edit and its top-level contents.
pub(crate) fn fallback_split_tickets(
    edited_at: &TimeTicket,
    contents_len: Option<usize>,
) -> impl FnMut() -> TimeTicket {
    let lamport = edited_at.lamport();
    let actor_id = edited_at.actor_id().clone();
    let mut delimiter = edited_at
        .delimiter()
        .saturating_add(contents_len.unwrap_or_default() as u32);
    move || {
        delimiter = delimiter.saturating_add(1);
        TimeTicket::new(lamport, delimiter, actor_id.clone())
    }
}

/// Bounds a walk of a split chain: `ins_next_id` arrives from peers, so a
/// chain that loops back on itself must not spin forever (JS
/// `InsNextWalker`).
#[derive(Default)]
struct InsNextWalker {
    seen: BTreeSet<TreeNodeId>,
}

impl InsNextWalker {
    /// Records `id` and reports whether this walk had not passed it yet.
    fn visit(&mut self, id: &TreeNodeId) -> bool {
        self.seen.insert(id.clone())
    }
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
    use crate::{TimeTicket, VersionVector, TIME_TICKET_SIZE};
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
            Some(vec![crate::TreeNodeInput::element(
                "p",
                vec![crate::TreeNodeInput::text("hello")]
            )]),
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

    /// JS `tree_test.ts` "Can edit its content": a deletion range is walked
    /// over tombstones as well, so a tombstone in front of the range must not
    /// shift it onto the node before.
    #[test]
    fn deletes_the_visible_range_when_a_tombstone_precedes_it() -> crate::Result<()> {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    None,
                    vec![
                        TreeNode::create_text(node_id(3, 0), "a"),
                        TreeNode::create_text(node_id(4, 0), "X"),
                        TreeNode::create_text(node_id(5, 0), "b"),
                    ],
                )],
            ),
            ticket(1, "a"),
        );
        // <p>a X b</p>: remove "a", leaving a tombstone in front of "X".
        let range = (tree.find_pos(1, true)?, tree.find_pos(2, true)?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(6, "a"), None)?;
        assert_eq!("<root><p>Xb</p></root>", tree.to_xml());

        let range = (tree.find_pos(1, true)?, tree.find_pos(2, true)?);
        let result = tree.edit_by_range_with_changes(range, None, 0, ticket(7, "a"), None)?;

        assert_eq!("<root><p>b</p></root>", tree.to_xml());
        assert_eq!((1, 2), (result.from_idx, result.to_idx));
        assert_eq!(1, result.removed_nodes.len());
        assert_eq!("X", result.removed_nodes[0].value());
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
        // JS reports a split as an empty change where it opened.
        assert_eq!(vec![(6, 6, false)], change_ranges(&result));
        assert_eq!(vec![0, 5], result.changes[0].from_path);
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
    fn applies_concurrent_splits_at_the_same_position() -> crate::Result<()> {
        let mut tree = text_tree("ab");
        let pos = tree.find_pos(2, true)?;

        tree.edit_by_range_with_changes(
            (pos.clone(), pos.clone()),
            None,
            1,
            ticket(10, "b"),
            None,
        )?;
        assert_eq!(r#"<root><p>a</p><p>b</p></root>"#, tree.to_xml());

        let mut version_vector = VersionVector::new();
        version_vector.set("a", 10);
        tree.edit_by_range_with_changes(
            (pos.clone(), pos),
            None,
            1,
            ticket(20, "c"),
            Some(&version_vector),
        )?;

        assert_eq!(r#"<root><p>a</p><p></p><p>b</p></root>"#, tree.to_xml());
        Ok(())
    }

    #[test]
    fn applies_concurrent_splits_at_different_positions() -> crate::Result<()> {
        let mut tree = text_tree("abc");
        let first_split = tree.find_pos(2, true)?;
        let second_split = tree.find_pos(3, true)?;

        tree.edit_by_range_with_changes(
            (first_split.clone(), first_split),
            None,
            1,
            ticket(10, "b"),
            None,
        )?;
        assert_eq!(r#"<root><p>a</p><p>bc</p></root>"#, tree.to_xml());

        let mut version_vector = VersionVector::new();
        version_vector.set("a", 10);
        tree.edit_by_range_with_changes(
            (second_split.clone(), second_split),
            None,
            1,
            ticket(20, "c"),
            Some(&version_vector),
        )?;

        assert_eq!(r#"<root><p>a</p><p>b</p><p>c</p></root>"#, tree.to_xml());
        Ok(())
    }

    #[test]
    fn applies_concurrent_splits_at_different_levels() -> crate::Result<()> {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    None,
                    vec![
                        TreeNode::create_element(
                            node_id(3, 0),
                            "p",
                            None,
                            vec![TreeNode::create_text(node_id(4, 0), "ab")],
                        ),
                        TreeNode::create_element(
                            node_id(5, 0),
                            "p",
                            None,
                            vec![TreeNode::create_text(node_id(6, 0), "c")],
                        ),
                    ],
                )],
            ),
            ticket(1, "a"),
        );
        let inner_split = tree.find_pos(3, true)?;
        let outer_split = tree.find_pos(5, true)?;

        tree.edit_by_range_with_changes(
            (inner_split.clone(), inner_split),
            None,
            1,
            ticket(10, "b"),
            None,
        )?;
        assert_eq!(
            r#"<root><p><p>a</p><p>b</p><p>c</p></p></root>"#,
            tree.to_xml()
        );

        let mut version_vector = VersionVector::new();
        version_vector.set("a", 10);
        tree.edit_by_range_with_changes(
            (outer_split.clone(), outer_split),
            None,
            1,
            ticket(20, "c"),
            Some(&version_vector),
        )?;

        assert_eq!(
            r#"<root><p><p>a</p><p>b</p></p><p><p>c</p></p></root>"#,
            tree.to_xml()
        );
        Ok(())
    }

    #[test]
    fn applies_concurrent_inserts_around_split_positions() -> crate::Result<()> {
        assert_concurrent_split_insert(2, r#"<root><p>ac</p><p>b</p></root>"#)?;
        assert_concurrent_split_insert(1, r#"<root><p>ca</p><p>b</p></root>"#)?;
        assert_concurrent_split_insert(3, r#"<root><p>a</p><p>bc</p></root>"#)?;
        Ok(())
    }

    #[test]
    fn applies_concurrent_delete_inside_split_node() -> crate::Result<()> {
        let mut tree = text_tree("ab");
        let split_pos = tree.find_pos(2, true)?;
        let delete_range = (tree.find_pos(2, true)?, tree.find_pos(3, true)?);

        tree.edit_by_range_with_changes(
            (split_pos.clone(), split_pos),
            None,
            1,
            ticket(10, "b"),
            None,
        )?;
        let mut version_vector = VersionVector::new();
        version_vector.set("a", 10);
        tree.edit_by_range_with_changes(
            delete_range,
            None,
            0,
            ticket(20, "c"),
            Some(&version_vector),
        )?;

        assert_eq!(r#"<root><p>a</p><p></p></root>"#, tree.to_xml());
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

    fn assert_concurrent_split_insert(insert_index: usize, expected: &str) -> crate::Result<()> {
        let mut tree = text_tree("ab");
        let split_pos = tree.find_pos(2, true)?;
        let insert_pos = tree.find_pos(insert_index, true)?;

        tree.edit_by_range_with_changes(
            (split_pos.clone(), split_pos),
            None,
            1,
            ticket(10, "b"),
            None,
        )?;
        let mut version_vector = VersionVector::new();
        version_vector.set("a", 10);
        tree.edit_by_range_with_changes(
            (insert_pos.clone(), insert_pos),
            Some(vec![TreeNode::create_text(node_id(20, 0), "c")]),
            0,
            ticket(20, "c"),
            Some(&version_vector),
        )?;

        assert_eq!(expected, tree.to_xml());
        Ok(())
    }

    /// The `(from, to, value)` of each edit change, as JS tests compare them.
    fn change_ranges(result: &super::TreeEditResult) -> Vec<(usize, usize, bool)> {
        result
            .changes
            .iter()
            .map(|change| (change.from, change.to, change.value.is_some()))
            .collect()
    }

    fn known(actor_id: &str, lamport: i64) -> VersionVector {
        let mut version_vector = VersionVector::new();
        version_vector.set(actor_id, lamport);
        version_vector
    }

    /// JS `tree_test.ts` "TreeChange: Concurrent delete and insert": each
    /// replica resolves the other's position by node ID, tombstones included.
    #[test]
    fn resolves_positions_anchored_on_concurrently_removed_nodes() -> crate::Result<()> {
        let origin = text_tree("ab");
        let delete = (origin.find_pos(1, true)?, origin.find_pos(3, true)?);
        let insert = origin.find_pos(2, true)?;
        let c = || {
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "b"), 0),
                "c",
            )])
        };

        // The deletion first, then the insertion anchored on the removed "a".
        let mut tree = origin.clone();
        tree.edit_by_range_with_changes(delete.clone(), None, 0, ticket(10, "a"), None)?;
        let result = tree.edit_by_range_with_changes(
            (insert.clone(), insert.clone()),
            c(),
            0,
            ticket(10, "b"),
            Some(&known("a", 3)),
        )?;
        assert_eq!("<root><p>c</p></root>", tree.to_xml());
        assert_eq!(vec![(1, 1, true)], change_ranges(&result));

        // The insertion first: the deletion skips the text it did not see and
        // reports the two ranges around it, last first.
        let mut tree = origin;
        tree.edit_by_range_with_changes((insert.clone(), insert), c(), 0, ticket(10, "b"), None)?;
        assert_eq!("<root><p>acb</p></root>", tree.to_xml());
        let result = tree.edit_by_range_with_changes(
            delete,
            None,
            0,
            ticket(10, "a"),
            Some(&known("a", 3)),
        )?;
        assert_eq!("<root><p>c</p></root>", tree.to_xml());
        assert_eq!(vec![(3, 4, false), (1, 2, false)], change_ranges(&result));
        Ok(())
    }

    /// JS `tree_test.ts` "Can insert text to the same position(left)
    /// concurrently": as in RGA, the later insertion goes first whichever
    /// arrives first.
    #[test]
    fn orders_concurrent_inserts_at_the_same_position() -> crate::Result<()> {
        let origin = text_tree("12");
        let pos = origin.find_pos(1, true)?;
        let a = (
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "a"), 0),
                "A",
            )]),
            ticket(10, "a"),
        );
        let b = (
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "b"), 0),
                "B",
            )]),
            ticket(10, "b"),
        );

        for order in [[&a, &b], [&b, &a]] {
            let mut tree = origin.clone();
            for (contents, edited_at) in order {
                tree.edit_by_range_with_changes(
                    (pos.clone(), pos.clone()),
                    contents.clone(),
                    0,
                    edited_at.clone(),
                    Some(&known("a", 3)),
                )?;
            }
            assert_eq!("<root><p>BA12</p></root>", tree.to_xml());
        }
        Ok(())
    }

    /// JS `tree_test.ts` "TreeChange: Concurrent delete and insert when parent
    /// removed": the content is inserted removed, counts as garbage, and
    /// reports no change.
    #[test]
    fn inserts_into_a_removed_parent_as_removed_content() -> crate::Result<()> {
        let mut tree = text_tree("ab");
        let insert = tree.find_pos(2, true)?;
        let delete = (tree.find_pos(0, true)?, tree.find_pos(4, true)?);
        tree.edit_by_range_with_changes(delete, None, 0, ticket(10, "a"), None)?;
        assert_eq!("<root></root>", tree.to_xml());

        let result = tree.edit_by_range_with_changes(
            (insert.clone(), insert),
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "b"), 0),
                "c",
            )]),
            0,
            ticket(10, "b"),
            Some(&known("a", 3)),
        )?;

        assert_eq!("<root></root>", tree.to_xml());
        assert!(result.changes.is_empty());
        assert_eq!(0, result.inserted_size);
        // As in JS: "c", and the "b" split off the removed "ab", which was
        // never live either.
        let ids = result
            .gc_pairs
            .iter()
            .map(|(id, _, _)| id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(vec!["10:b:0:0", "3:a:0:1"], ids);
        assert_eq!(super::DataSize::default(), result.diff);
        Ok(())
    }

    /// `(from, to, from_path, to_path, has_value)` of a change.
    type ChangePath = (usize, usize, Vec<usize>, Vec<usize>, bool);

    fn change_paths(result: &super::TreeEditResult) -> Vec<ChangePath> {
        result
            .changes
            .iter()
            .map(|change| {
                (
                    change.from,
                    change.to,
                    change.from_path.clone(),
                    change.to_path.clone(),
                    change.value.is_some(),
                )
            })
            .collect()
    }

    /// JS `CRDTTree.Split` "Can split element nodes level 1": a split
    /// reports one empty change where it opened.
    #[test]
    fn splits_element_nodes_at_level_one() -> crate::Result<()> {
        for (index, xml, path) in [
            (1, "<root><p></p><p>ab</p></root>", vec![0, 0]),
            (2, "<root><p>a</p><p>b</p></root>", vec![0, 1]),
            (3, "<root><p>ab</p><p></p></root>", vec![0, 2]),
        ] {
            let mut tree = text_tree("ab");
            let pos = tree.find_pos(index, true)?;
            let result = tree.edit_by_range_with_changes(
                (pos.clone(), pos),
                None,
                1,
                ticket(10, "a"),
                None,
            )?;
            assert_eq!(xml, tree.to_xml());
            assert_eq!(6, tree.root().len());
            assert_eq!(
                vec![(index, index, path.clone(), path, false)],
                change_paths(&result)
            );
        }
        Ok(())
    }

    /// JS `CRDTTree.Split` "Can split element nodes multi-level".
    #[test]
    fn splits_element_nodes_at_several_levels() -> crate::Result<()> {
        let bold_tree = || {
            CrdtTree::create(
                TreeNode::create_element(
                    node_id(1, 0),
                    "root",
                    None,
                    vec![TreeNode::create_element(
                        node_id(2, 0),
                        "p",
                        None,
                        vec![TreeNode::create_element(
                            node_id(3, 0),
                            "b",
                            None,
                            vec![TreeNode::create_text(node_id(4, 0), "ab")],
                        )],
                    )],
                ),
                ticket(1, "a"),
            )
        };
        for (split_level, xml) in [
            (1, "<root><p><b>a</b><b>b</b></p></root>"),
            (2, "<root><p><b>a</b></p><p><b>b</b></p></root>"),
        ] {
            let mut tree = bold_tree();
            let pos = tree.find_pos(3, true)?;
            let result = tree.edit_by_range_with_changes(
                (pos.clone(), pos),
                None,
                split_level,
                ticket(10, "a"),
                None,
            )?;
            assert_eq!(xml, tree.to_xml());
            assert_eq!(
                vec![(3, 3, vec![0, 0, 1], vec![0, 0, 1], false)],
                change_paths(&result)
            );
        }
        Ok(())
    }

    /// JS `CRDTTree.Split` "Can split and merge element nodes by edit".
    #[test]
    fn splits_and_merges_element_nodes_by_edit() -> crate::Result<()> {
        let mut tree = text_tree("abcd");
        let pos = tree.find_pos(3, true)?;
        let result =
            tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(10, "a"), None)?;
        assert_eq!("<root><p>ab</p><p>cd</p></root>", tree.to_xml());
        assert_eq!(8, tree.root().len());
        assert_eq!(
            vec![(3, 3, vec![0, 2], vec![0, 2], false)],
            change_paths(&result)
        );

        let range = (tree.find_pos(3, true)?, tree.find_pos(5, true)?);
        let result = tree.edit_by_range_with_changes(range, None, 0, ticket(11, "a"), None)?;
        assert_eq!("<root><p>abcd</p></root>", tree.to_xml());
        assert_eq!(6, tree.root().len());
        assert_eq!(
            vec![(3, 5, vec![0, 2], vec![1, 0], false)],
            change_paths(&result)
        );
        Ok(())
    }

    /// JS `CRDTTreeNode.split`: a split product sits between the node it was
    /// split off and that node's former next split sibling, so the sibling
    /// points back at the product.
    #[test]
    fn links_an_element_split_to_the_next_split_sibling() -> crate::Result<()> {
        let mut tree = text_tree("abc");
        let pos = tree.find_pos(3, true)?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(10, "a"), None)?;
        let pos = tree.find_pos(2, true)?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(11, "a"), None)?;
        assert_eq!("<root><p>a</p><p>b</p><p>c</p></root>", tree.to_xml());

        let first = TreeNodeId::new(TimeTicket::new(10, 1, "a"), 0);
        let second = TreeNodeId::new(TimeTicket::new(11, 1, "a"), 0);
        let node = |id: &TreeNodeId| tree.find_floor_node(id).unwrap().clone();
        assert_eq!(Some(&second), node(&node_id(2, 0)).ins_next_id());
        assert_eq!(Some(&node_id(2, 0)), node(&second).ins_prev_id());
        assert_eq!(Some(&first), node(&second).ins_next_id());
        assert_eq!(Some(&second), node(&first).ins_prev_id());
        Ok(())
    }

    /// JS `CRDTTree.purge`: purging a split product links the split siblings
    /// on either side of it, so walks of the split chain do not stop at it.
    #[test]
    fn links_the_split_chain_past_a_purged_split_product() -> crate::Result<()> {
        let mut tree = text_tree("abc");
        let pos = tree.find_pos(3, true)?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(10, "a"), None)?;
        let pos = tree.find_pos(2, true)?;
        tree.edit_by_range_with_changes((pos.clone(), pos), None, 1, ticket(11, "a"), None)?;
        assert_eq!("<root><p>a</p><p>b</p><p>c</p></root>", tree.to_xml());

        let range = (tree.find_pos(3, true)?, tree.find_pos(6, true)?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(12, "a"), None)?;
        assert_eq!("<root><p>a</p><p>c</p></root>", tree.to_xml());
        let middle = TreeNodeId::new(TimeTicket::new(11, 1, "a"), 0);
        assert!(tree.purge_gc_pair_by_id(&middle.to_id_string()));

        let last = TreeNodeId::new(TimeTicket::new(10, 1, "a"), 0);
        let node = |id: &TreeNodeId| tree.find_floor_node(id).unwrap().clone();
        assert_eq!(Some(&last), node(&node_id(2, 0)).ins_next_id());
        assert_eq!(Some(&node_id(2, 0)), node(&last).ins_prev_id());
        Ok(())
    }

    /// JS `tree_split_opinfo_test.ts`: text typed concurrently at a split
    /// boundary stays on the left of the split (JS `splitElement` §7.3), and
    /// the split is reported after it.
    #[test]
    fn keeps_concurrent_inserts_at_a_split_boundary_on_the_left() -> crate::Result<()> {
        let mut tree = CrdtTree::create(
            TreeNode::create_element(
                node_id(1, 0),
                "doc",
                None,
                vec![TreeNode::create_element(
                    node_id(2, 0),
                    "p",
                    None,
                    vec![TreeNode::create_element(
                        node_id(3, 0),
                        "t",
                        None,
                        vec![TreeNode::create_text(node_id(4, 0), "가나다0123")],
                    )],
                )],
            ),
            ticket(1, "a"),
        );
        let split = tree.path_to_pos(&[0, 0, 7])?;
        let four = tree.path_to_pos(&[0, 0, 7])?;
        tree.edit_by_range_with_changes(
            (four.clone(), four),
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(10, "b"), 0),
                "4",
            )]),
            0,
            ticket(10, "b"),
            None,
        )?;
        let five = tree.path_to_pos(&[0, 0, 8])?;
        tree.edit_by_range_with_changes(
            (five.clone(), five),
            Some(vec![TreeNode::create_text(
                TreeNodeId::new(ticket(11, "b"), 0),
                "5",
            )]),
            0,
            ticket(11, "b"),
            None,
        )?;

        let mut version_vector = known("a", 5);
        version_vector.set("c", 10);
        let result = tree.edit_by_range_with_changes(
            (split.clone(), split),
            None,
            1,
            ticket(10, "c"),
            Some(&version_vector),
        )?;

        assert_eq!(
            "<doc><p><t>가나다012345</t><t></t></p></doc>",
            tree.to_xml()
        );
        let change = &result.changes[0];
        assert_eq!(
            (11, 11, vec![0, 0, 9], vec![0, 0, 9]),
            (
                change.from,
                change.to,
                change.from_path.clone(),
                change.to_path.clone()
            )
        );
        Ok(())
    }

    /// JS `orderSameBoundarySplit`: concurrent splits of one node at one
    /// boundary order their products by ticket, newest first, whichever
    /// arrives first.
    #[test]
    fn orders_concurrent_splits_at_the_same_boundary() -> crate::Result<()> {
        let origin = text_tree("ab");
        let pos = origin.find_pos(3, true)?;
        let mut b_known = known("a", 5);
        b_known.set("b", 10);
        let mut c_known = known("a", 5);
        c_known.set("c", 10);
        let b = (ticket(10, "b"), b_known);
        let c = (ticket(10, "c"), c_known);

        let mut orders = Vec::new();
        for order in [[&b, &c], [&c, &b]] {
            let mut tree = origin.clone();
            for (edited_at, version_vector) in order {
                tree.edit_by_range_with_changes(
                    (pos.clone(), pos.clone()),
                    None,
                    1,
                    edited_at.clone(),
                    Some(version_vector),
                )?;
            }
            assert_eq!("<root><p>ab</p><p></p><p></p></root>", tree.to_xml());
            orders.push(
                tree.root()
                    .all_children()
                    .map(|child| child.id().to_test_string())
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(orders[0], orders[1]);
        Ok(())
    }

    /// JS `CRDTTree.Merge` "Can delete nodes between element nodes with
    /// edit".
    #[test]
    fn merges_paragraphs_by_deleting_across_them() -> crate::Result<()> {
        let mut tree = paragraphs(&["ab", "cd"]);
        let range = (tree.find_pos(2, true)?, tree.find_pos(6, true)?);
        let result = tree.edit_by_range_with_changes(range, None, 0, ticket(10, "a"), None)?;
        assert_eq!(
            vec![(2, 6, vec![0, 1], vec![1, 1], false)],
            change_paths(&result)
        );
        assert_eq!("<root><p>ad</p></root>", tree.to_xml());
        assert_eq!(4, tree.root().len());
        assert_eq!(2, tree.root().children().next().unwrap().len());

        let pos = tree.find_pos(1, true)?;
        let result = tree.edit_by_range_with_changes(
            (pos.clone(), pos),
            Some(vec![TreeNode::create_text(node_id(11, 0), "@")]),
            0,
            ticket(11, "a"),
            None,
        )?;
        assert_eq!(
            vec![(1, 1, vec![0, 0], vec![0, 0], true)],
            change_paths(&result)
        );
        assert_eq!("<root><p>@ad</p></root>", tree.to_xml());
        Ok(())
    }

    /// JS "merge moves an element tombstone with correct length accounting":
    /// a merge moves the removed children too, so they stay anchors in the
    /// merge target, and they add nothing to its visible size.
    #[test]
    fn moves_removed_children_with_a_merge() -> crate::Result<()> {
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
                        vec![
                            TreeNode::create_element(node_id(5, 0), "b", None, Vec::new()),
                            TreeNode::create_text(node_id(6, 0), "cd"),
                        ],
                    ),
                ],
            ),
            ticket(1, "a"),
        );
        let range = (tree.find_pos(2, true)?, tree.find_pos(8, true)?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(10, "a"), None)?;

        assert_eq!("<root><p>ad</p></root>", tree.to_xml());
        assert_eq!(4, tree.root().len());
        let target = tree.root().children().next().unwrap();
        assert_eq!(2, target.len());
        let moved = target
            .all_children()
            .find(|child| child.id() == &node_id(5, 0))
            .expect("the removed <b> moves with the merge");
        assert!(moved.is_removed());
        assert_eq!(Some(&node_id(4, 0)), moved.merged_from());
        Ok(())
    }

    /// JS `tree_merge_anchor_test.ts` "converges when insert is anchored in
    /// a chained merge": P3 merges into P2 while P2 merges into P1, so the
    /// children of P3 flow to P1, and an insert declared at the start of P3
    /// follows them, in every order.
    #[test]
    fn converges_when_an_insert_is_anchored_in_a_chained_merge() -> crate::Result<()> {
        let origin = paragraphs(&["ab", "cd", "ef"]);
        let edits = vec![
            RemoteEdit::new(&origin, 2, 6, None, ticket(10, "b"))?,
            RemoteEdit::new(&origin, 6, 10, None, ticket(10, "c"))?,
            RemoteEdit::new(
                &origin,
                9,
                9,
                Some(vec![TreeNode::create_element(
                    TreeNodeId::new(ticket(10, "d"), 0),
                    "x",
                    None,
                    Vec::new(),
                )]),
                ticket(10, "d"),
            )?,
        ];
        for xml in apply_in_every_order(&origin, &edits)? {
            assert_eq!("<root><p>a<x></x>f</p></root>", xml);
        }
        Ok(())
    }

    /// JS `tree_concurrent_split_merge_test.ts` "keeps the text in order
    /// when a split meets a merge": the split boundary stands and the rest
    /// merges behind it.
    #[test]
    fn converges_when_a_split_meets_a_merge() -> crate::Result<()> {
        let origin = paragraphs(&["abc", "de"]);
        let edits = vec![
            RemoteEdit::new(&origin, 4, 6, None, ticket(10, "b"))?,
            RemoteEdit::split(&origin, 2, 1, ticket(10, "c")),
        ];
        for xml in apply_in_every_order(&origin, &edits)? {
            assert_eq!("<root><p>a</p><p>bcde</p></root>", xml);
        }
        Ok(())
    }

    /// JS "resolves a range boundary right after the merge-source
    /// tombstone": a position at the start of a paragraph merged away
    /// resolves, for an insert, before the children the merge moved and, for
    /// a range, right after the removed paragraph (JS step 02-1).
    #[test]
    fn resolves_positions_in_a_merged_away_parent() -> crate::Result<()> {
        let mut tree = paragraphs(&["ab", "cd"]);
        let pos = tree.find_pos(5, true)?;
        let range = (tree.find_pos(3, true)?, tree.find_pos(5, true)?);
        tree.edit_by_range_with_changes(range, None, 0, ticket(10, "a"), None)?;
        assert_eq!("<root><p>abcd</p></root>", tree.to_xml());

        let mut index_of = |range_boundary: bool| -> crate::Result<usize> {
            let mut diff = super::DataSize::default();
            let (parent, left) =
                tree.find_nodes_and_split_text(&pos, &ticket(11, "a"), range_boundary, &mut diff)?;
            Ok(super::visible_index_of_physical(
                &tree.root,
                super::physical_index_after_left_path(&tree.root, &parent, &left)?,
            ))
        };
        assert_eq!(3, index_of(false)?);
        assert_eq!(6, index_of(true)?);
        Ok(())
    }

    /// JS "stamps an insert declared inside a merged-away parent" (§9.4):
    /// content declared in a paragraph a merge removed lands in the merge
    /// target, stamped as merged from the paragraph with the merge ticket of
    /// a moved sibling, not the later removal of the paragraph.
    #[test]
    fn stamps_an_insert_declared_inside_a_merged_away_parent() -> crate::Result<()> {
        let mut tree = paragraphs(&["ab", "cd"]);
        let pos = tree.find_pos(5, true)?;
        let declared_parent = node_id(4, 0);
        let merge_ticket = ticket(10, "a");
        let range = (tree.find_pos(3, true)?, tree.find_pos(5, true)?);
        tree.edit_by_range_with_changes(range, None, 0, merge_ticket.clone(), None)?;
        tree.node_mut(&declared_parent)?.remove(ticket(11, "a"));

        let content = TreeNodeId::new(ticket(12, "a"), 0);
        tree.edit_by_range_with_changes(
            (pos.clone(), pos),
            Some(vec![TreeNode::create_element(
                content.clone(),
                "b",
                None,
                Vec::new(),
            )]),
            0,
            ticket(12, "a"),
            None,
        )?;

        assert_eq!("<root><p>ab<b></b>cd</p></root>", tree.to_xml());
        assert_eq!(Some(node_id(2, 0)), tree.parent_id(&content));
        let inserted = tree.node(&content)?;
        assert_eq!(Some(&declared_parent), inserted.merged_from());
        assert_eq!(Some(&merge_ticket), inserted.merged_at());
        Ok(())
    }

    /// An edit made on a replica that knew the base tree only.
    struct RemoteEdit {
        range: (super::TreePos, super::TreePos),
        contents: Option<Vec<TreeNode>>,
        split_level: usize,
        edited_at: TimeTicket,
    }

    impl RemoteEdit {
        fn new(
            origin: &CrdtTree,
            from: usize,
            to: usize,
            contents: Option<Vec<TreeNode>>,
            edited_at: TimeTicket,
        ) -> crate::Result<Self> {
            Ok(Self {
                range: (origin.find_pos(from, true)?, origin.find_pos(to, true)?),
                contents,
                split_level: 0,
                edited_at,
            })
        }

        fn split(origin: &CrdtTree, at: usize, level: usize, edited_at: TimeTicket) -> Self {
            let pos = origin.find_pos(at, true).expect("a valid split position");
            Self {
                range: (pos.clone(), pos),
                contents: None,
                split_level: level,
                edited_at,
            }
        }

        fn apply(&self, tree: &mut CrdtTree) -> crate::Result<super::TreeEditResult> {
            let mut version_vector = known("a", 9);
            version_vector.set(self.edited_at.actor_id().as_str(), self.edited_at.lamport());
            tree.edit_by_range_with_changes(
                self.range.clone(),
                self.contents.clone(),
                self.split_level,
                self.edited_at.clone(),
                Some(&version_vector),
            )
        }
    }

    /// Applies `edits` to copies of `origin` in every order and returns the
    /// XML each order ends with.
    fn apply_in_every_order(origin: &CrdtTree, edits: &[RemoteEdit]) -> crate::Result<Vec<String>> {
        fn permutations(items: Vec<usize>) -> Vec<Vec<usize>> {
            if items.len() <= 1 {
                return vec![items];
            }
            let mut result = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut rest = items.clone();
                rest.remove(index);
                for mut tail in permutations(rest) {
                    tail.insert(0, *item);
                    result.push(tail);
                }
            }
            result
        }

        let mut results = Vec::new();
        for order in permutations((0..edits.len()).collect()) {
            let mut tree = origin.clone();
            for index in order {
                edits[index].apply(&mut tree)?;
            }
            results.push(tree.to_xml());
        }
        Ok(results)
    }

    /// `<root>` holding one `<p>` per value, each with one text node.
    fn paragraphs(values: &[&str]) -> CrdtTree {
        let children = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let lamport = 2 + 2 * index as i64;
                TreeNode::create_element(
                    node_id(lamport, 0),
                    "p",
                    None,
                    vec![TreeNode::create_text(node_id(lamport + 1, 0), *value)],
                )
            })
            .collect();
        CrdtTree::create(
            TreeNode::create_element(node_id(1, 0), "root", None, children),
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
