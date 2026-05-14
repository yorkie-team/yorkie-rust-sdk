use super::element::DataSize;
use super::splay::{NodeId as SplayNodeId, SplayTree, SplayValue};
use crate::{Result, TimeTicket, VersionVector, YorkieError, TIME_TICKET_SIZE};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SplitIndexValue {
    node_index: usize,
    len: usize,
}

impl SplitIndexValue {
    fn new(node_index: usize, len: usize) -> Self {
        Self { node_index, len }
    }
}

impl SplayValue for SplitIndexValue {
    fn len(&self) -> usize {
        self.len
    }

    fn to_test_string(&self) -> String {
        self.node_index.to_string()
    }
}

pub(crate) trait RgaTreeSplitValue: Clone + PartialEq {
    fn split(&mut self, offset: usize) -> Self;
    fn len(&self) -> usize;
    fn data_size(&self) -> DataSize;
    fn to_json(&self) -> String;
    fn to_test_string(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RgaTreeSplitNodeId {
    created_at: TimeTicket,
    offset: usize,
}

impl RgaTreeSplitNodeId {
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
        format!("{}:{}", self.created_at.to_test_string(), self.offset)
    }
}

impl Ord for RgaTreeSplitNodeId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.created_at
            .cmp(&other.created_at)
            .then_with(|| self.offset.cmp(&other.offset))
    }
}

impl PartialOrd for RgaTreeSplitNodeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RgaTreeSplitPos {
    id: RgaTreeSplitNodeId,
    relative_offset: usize,
}

impl RgaTreeSplitPos {
    pub(crate) fn new(id: RgaTreeSplitNodeId, relative_offset: usize) -> Self {
        Self {
            id,
            relative_offset,
        }
    }

    pub(crate) fn id(&self) -> &RgaTreeSplitNodeId {
        &self.id
    }

    pub(crate) fn relative_offset(&self) -> usize {
        self.relative_offset
    }

    pub(crate) fn absolute_id(&self) -> RgaTreeSplitNodeId {
        self.id.split(self.relative_offset)
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!("{}:{}", self.id.to_test_string(), self.relative_offset)
    }
}

pub(crate) type RgaTreeSplitPosRange = (RgaTreeSplitPos, RgaTreeSplitPos);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RgaTreeSplitNode<V>
where
    V: RgaTreeSplitValue,
{
    id: RgaTreeSplitNodeId,
    value: V,
    removed_at: Option<TimeTicket>,
    ins_prev_id: Option<RgaTreeSplitNodeId>,
    ins_next_id: Option<RgaTreeSplitNodeId>,
}

