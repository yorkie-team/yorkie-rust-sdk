use super::element::{CrdtElementMeta, DataSize};
use super::rht::{Rht, RhtNode};
use crate::json::escape_json_string;
use crate::{JsonValue, TimeTicket, TIME_TICKET_SIZE};
use std::cmp::Ordering;
use std::collections::BTreeMap;

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

    pub(crate) fn is_removed(&self) -> bool {
        self.removed_at.is_some()
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

    fn node_id(lamport: i64, offset: usize) -> TreeNodeId {
        TreeNodeId::new(ticket(lamport, "a"), offset)
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
