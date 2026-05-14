use super::element::{CrdtElementMeta, DataSize};
use super::rga_tree_split::{
    RgaTreeSplit, RgaTreeSplitNode, RgaTreeSplitNodeId, RgaTreeSplitPosRange, RgaTreeSplitValue,
};
use super::rht::{Rht, RhtNode};
use crate::json::escape_json_string;
use crate::{Result, TimeTicket, VersionVector};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextValue {
    content: String,
    attributes: Rht,
}

impl TextValue {
    pub(crate) fn new(content: impl Into<String>, attributes: Rht) -> Self {
        Self {
            content: content.into(),
            attributes,
        }
    }

    pub(crate) fn create(content: impl Into<String>) -> Self {
        Self::new(content, Rht::new())
    }

    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    pub(crate) fn attributes(&self) -> &Rht {
        &self.attributes
    }

    pub(crate) fn attributes_mut(&mut self) -> &mut Rht {
        &mut self.attributes
    }

    pub(crate) fn set_attr(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        updated_at: TimeTicket,
    ) -> (Option<RhtNode>, Option<RhtNode>) {
        self.attributes.set(key, value, updated_at)
    }

    pub(crate) fn remove_attr(&mut self, key: &str, updated_at: TimeTicket) -> Vec<RhtNode> {
        self.attributes.remove(key, updated_at)
    }

    pub(crate) fn get_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.to_object()
    }

    pub(crate) fn purge(&mut self, node: &RhtNode) {
        self.attributes.purge(node);
    }

    pub(crate) fn gc_nodes(&self) -> Vec<RhtNode> {
        self.attributes
            .iter()
            .filter(|node| node.removed_at().is_some())
            .cloned()
            .collect()
    }
}

impl RgaTreeSplitValue for TextValue {
    fn split(&mut self, offset: usize) -> Self {
        let (left, right) = split_utf16(&self.content, offset);
        self.content = left;
        Self::new(right, self.attributes.deepcopy())
    }

    fn len(&self) -> usize {
        utf16_len(&self.content)
    }

    fn data_size(&self) -> DataSize {
        let mut size = DataSize {
            data: utf16_len(&self.content) * 2,
            meta: 0,
        };

        for node in self.attributes.iter() {
            let node_size = node.data_size();
            size.data += node_size.data;
            size.meta += node_size.meta;
        }

        size
    }

    fn to_json(&self) -> String {
        let content = escape_json_string(&self.content);
        let attributes = self.attributes.to_object();

        if attributes.is_empty() {
            return format!("{{\"val\":\"{content}\"}}");
        }

        let attrs = attributes
            .into_iter()
            .map(|(key, value)| {
                format!(
                    "\"{}\":{}",
                    escape_json_string(&key),
                    attribute_value_to_json(&value)
                )
            })
            .collect::<Vec<_>>();

        format!(
            "{{\"attrs\":{{{}}},\"val\":\"{}\"}}",
            attrs.join(","),
            content
        )
    }

