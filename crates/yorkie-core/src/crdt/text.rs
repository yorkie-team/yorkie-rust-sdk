use super::element::{CrdtElementMeta, DataSize};
use super::rga_tree_split::{
    RgaTreeSplit, RgaTreeSplitNode, RgaTreeSplitNodeId, RgaTreeSplitPos, RgaTreeSplitPosRange,
    RgaTreeSplitValue,
};
use super::rht::{Rht, RhtNode};
use crate::json::escape_json_string;
use crate::{JsonArray, JsonObject, JsonValue, Result, TimeTicket, VersionVector};
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

    pub(crate) fn to_json_object(&self) -> Result<JsonObject> {
        let mut object = JsonObject::new();
        let attributes = self.get_attributes();

        if !attributes.is_empty() {
            let mut attrs = JsonObject::new();
            for (key, value) in attributes {
                attrs.set_unchecked(key, attribute_value_to_json_value(&value));
            }
            object.set("attrs", attrs)?;
        }

        object.set("val", self.content.clone())?;
        Ok(object)
    }

    pub(crate) fn purge(&mut self, node: &RhtNode) {
        self.attributes.purge(node);
    }

    pub(crate) fn purge_attr_by_id(&mut self, child_id: &str) -> bool {
        self.attributes.purge_by_id(child_id)
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

    pub(crate) fn index_to_pos(&self, index: usize) -> Result<RgaTreeSplitPos> {
        self.rga_tree_split.index_to_pos(index)
    }

    pub(crate) fn find_indexes_from_range(
        &self,
        range: &RgaTreeSplitPosRange,
    ) -> Result<(usize, usize)> {
        self.rga_tree_split.find_indexes_from_range(range)
    }

    pub(crate) fn normalize_pos(&self, pos: &RgaTreeSplitPos) -> Result<RgaTreeSplitPos> {
        self.rga_tree_split.normalize_pos(pos)
    }

    pub(crate) fn refine_pos(&self, pos: &RgaTreeSplitPos) -> Result<RgaTreeSplitPos> {
        self.rga_tree_split.refine_pos(pos)
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
        self.edit_by_range(range, content, attributes, edited_at, version_vector)
    }

    pub(crate) fn edit_by_range(
        &mut self,
        range: RgaTreeSplitPosRange,
        content: impl Into<String>,
        attributes: Option<BTreeMap<String, String>>,
        edited_at: TimeTicket,
        version_vector: Option<&VersionVector>,
    ) -> Result<(Vec<RgaTreeSplitNode<TextValue>>, DataSize, Vec<TextValue>)> {
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
        let range = self.index_range_to_pos_range(from_idx, to_idx)?;
        self.set_style_by_range(range, attributes, edited_at, version_vector)
    }

    pub(crate) fn set_style_by_range(
        &mut self,
        range: RgaTreeSplitPosRange,
        attributes: BTreeMap<String, String>,
        edited_at: TimeTicket,
        version_vector: Option<&VersionVector>,
    ) -> Result<(Vec<RhtNode>, DataSize)> {
        let mut diff = DataSize::default();
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
        let range = self.index_range_to_pos_range(from_idx, to_idx)?;
        self.remove_style_by_range(range, attributes_to_remove, edited_at, version_vector)
    }

    pub(crate) fn remove_style_by_range(
        &mut self,
        range: RgaTreeSplitPosRange,
        attributes_to_remove: &[String],
        edited_at: TimeTicket,
        version_vector: Option<&VersionVector>,
    ) -> Result<(Vec<RhtNode>, DataSize)> {
        let mut diff = DataSize::default();
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

    pub(crate) fn to_json_array(&self) -> Result<JsonArray> {
        let mut array = JsonArray::new();
        for node in self.rga_tree_split.iter() {
            if node.is_removed() {
                continue;
            }

            array.push(node.value().to_json_object()?);
        }

        Ok(array)
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

    pub(crate) fn gc_pair_entries(&self) -> Vec<(String, DataSize, TimeTicket)> {
        let mut pairs = Vec::new();

        for node in self.rga_tree_split.iter() {
            if let Some(removed_at) = node.removed_at() {
                pairs.push((node.id_string(), node.data_size(), removed_at.clone()));
            }

            for attr_node in node.value().gc_nodes() {
                if let Some(removed_at) = attr_node.removed_at() {
                    pairs.push((
                        attr_node.id_string(),
                        attr_node.data_size(),
                        removed_at.clone(),
                    ));
                }
            }
        }

        pairs
    }

    pub(crate) fn purge_gc_pair_by_id(&mut self, child_id: &str) -> bool {
        if self.rga_tree_split.purge_by_id(child_id) {
            return true;
        }

        for node in self.rga_tree_split.iter_mut() {
            if node.value_mut().purge_attr_by_id(child_id) {
                return true;
            }
        }

        false
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

fn attribute_value_to_json_value(value: &str) -> JsonValue {
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

fn unescape_json_string(value: &str) -> String {
    let mut unescaped = String::new();
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            unescaped.push(ch);
            continue;
        }

        match chars.next() {
            Some('"') => unescaped.push('"'),
            Some('\\') => unescaped.push('\\'),
            Some('/') => unescaped.push('/'),
            Some('b') => unescaped.push('\u{08}'),
            Some('f') => unescaped.push('\u{0c}'),
            Some('n') => unescaped.push('\n'),
            Some('r') => unescaped.push('\r'),
            Some('t') => unescaped.push('\t'),
            Some('u') => {
                let hex = chars.by_ref().take(4).collect::<String>();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        unescaped.push(ch);
                    }
                }
            }
            Some(ch) => unescaped.push(ch),
            None => unescaped.push('\\'),
        }
    }

    unescaped
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
    use crate::{TimeTicket, VersionVector};
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
    fn converts_text_values_to_json_objects_without_document_key_validation() -> crate::Result<()> {
        let mut value = TextValue::create("H");
        value.set_attr("font.size", "12", ticket(1));

        assert_eq!(
            r#"{"attrs":{"font.size":12},"val":"H"}"#,
            value.to_json_object()?.to_sorted_json()
        );
        Ok(())
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
    fn matches_js_edit_split_positions() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        text.edit_by_index(0, 0, "ABCD", None, ticket(2), None)?;
        text.edit_by_index(1, 3, "12", None, ticket(3), None)?;

        assert_eq!(r#"[{"val":"A"},{"val":"12"},{"val":"D"}]"#, text.to_json());
        assert_eq!(
            r#"[0:00:0:0 {} ""][2:a:0:0 {} "A"][3:a:0:0 {} "12"]{2:a:0:1 {} "BC"}[2:a:0:3 {} "D"]"#,
            text.to_test_string()
        );

        let expected_positions = [
            (0, "0:00:0:0:0"),
            (1, "2:a:0:0:1"),
            (2, "3:a:0:0:1"),
            (3, "3:a:0:0:2"),
            (4, "2:a:0:3:1"),
        ];

        for (index, expected) in expected_positions {
            let range = text.index_range_to_pos_range(index, index)?;
            assert_eq!(expected, range.0.to_test_string());
        }

        Ok(())
    }

    #[test]
    fn inserts_newline_between_split_blocks_with_attributes() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        let mut attrs = BTreeMap::new();
        attrs.insert("b".to_owned(), "\"1\"".to_owned());

        text.edit_by_index(0, 0, "ABCD", Some(attrs), ticket(2), None)?;
        text.edit_by_index(3, 3, "\n", None, ticket(3), None)?;

        assert_eq!(
            r#"[{"attrs":{"b":"1"},"val":"ABC"},{"val":"\n"},{"attrs":{"b":"1"},"val":"D"}]"#,
            text.to_json()
        );
        Ok(())
    }

    #[test]
    fn handles_composition_replacements() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));

        text.edit_by_index(0, 0, "ㅎ", None, ticket(2), None)?;
        text.edit_by_index(0, 1, "하", None, ticket(3), None)?;
        text.edit_by_index(0, 1, "한", None, ticket(4), None)?;
        text.edit_by_index(0, 1, "하", None, ticket(5), None)?;
        text.edit_by_index(1, 1, "느", None, ticket(6), None)?;
        text.edit_by_index(1, 2, "늘", None, ticket(7), None)?;

        assert_eq!("하늘", text.to_string());
        assert_eq!(r#"[{"val":"하"},{"val":"늘"}]"#, text.to_json());
        Ok(())
    }

    #[test]
    fn handles_nested_deletion_scenarios_from_js_tests() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        let commands = [
            (0, 0, "ABC", "ABC"),
            (3, 3, "DEF", "ABCDEF"),
            (2, 4, "1", "AB1EF"),
            (1, 4, "2", "A2F"),
        ];

        for (idx, (from, to, content, expected)) in commands.into_iter().enumerate() {
            text.edit_by_index(from, to, content, None, ticket((idx + 2) as i64), None)?;
            assert_eq!(expected, text.to_string());
        }

        Ok(())
    }

    #[test]
    fn handles_deletion_of_last_nodes_from_js_tests() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        let commands = [
            (0, 0, "A", "A"),
            (1, 1, "B", "AB"),
            (2, 2, "C", "ABC"),
            (3, 3, "DE", "ABCDE"),
            (5, 5, "F", "ABCDEF"),
            (6, 6, "GHI", "ABCDEFGHI"),
            (9, 9, "", "ABCDEFGHI"),
            (8, 9, "", "ABCDEFGH"),
            (6, 8, "", "ABCDEF"),
            (4, 6, "", "ABCD"),
            (2, 4, "", "AB"),
            (0, 2, "", ""),
        ];

        for (idx, (from, to, content, expected)) in commands.into_iter().enumerate() {
            text.edit_by_index(from, to, content, None, ticket((idx + 2) as i64), None)?;
            assert_eq!(expected, text.to_string());
        }

        Ok(())
    }

    #[test]
    fn handles_deletion_with_removed_boundary_nodes_from_js_tests() -> crate::Result<()> {
        let mut text = CrdtText::create(ticket(1));
        let commands = [
            (0, 0, "1A1BCXEF1", "1A1BCXEF1"),
            (8, 9, "", "1A1BCXEF"),
            (2, 3, "", "1ABCXEF"),
            (0, 1, "", "ABCXEF"),
            (0, 1, "", "BCXEF"),
            (0, 1, "", "CXEF"),
            (3, 4, "", "CXE"),
            (1, 2, "", "CE"),
            (0, 2, "", ""),
        ];

        for (idx, (from, to, content, expected)) in commands.into_iter().enumerate() {
            text.edit_by_index(from, to, content, None, ticket((idx + 2) as i64), None)?;
            assert_eq!(expected, text.to_string());
        }

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
    fn applies_remote_insert_and_delete_with_original_positions() -> crate::Result<()> {
        let mut left = CrdtText::create(ticket_actor(1, "a"));
        left.edit_by_index(0, 0, "AB", None, ticket_actor(2, "a"), None)?;
        let mut right = left.deepcopy();

        let delete_range = left.index_range_to_pos_range(0, 2)?;
        let insert_range = left.index_range_to_pos_range(1, 1)?;
        let left_delete_at = ticket_actor(3, "a");
        let right_insert_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);

        left.edit_by_range(delete_range.clone(), "", None, left_delete_at.clone(), None)?;
        right.edit_by_range(
            insert_range.clone(),
            "C",
            None,
            right_insert_at.clone(),
            None,
        )?;

        left.edit_by_range(insert_range, "C", None, right_insert_at, Some(&seen_base))?;
        right.edit_by_range(delete_range, "", None, left_delete_at, Some(&seen_base))?;

        assert_eq!(r#"[{"val":"C"}]"#, left.to_json());
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
    }

    #[test]
    fn applies_concurrent_insertions_from_peritext_example() -> crate::Result<()> {
        let mut left = CrdtText::create(ticket_actor(1, "a"));
        left.edit_by_index(0, 0, "The fox jumped.", None, ticket_actor(2, "a"), None)?;
        let mut right = left.deepcopy();

        let left_range = left.index_range_to_pos_range(4, 4)?;
        let right_range = left.index_range_to_pos_range(14, 14)?;
        let left_edit_at = ticket_actor(3, "a");
        let right_edit_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);

        left.edit_by_range(
            left_range.clone(),
            "quick ",
            None,
            left_edit_at.clone(),
            None,
        )?;
        right.edit_by_range(
            right_range.clone(),
            " over the dog",
            None,
            right_edit_at.clone(),
            None,
        )?;

        left.edit_by_range(
            right_range,
            " over the dog",
            None,
            right_edit_at,
            Some(&seen_base),
        )?;
        right.edit_by_range(left_range, "quick ", None, left_edit_at, Some(&seen_base))?;

        assert_eq!(
            r#"[{"val":"The "},{"val":"quick "},{"val":"fox jumped"},{"val":" over the dog"},{"val":"."}]"#,
            left.to_json()
        );
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
    }

    #[test]
    fn keeps_concurrent_insertions_unstyled_when_format_did_not_see_them() -> crate::Result<()> {
        let mut left = CrdtText::create(ticket_actor(1, "a"));
        left.edit_by_index(0, 0, "The fox jumped.", None, ticket_actor(2, "a"), None)?;
        let mut right = left.deepcopy();

        let style_range = left.index_range_to_pos_range(0, 15)?;
        let insert_range = left.index_range_to_pos_range(4, 4)?;
        let style_at = ticket_actor(3, "a");
        let insert_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);

        let mut bold = BTreeMap::new();
        bold.insert("bold".to_owned(), "true".to_owned());
        left.set_style_by_range(style_range.clone(), bold, style_at.clone(), None)?;
        right.edit_by_range(
            insert_range.clone(),
            "brown ",
            None,
            insert_at.clone(),
            None,
        )?;

        left.edit_by_range(insert_range, "brown ", None, insert_at, Some(&seen_base))?;

        let mut bold = BTreeMap::new();
        bold.insert("bold".to_owned(), "true".to_owned());
        right.set_style_by_range(style_range, bold, style_at, Some(&seen_base))?;

        assert_eq!(
            r#"[{"attrs":{"bold":true},"val":"The "},{"val":"brown "},{"attrs":{"bold":true},"val":"fox jumped."}]"#,
            left.to_json()
        );
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
    }

    #[test]
    fn applies_overlapping_remote_styles_with_original_positions() -> crate::Result<()> {
        let mut left = CrdtText::create(ticket_actor(1, "a"));
        left.edit_by_index(0, 0, "The fox jumped.", None, ticket_actor(2, "a"), None)?;
        let mut right = left.deepcopy();

        let left_range = left.index_range_to_pos_range(0, 7)?;
        let right_range = left.index_range_to_pos_range(4, 15)?;
        let left_style_at = ticket_actor(3, "a");
        let right_style_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);

        let mut bold = BTreeMap::new();
        bold.insert("bold".to_owned(), "true".to_owned());
        left.set_style_by_range(left_range.clone(), bold, left_style_at.clone(), None)?;

        let mut italic = BTreeMap::new();
        italic.insert("italic".to_owned(), "true".to_owned());
        right.set_style_by_range(right_range.clone(), italic, right_style_at.clone(), None)?;

        let mut italic = BTreeMap::new();
        italic.insert("italic".to_owned(), "true".to_owned());
        left.set_style_by_range(right_range, italic, right_style_at, Some(&seen_base))?;

        let mut bold = BTreeMap::new();
        bold.insert("bold".to_owned(), "true".to_owned());
        right.set_style_by_range(left_range, bold, left_style_at, Some(&seen_base))?;

        assert_eq!(
            r#"[{"attrs":{"bold":true},"val":"The "},{"attrs":{"bold":true,"italic":true},"val":"fox"},{"attrs":{"italic":true},"val":" jumped."}]"#,
            left.to_json()
        );
        assert_eq!(left.to_json(), right.to_json());
        Ok(())
    }

    #[test]
    fn resolves_conflicting_overlapping_styles_by_lww() -> crate::Result<()> {
        let mut left = CrdtText::create(ticket_actor(1, "a"));
        left.edit_by_index(0, 0, "The fox jumped.", None, ticket_actor(2, "a"), None)?;
        let mut right = left.deepcopy();

        let left_range = left.index_range_to_pos_range(0, 7)?;
        let right_range = left.index_range_to_pos_range(4, 15)?;
        let left_style_at = ticket_actor(3, "a");
        let right_style_at = ticket_actor(3, "b");
        let seen_base = vector([("a", 2)]);

        let mut red = BTreeMap::new();
        red.insert("highlight".to_owned(), "\"red\"".to_owned());
        left.set_style_by_range(left_range.clone(), red, left_style_at.clone(), None)?;

        let mut blue = BTreeMap::new();
        blue.insert("highlight".to_owned(), "\"blue\"".to_owned());
        right.set_style_by_range(right_range.clone(), blue, right_style_at.clone(), None)?;

        let mut blue = BTreeMap::new();
        blue.insert("highlight".to_owned(), "\"blue\"".to_owned());
        left.set_style_by_range(right_range, blue, right_style_at, Some(&seen_base))?;

        let mut red = BTreeMap::new();
        red.insert("highlight".to_owned(), "\"red\"".to_owned());
        right.set_style_by_range(left_range, red, left_style_at, Some(&seen_base))?;

        assert_eq!(
            r#"[{"attrs":{"highlight":"red"},"val":"The "},{"attrs":{"highlight":"blue"},"val":"fox"},{"attrs":{"highlight":"blue"},"val":" jumped."}]"#,
            left.to_json()
        );
        assert_eq!(left.to_json(), right.to_json());
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

    fn ticket_actor(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }

    fn vector<const N: usize>(entries: [(&str, i64); N]) -> VersionVector {
        let mut vector = VersionVector::new();
        for (actor_id, lamport) in entries {
            vector.set(actor_id, lamport);
        }
        vector
    }
}