impl<V> RgaTreeSplitNode<V>
where
    V: RgaTreeSplitValue,
{
    pub(crate) fn new(id: RgaTreeSplitNodeId, value: V) -> Self {
        Self {
            id,
            value,
            removed_at: None,
            ins_prev_id: None,
            ins_next_id: None,
        }
    }

    pub(crate) fn id(&self) -> &RgaTreeSplitNodeId {
        &self.id
    }

    pub(crate) fn id_string(&self) -> String {
        self.id.to_id_string()
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        self.id.created_at()
    }

    pub(crate) fn value(&self) -> &V {
        &self.value
    }

    pub(crate) fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.removed_at.as_ref()
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    pub(crate) fn len(&self) -> usize {
        if self.is_removed() {
            return 0;
        }

        self.content_len()
    }

    pub(crate) fn content_len(&self) -> usize {
        self.value.len()
    }

    pub(crate) fn create_pos_range(&self) -> RgaTreeSplitPosRange {
        (
            RgaTreeSplitPos::new(self.id.clone(), 0),
            RgaTreeSplitPos::new(self.id.clone(), self.len()),
        )
    }

    pub(crate) fn can_style(&self, edited_at: &TimeTicket, client_lamport_at_change: i64) -> bool {
        let node_existed = self.created_at().lamport() <= client_lamport_at_change;
        node_existed
            && self
                .removed_at
                .as_ref()
                .map(|removed_at| edited_at.after(removed_at))
                .unwrap_or(true)
    }

    pub(crate) fn data_size(&self) -> DataSize {
        let mut size = self.value.data_size();
        size.meta += TIME_TICKET_SIZE;

        if self.removed_at.is_some() {
            size.meta += TIME_TICKET_SIZE;
        }

        size
    }

    pub(crate) fn to_test_string(&self) -> String {
        format!(
            "{} {}",
            self.id.to_test_string(),
            self.value.to_test_string()
        )
    }

    fn split(&mut self, offset: usize) -> Self {
        let mut node = Self::new(self.id.split(offset), self.value.split(offset));
        node.removed_at = self.removed_at.clone();
        node
    }

    fn remove(
        &mut self,
        removed_at: TimeTicket,
        creation_known: bool,
        tombstone_known: bool,
    ) -> bool {
        if !creation_known {
            return false;
        }

        if self.removed_at.is_none() {
            self.removed_at = Some(removed_at);
            return true;
        }

        if !tombstone_known
            && removed_at.after(
                self.removed_at
                    .as_ref()
                    .expect("removed_at is checked above"),
            )
        {
            self.removed_at = Some(removed_at);
        }

        false
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RgaTreeSplit<V>
where
    V: RgaTreeSplitValue,
{
    nodes: Vec<RgaTreeSplitNode<V>>,
    tree_by_index: RefCell<SplayTree<SplitIndexValue>>,
    tree_by_id: BTreeMap<RgaTreeSplitNodeId, usize>,
    splay_node_by_id: BTreeMap<RgaTreeSplitNodeId, SplayNodeId>,
}

impl<V> RgaTreeSplit<V>
where
    V: RgaTreeSplitValue,
{
    pub(crate) fn new(initial_head: RgaTreeSplitNode<V>) -> Self {
        let mut split = Self {
            nodes: vec![initial_head],
            tree_by_index: RefCell::new(SplayTree::new()),
            tree_by_id: BTreeMap::new(),
            splay_node_by_id: BTreeMap::new(),
        };
        split.rebuild_indexes();
        split
    }

    pub(crate) fn initial_head(&self) -> &RgaTreeSplitNode<V> {
        &self.nodes[0]
    }

    pub(crate) fn len(&self) -> usize {
        self.tree_by_index.borrow().len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn create_range(&self, from: usize, to: usize) -> Result<RgaTreeSplitPosRange> {
        if from > to {
            return Err(YorkieError::InvalidTextPosition(
                "from must be less than or equal to to".to_owned(),
            ));
        }

        let from_pos = self.find_node_pos(from)?;
        let to_pos = if from == to {
            from_pos.clone()
        } else {
            self.find_node_pos(to)?
        };

        Ok((from_pos, to_pos))
    }

    pub(crate) fn index_to_pos(&self, index: usize) -> Result<RgaTreeSplitPos> {
        self.find_node_pos(index)
    }

    pub(crate) fn find_indexes_from_range(
        &self,
        range: &RgaTreeSplitPosRange,
    ) -> Result<(usize, usize)> {
        Ok((
            self.pos_to_index(&range.0, false)?,
            self.pos_to_index(&range.1, true)?,
        ))
    }

    pub(crate) fn normalize_pos(&self, pos: &RgaTreeSplitPos) -> Result<RgaTreeSplitPos> {
        let mut index = self.find_floor_node(pos.id()).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                pos.id().to_test_string()
            ))
        })?;

        let mut total = pos.relative_offset();
        while index > 0 {
            index -= 1;
            total += self.nodes[index].len();
        }

        Ok(RgaTreeSplitPos::new(self.nodes[index].id().clone(), total))
    }

    pub(crate) fn refine_pos(&self, pos: &RgaTreeSplitPos) -> Result<RgaTreeSplitPos> {
        let mut index = self.find_floor_node(pos.id()).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                pos.id().to_test_string()
            ))
        })?;
        let mut offset_in_part = pos.relative_offset();
        let mut part_len = self.nodes[index].content_len();

        while offset_in_part > part_len {
            offset_in_part -= part_len;
            let Some(next_index) = self.next_index(index) else {
                return Ok(RgaTreeSplitPos::new(
                    self.nodes[index].id().clone(),
                    part_len,
                ));
            };

            index = next_index;
            part_len = self.nodes[index].len();
        }

        Ok(RgaTreeSplitPos::new(
            self.nodes[index].id().clone(),
            offset_in_part,
        ))
    }

    pub(crate) fn edit(
        &mut self,
        range: RgaTreeSplitPosRange,
        edited_at: TimeTicket,
        value: Option<V>,
        version_vector: Option<&VersionVector>,
    ) -> Result<(RgaTreeSplitPos, Vec<RgaTreeSplitNode<V>>, DataSize, Vec<V>)> {
        let mut diff = DataSize::default();

        let (to_left, diff_to, to_right) = self.find_node_with_split(&range.1, &edited_at)?;
        let to_left_id = self.nodes[to_left].id().clone();
        let to_right_id = to_right.map(|index| self.nodes[index].id().clone());
        let (from_left, diff_from, from_right) = self.find_node_with_split(&range.0, &edited_at)?;
        let to_left = self.find_node_index(&to_left_id).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!(
                "node not found for {}",
                to_left_id.to_test_string()
            ))
        })?;
        let to_right = to_right_id
            .as_ref()
            .and_then(|to_right_id| self.find_node_index(to_right_id));

        add_data_size(&mut diff, diff_to);
        add_data_size(&mut diff, diff_from);

        let nodes_to_delete = self.find_between(from_right, to_right);
        let removed_nodes = self.delete_nodes(nodes_to_delete, edited_at.clone(), version_vector);

        let caret_id = to_right
            .and_then(|index| self.nodes.get(index))
            .map(|node| node.id().clone())
            .unwrap_or_else(|| self.nodes[to_left].id().clone());
        let mut caret_pos = RgaTreeSplitPos::new(caret_id, 0);

        if let Some(value) = value.filter(|value| value.len() > 0) {
            let inserted = self.insert_after_index(
                from_left,
                RgaTreeSplitNode::new(RgaTreeSplitNodeId::new(edited_at, 0), value),
            );
            add_data_size(&mut diff, self.nodes[inserted].data_size());
            caret_pos = RgaTreeSplitPos::new(
                self.nodes[inserted].id().clone(),
                self.nodes[inserted].content_len(),
            );
        }

        let removed_values = removed_nodes
            .iter()
            .map(|node| node.value().clone())
            .collect::<Vec<_>>();

        Ok((caret_pos, removed_nodes, diff, removed_values))
    }

    pub(crate) fn find_node_with_split(
        &mut self,
        pos: &RgaTreeSplitPos,
        edited_at: &TimeTicket,
    ) -> Result<(usize, DataSize, Option<usize>)> {
        let absolute_id = pos.absolute_id();
        let mut node_index = self.find_floor_node_prefer_to_left(&absolute_id)?;
        let node_offset = self.nodes[node_index].id().offset();
        let relative_offset = absolute_id
            .offset()
            .checked_sub(node_offset)
            .ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "position {} precedes node {}",
                    absolute_id.to_test_string(),
                    self.nodes[node_index].id().to_test_string()
                ))
            })?;

        let diff = self.split_node(node_index, relative_offset)?;

        while self
            .next_index(node_index)
            .is_some_and(|next| self.nodes[next].created_at().after(edited_at))
        {
            node_index += 1;
        }

        Ok((node_index, diff, self.next_index(node_index)))
    }

    pub(crate) fn find_between(&self, from: Option<usize>, to: Option<usize>) -> Vec<usize> {
        let Some(mut index) = from else {
            return Vec::new();
        };

        let mut nodes = Vec::new();
        while index < self.nodes.len() && Some(index) != to {
            nodes.push(index);
            index += 1;
        }

        nodes
    }

    pub(crate) fn node(&self, index: usize) -> Option<&RgaTreeSplitNode<V>> {
        self.nodes.get(index)
    }

    pub(crate) fn node_mut(&mut self, index: usize) -> Option<&mut RgaTreeSplitNode<V>> {
        self.nodes.get_mut(index)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &RgaTreeSplitNode<V>> {
        self.nodes.iter().skip(1)
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut RgaTreeSplitNode<V>> {
        self.nodes.iter_mut().skip(1)
    }

    pub(crate) fn to_json(&self) -> String {
        let values = self
            .iter()
            .filter(|node| !node.is_removed())
            .map(|node| node.value().to_json())
            .collect::<Vec<_>>();

        format!("[{}]", values.join(","))
    }

    pub(crate) fn to_test_string(&self) -> String {
        self.nodes
            .iter()
            .map(|node| {
                if node.is_removed() {
                    format!("{{{}}}", node.to_test_string())
                } else {
                    format!("[{}]", node.to_test_string())
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub(crate) fn purge(&mut self, child: &RgaTreeSplitNode<V>) {
        self.purge_by_id(&child.id_string());
    }

    pub(crate) fn purge_by_id(&mut self, child_id: &str) -> bool {
        let Some(index) = self
            .nodes
            .iter()
            .position(|node| node.id_string() == child_id)
        else {
            return false;
        };

        if index == 0 {
            return false;
        }

        let child_id = self.nodes[index].id().clone();
        let ins_prev_id = self.nodes[index].ins_prev_id.clone();
        let ins_next_id = self.nodes[index].ins_next_id.clone();
        self.nodes.remove(index);

        for node in &mut self.nodes {
            if node.ins_prev_id.as_ref() == Some(&child_id) {
                node.ins_prev_id = ins_prev_id.clone();
            }

            if node.ins_next_id.as_ref() == Some(&child_id) {
                node.ins_next_id = ins_next_id.clone();
            }
        }

        self.rebuild_indexes();
        true
    }

    fn find_node_pos(&self, index: usize) -> Result<RgaTreeSplitPos> {
        let length = self.len();
        if index > length {
            return Err(YorkieError::InvalidTextPosition(format!(
                "index {index} exceeds text length {length}"
            )));
        }

        if index == 0 {
            return Ok(RgaTreeSplitPos::new(self.initial_head().id().clone(), 0));
        }

        let (node_index, offset) = {
            let mut tree = self.tree_by_index.borrow_mut();
            let (splay_node, offset) = tree.find_for_text(index)?.ok_or_else(|| {
                YorkieError::InvalidTextPosition("text index tree is empty".to_owned())
            })?;
            (tree.value(splay_node).node_index, offset)
        };
        let node = self.nodes.get(node_index).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!("node index {node_index} not found"))
        })?;

        Ok(RgaTreeSplitPos::new(node.id().clone(), offset))
    }

    fn pos_to_index(&self, pos: &RgaTreeSplitPos, prefer_to_left: bool) -> Result<usize> {
        let absolute_id = pos.absolute_id();
        let index = if prefer_to_left {
            self.find_floor_node_prefer_to_left(&absolute_id)?
        } else {
            self.find_floor_node(&absolute_id).ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "node not found for {}",
                    absolute_id.to_test_string()
                ))
            })?
        };

        let splay_node = *self
            .splay_node_by_id
            .get(self.nodes[index].id())
            .ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "index node not found for {}",
                    self.nodes[index].id().to_test_string()
                ))
            })?;
        let cursor = self
            .tree_by_index
            .borrow_mut()
            .index_of(splay_node)
            .ok_or_else(|| {
                YorkieError::InvalidTextPosition(format!(
                    "index node is detached for {}",
                    self.nodes[index].id().to_test_string()
                ))
            })?;

        let offset = if self.nodes[index].is_removed() {
            0
        } else {
            absolute_id.offset() - self.nodes[index].id().offset()
        };

        Ok(cursor + offset)
    }

    fn find_floor_node_prefer_to_left(&self, id: &RgaTreeSplitNodeId) -> Result<usize> {
        let mut index = self.find_floor_node(id).ok_or_else(|| {
            YorkieError::InvalidTextPosition(format!("node not found for {}", id.to_test_string()))
        })?;

        if id.offset() > 0 && self.nodes[index].id().offset() == id.offset() {
            if let Some(ins_prev_id) = self.nodes[index].ins_prev_id.clone() {
                if let Some(ins_prev_index) = self.find_node_index(&ins_prev_id) {
                    index = ins_prev_index;
                }
            }
        }

        Ok(index)
    }

    fn find_floor_node(&self, id: &RgaTreeSplitNodeId) -> Option<usize> {
        let (floor_id, index) = self.tree_by_id.range(..=id.clone()).next_back()?;

        if floor_id != id && !floor_id.has_same_created_at(id) {
            return None;
        }

        Some(*index)
    }

    fn find_node_index(&self, id: &RgaTreeSplitNodeId) -> Option<usize> {
        self.tree_by_id.get(id).copied()
    }

    fn split_node(&mut self, index: usize, offset: usize) -> Result<DataSize> {
        let mut diff = DataSize::default();
        let content_len = self.nodes[index].content_len();
        if offset > content_len {
            return Err(YorkieError::InvalidTextPosition(format!(
                "offset {offset} exceeds node length {content_len}"
            )));
        }

        if offset == 0 || offset == content_len {
            return Ok(diff);
        }

        let prev_size = self.nodes[index].data_size();
        let left_id = self.nodes[index].id().clone();
        let old_ins_next_id = self.nodes[index].ins_next_id.clone();
        let mut split_node = self.nodes[index].split(offset);
        let split_id = split_node.id().clone();

        split_node.ins_prev_id = Some(left_id);
        split_node.ins_next_id = old_ins_next_id.clone();
        self.nodes[index].ins_next_id = Some(split_id.clone());

        if let Some(old_ins_next_id) = old_ins_next_id {
            if let Some(old_ins_next_index) = self.find_node_index(&old_ins_next_id) {
                self.nodes[old_ins_next_index].ins_prev_id = Some(split_id);
            }
        }

        self.insert_after_index(index, split_node);

        add_data_size(&mut diff, self.nodes[index].data_size());
        add_data_size(&mut diff, self.nodes[index + 1].data_size());
        sub_data_size(&mut diff, prev_size);

        Ok(diff)
    }

    fn insert_after_index(&mut self, prev_index: usize, node: RgaTreeSplitNode<V>) -> usize {
        let insert_index = prev_index + 1;
        self.nodes.insert(insert_index, node);
        self.rebuild_indexes();
        insert_index
    }

    fn delete_nodes(
        &mut self,
        candidates: Vec<usize>,
        edited_at: TimeTicket,
        vector: Option<&VersionVector>,
    ) -> Vec<RgaTreeSplitNode<V>> {
        let is_local = vector.map(VersionVector::is_empty).unwrap_or(true);
        let mut removed_nodes = Vec::new();

        for index in candidates {
            let creation_known = if is_local {
                true
            } else {
                vector
                    .and_then(|vector| {
                        vector.get(self.nodes[index].created_at().actor_id().as_str())
                    })
                    .map(|lamport| lamport >= self.nodes[index].created_at().lamport())
                    .unwrap_or(false)
            };

            let tombstone_known = if let Some(removed_at) = self.nodes[index].removed_at() {
                if is_local {
                    true
                } else {
                    vector
                        .and_then(|vector| vector.get(removed_at.actor_id().as_str()))
                        .map(|lamport| lamport >= removed_at.lamport())
                        .unwrap_or(false)
                }
            } else {
                false
            };

            if self.nodes[index].remove(edited_at.clone(), creation_known, tombstone_known) {
                removed_nodes.push(self.nodes[index].clone());
            }
        }

        self.rebuild_indexes();
        removed_nodes
    }

    fn next_index(&self, index: usize) -> Option<usize> {
        (index + 1 < self.nodes.len()).then_some(index + 1)
    }

    fn rebuild_indexes(&mut self) {
        let mut tree_by_index = SplayTree::new();
        let mut tree_by_id = BTreeMap::new();
        let mut splay_node_by_id = BTreeMap::new();

        for (index, node) in self.nodes.iter().enumerate() {
            let splay_node = tree_by_index.insert(SplitIndexValue::new(index, node.len()));
            tree_by_id.insert(node.id().clone(), index);
            splay_node_by_id.insert(node.id().clone(), splay_node);
        }

        self.tree_by_index = RefCell::new(tree_by_index);
        self.tree_by_id = tree_by_id;
        self.splay_node_by_id = splay_node_by_id;
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