    fn to_test_string(&self) -> String {
        format!(
            "{} \"{}\"",
            self.attributes.to_json(),
            escape_json_string(&self.content)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtText {
    meta: CrdtElementMeta,
    rga_tree_split: RgaTreeSplit<TextValue>,
}

impl CrdtText {
    pub(crate) fn new(created_at: TimeTicket, rga_tree_split: RgaTreeSplit<TextValue>) -> Self {
        Self {
            meta: CrdtElementMeta::new(created_at),
            rga_tree_split,
        }
    }

    pub(crate) fn create(created_at: TimeTicket) -> Self {
        Self::new(created_at, RgaTreeSplit::new(initial_text_node()))
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

    pub(crate) fn len(&self) -> usize {
        self.rga_tree_split.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rga_tree_split.is_empty()
    }

    pub(crate) fn index_range_to_pos_range(
        &self,
        from_idx: usize,
        to_idx: usize,
    ) -> Result<RgaTreeSplitPosRange> {
        self.rga_tree_split.create_range(from_idx, to_idx)
    }

    pub(crate) fn edit_by_index(
        &mut self,
        from_idx: usize,
        to_idx: usize,
        content: impl Into<String>,
        attributes: Option<BTreeMap<String, String>>,
        edited_at: TimeTicket,
        version_vector: Option<&VersionVector>,
    ) -> Result<(Vec<RgaTreeSplitNode<TextValue>>, DataSize, Vec<TextValue>)> {
        let range = self.index_range_to_pos_range(from_idx, to_idx)?;
        let content = content.into();
        let value = if content.is_empty() {
            None
        } else {
            let mut value = TextValue::create(content);
            if let Some(attributes) = attributes {
                for (key, attr_value) in attributes {
                    value.set_attr(key, attr_value, edited_at.clone());
                }
            }
            Some(value)
        };

        let (_, pairs, diff, removed_values) =
            self.rga_tree_split
                .edit(range, edited_at, value, version_vector)?;
        Ok((pairs, diff, removed_values))
    }

    pub(crate) fn set_style_by_index(
        &mut self,
        from_idx: usize,
        to_idx: usize,
        attributes: BTreeMap<String, String>,
        edited_at: TimeTicket,
        version_vector: Option<&VersionVector>,
    ) -> Result<(Vec<RhtNode>, DataSize)> {
        let mut diff = DataSize::default();
        let range = self.index_range_to_pos_range(from_idx, to_idx)?;
        let (_, diff_to, to_right) = self
            .rga_tree_split
            .find_node_with_split(&range.1, &edited_at)?;
        let to_right_id = to_right.map(|index| {
            self.rga_tree_split
                .node(index)
                .expect("node returned from rga tree split")
                .id()
                .clone()
        });
        let (_, diff_from, from_right) = self
            .rga_tree_split
            .find_node_with_split(&range.0, &edited_at)?;
        let to_right = to_right_id
            .as_ref()
            .and_then(|id| self.rga_tree_split.iter().position(|node| node.id() == id))
            .map(|position| position + 1);

        add_data_size(&mut diff, diff_to);
        add_data_size(&mut diff, diff_from);

        let candidates = self.rga_tree_split.find_between(from_right, to_right);
        let mut to_be_styled = Vec::new();
        for index in candidates {
            let node = self
                .rga_tree_split
                .node(index)
                .expect("candidate index comes from rga tree split");
            let client_lamport_at_change = version_vector
                .and_then(|vector| vector.get(node.created_at().actor_id().as_str()))
                .unwrap_or(if version_vector.is_some() {
                    0
                } else {
                    i64::MAX
                });

            if node.can_style(&edited_at, client_lamport_at_change) {
                to_be_styled.push(index);
            }
        }

        let mut gc_nodes = Vec::new();
        for index in to_be_styled {
            let node = self
                .rga_tree_split
                .node_mut(index)
                .expect("candidate index comes from rga tree split");
            if node.is_removed() {
                continue;
            }

            for (key, value) in &attributes {
                let (prev, curr) =
                    node.value_mut()
                        .set_attr(key.clone(), value.clone(), edited_at.clone());
                if let Some(prev) = prev {
                    gc_nodes.push(prev);
                }
                if let Some(curr) = curr {
                    add_data_size(&mut diff, curr.data_size());
                }
            }
        }

        Ok((gc_nodes, diff))
    }

    pub(crate) fn remove_style_by_index(
        &mut self,
        from_idx: usize,
        to_idx: usize,
        attributes_to_remove: &[String],
        edited_at: TimeTicket,
        version_vector: Option<&VersionVector>,
    ) -> Result<(Vec<RhtNode>, DataSize)> {
        let mut diff = DataSize::default();
        let range = self.index_range_to_pos_range(from_idx, to_idx)?;
        let (_, diff_to, to_right) = self
            .rga_tree_split
            .find_node_with_split(&range.1, &edited_at)?;
        let to_right_id = to_right.map(|index| {
            self.rga_tree_split
                .node(index)
                .expect("node returned from rga tree split")
                .id()
                .clone()
        });
        let (_, diff_from, from_right) = self
            .rga_tree_split
            .find_node_with_split(&range.0, &edited_at)?;
        let to_right = to_right_id
            .as_ref()
            .and_then(|id| self.rga_tree_split.iter().position(|node| node.id() == id))
            .map(|position| position + 1);

        add_data_size(&mut diff, diff_to);
        add_data_size(&mut diff, diff_from);

        let candidates = self.rga_tree_split.find_between(from_right, to_right);
        let mut to_be_styled = Vec::new();
        for index in candidates {
            let node = self
                .rga_tree_split
                .node(index)
                .expect("candidate index comes from rga tree split");
            let client_lamport_at_change = version_vector
                .and_then(|vector| vector.get(node.created_at().actor_id().as_str()))
                .unwrap_or(if version_vector.is_some() {
                    0
                } else {
                    i64::MAX
                });

            if node.can_style(&edited_at, client_lamport_at_change) {
                to_be_styled.push(index);
            }
        }

        let mut gc_nodes = Vec::new();
        for index in to_be_styled {
            let node = self
                .rga_tree_split
                .node_mut(index)
                .expect("candidate index comes from rga tree split");
            if node.is_removed() {
                continue;
            }

            for key in attributes_to_remove {
                for removed in node.value_mut().remove_attr(key, edited_at.clone()) {
                    add_data_size(&mut diff, removed.data_size());
                    gc_nodes.push(removed);
                }
            }
        }

        Ok((gc_nodes, diff))
    }

    pub(crate) fn data_size(&self) -> DataSize {
        let mut size = DataSize::default();

        for node in self.rga_tree_split.iter() {
            if node.is_removed() {
                continue;
            }

            add_data_size(&mut size, node.data_size());
        }

        size.meta += self.meta_usage();
        size
    }

    pub(crate) fn to_json(&self) -> String {
        self.rga_tree_split.to_json()
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.to_json()
    }

    pub(crate) fn to_string(&self) -> String {
        self.rga_tree_split
            .iter()
            .filter(|node| !node.is_removed())
            .map(|node| node.value().content())
            .collect::<Vec<_>>()
            .join("")
    }

    pub(crate) fn to_test_string(&self) -> String {
        self.rga_tree_split.to_test_string()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        self.clone()
    }
}

fn initial_text_node() -> RgaTreeSplitNode<TextValue> {
    RgaTreeSplitNode::new(
        RgaTreeSplitNodeId::new(TimeTicket::initial(), 0),
        TextValue::create(""),
    )
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

fn add_data_size(target: &mut DataSize, size: DataSize) {
    target.data += size.data;
    target.meta += size.meta;
}

#[cfg(test)]
mod tests {
    use super::{CrdtText, TextValue};
    use crate::crdt::rga_tree_split::RgaTreeSplitValue;
    use crate::crdt::rht::Rht;
    use crate::TimeTicket;
    use std::collections::BTreeMap;

    #[test]
    fn counts_and_splits_text_values_by_utf16_code_units() {
        let values = [
            (4, "abcd"),
            (2, "한글"),
            (8, "अनुच्छेद"),
            (12, "🌷🎁💩😜👍🏳"),
            (10, "Ĺo͂řȩm̅"),
        ];

        for (length, value) in values {
            let mut text_value = TextValue::new(value, Rht::new());
            assert_eq!(length, text_value.len());
            assert_eq!(length - 2, text_value.split(2).len());
        }
    }

    #[test]
    fn serializes_text_values_with_sorted_attributes() {
        let mut value = TextValue::create("H");
        value.set_attr("i", "true", ticket(1));
        value.set_attr("b", "\"1\"", ticket(2));

        assert_eq!(r#"{"attrs":{"b":"1","i":true},"val":"H"}"#, value.to_json());
    }

    #[test]
    fn edits_text_content_with_split_blocks() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));

        text.edit_by_index(0, 0, "Hello World", None, ticket(2), None)?;
        assert_eq!(r#"[{"val":"Hello World"}]"#, text.to_json());
        assert_eq!("Hello World", text.to_string());

        text.edit_by_index(6, 11, "Yorkie", None, ticket(3), None)?;
        assert_eq!(r#"[{"val":"Hello "},{"val":"Yorkie"}]"#, text.to_json());
        assert_eq!("Hello Yorkie", text.to_string());

        Ok(())
    }

    #[test]
    fn styles_text_ranges() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "Hello World", None, ticket(2), None)?;
        text.edit_by_index(6, 11, "Yorkie", None, ticket(3), None)?;

        let mut attrs = BTreeMap::new();
        attrs.insert("b".to_owned(), "\"1\"".to_owned());
        let (gc_nodes, _) = text.set_style_by_index(0, 1, attrs, ticket(4), None)?;

        assert!(gc_nodes.is_empty());
        assert_eq!(
            r#"[{"attrs":{"b":"1"},"val":"H"},{"val":"ello "},{"val":"Yorkie"}]"#,
            text.to_json()
        );

        Ok(())
    }

    #[test]
    fn preserves_styles_across_overlapping_edits() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "Hello world", None, ticket(2), None)?;

        let mut bold = BTreeMap::new();
        bold.insert("b".to_owned(), "\"1\"".to_owned());
        text.set_style_by_index(0, 5, bold, ticket(3), None)?;

        assert_eq!(
            r#"[{"attrs":{"b":"1"},"val":"Hello"},{"val":" world"}]"#,
            text.to_json()
        );

        let mut italic = BTreeMap::new();
        italic.insert("i".to_owned(), "\"1\"".to_owned());
        text.set_style_by_index(3, 5, italic, ticket(4), None)?;

        assert_eq!(
            r#"[{"attrs":{"b":"1"},"val":"Hel"},{"attrs":{"b":"1","i":"1"},"val":"lo"},{"val":" world"}]"#,
            text.to_json()
        );

        text.edit_by_index(5, 11, " yorkie", None, ticket(5), None)?;

        assert_eq!(
            r#"[{"attrs":{"b":"1"},"val":"Hel"},{"attrs":{"b":"1","i":"1"},"val":"lo"},{"val":" yorkie"}]"#,
            text.to_json()
        );

        let mut list = BTreeMap::new();
        list.insert("list".to_owned(), "\"true\"".to_owned());
        text.edit_by_index(5, 5, "\n", Some(list), ticket(6), None)?;

        assert_eq!(
            r#"[{"attrs":{"b":"1"},"val":"Hel"},{"attrs":{"b":"1","i":"1"},"val":"lo"},{"attrs":{"list":"true"},"val":"\n"},{"val":" yorkie"}]"#,
            text.to_json()
        );

        Ok(())
    }

    #[test]
    fn removes_text_styles_and_returns_gc_nodes() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "Hello", None, ticket(2), None)?;

        let mut attrs = BTreeMap::new();
        attrs.insert("b".to_owned(), "\"1\"".to_owned());
        text.set_style_by_index(0, 1, attrs, ticket(3), None)?;

        let (gc_nodes, _) = text.remove_style_by_index(0, 1, &["b".to_owned()], ticket(4), None)?;

        assert_eq!(1, gc_nodes.len());
        assert_eq!("b", gc_nodes[0].key());
        assert_eq!(r#"[{"val":"H"},{"val":"ello"}]"#, text.to_json());

        Ok(())
    }

    #[test]
    fn deletes_text_ranges_and_tracks_removed_nodes() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "Hello", None, ticket(2), None)?;

        let (removed_nodes, _, removed_values) =
            text.edit_by_index(1, 4, "", None, ticket(3), None)?;

        assert_eq!(1, removed_nodes.len());
        assert_eq!(1, removed_values.len());
        assert_eq!("Ho", text.to_string());
        assert_eq!(r#"[{"val":"H"},{"val":"o"}]"#, text.to_json());

        Ok(())
    }

    fn ticket(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, "a")
    }
}
