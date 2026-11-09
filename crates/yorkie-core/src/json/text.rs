//! The public text facade (JS `src/document/json/text.ts`, Go
//! `pkg/document/json/text.go`).

use super::{lock_recorder, JsonEditRecorderRef};
use crate::crdt::rga_tree_split::RgaTreeSplitPos;
use crate::crdt::text::CrdtText;
use crate::{Result, TextPosStructRange, TimeTicket, YorkieError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// A run of text that shares one set of attributes (JS `TextValueType`, an
/// entry of `Text.values()`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    pub content: String,
    /// The attributes of this run, or `None` when it has none. JS always
    /// reports an object here, empty when the run has no attributes.
    pub attributes: Option<BTreeMap<String, Value>>,
}

/// An extended data type for the contents of a text editor (JS `Text`).
///
/// Indexes count UTF-16 code units, as in JS. A `JsonText` edits only inside
/// `Document::update`: one taken from anywhere else, such as a clone of
/// `Document::get_root`, can be read but its edits fail with
/// `YorkieError::NotInitialized`, like an uninitialized JS `Text`.
#[derive(Clone)]
pub struct JsonText {
    text: CrdtText,
    recorder: Option<JsonEditRecorderRef>,
}

impl fmt::Debug for JsonText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("JsonText")
            .field(&self.text.to_json())
            .finish()
    }
}

impl Default for JsonText {
    fn default() -> Self {
        Self::new()
    }
}

/// Texts are equal when they show the same content and attributes, like the
/// other JSON values, which compare by value rather than CRDT identity.
impl PartialEq for JsonText {
    fn eq(&self, other: &Self) -> bool {
        self.text.to_json() == other.text.to_json()
    }
}

/// Writes the text content (JS `Text.toString`).
impl fmt::Display for JsonText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.text.fmt(f)
    }
}

impl JsonText {
    /// Creates an empty text to set into a document (JS `new Text()`). As in
    /// JS, setting it into a document always creates an empty text there.
    pub fn new() -> Self {
        Self {
            text: CrdtText::create(TimeTicket::initial()),
            recorder: None,
        }
    }

    /// Returns the ID of this text (JS `Text.getID`).
    pub fn id(&self) -> &TimeTicket {
        self.text.created_at()
    }

    /// Replaces the range `from..to` with `content` and returns the caret
    /// range after the edit (JS `Text.edit`). `attributes` style the inserted
    /// content.
    pub fn edit(
        &mut self,
        from: usize,
        to: usize,
        content: &str,
        attributes: Option<BTreeMap<String, Value>>,
    ) -> Result<(usize, usize)> {
        let recorder = self.recorder()?;
        let range = self.text.create_range(from, to)?;
        let attributes = attributes.map(stringify_attributes);

        let mut recorder = lock_recorder(&recorder);
        let executed_at = recorder.issue_time_ticket();
        let result = self.text.edit_by_range(
            range.clone(),
            content,
            attributes.clone(),
            executed_at.clone(),
            None,
        )?;
        recorder.record_text_edit(
            self.text.created_at(),
            range,
            content,
            attributes.unwrap_or_default(),
            executed_at,
        );

        let caret = result.caret_pos;
        self.text.find_indexes_from_range(&(caret.clone(), caret))
    }

    /// Deletes the range `from..to` (JS `Text.delete`).
    pub fn delete(&mut self, from: usize, to: usize) -> Result<(usize, usize)> {
        self.edit(from, to, "", None)
    }

    /// Deletes all content (JS `Text.empty`).
    pub fn empty(&mut self) -> Result<(usize, usize)> {
        self.edit(0, self.len(), "", None)
    }

    /// Styles the range `from..to` with `attributes` (JS `Text.setStyle`).
    pub fn set_style(
        &mut self,
        from: usize,
        to: usize,
        attributes: BTreeMap<String, Value>,
    ) -> Result<()> {
        let recorder = self.recorder()?;
        let range = self.text.create_range(from, to)?;
        let attributes = stringify_attributes(attributes);

        let mut recorder = lock_recorder(&recorder);
        let executed_at = recorder.issue_time_ticket();
        self.text.set_style_by_range(
            range.clone(),
            attributes.clone(),
            executed_at.clone(),
            None,
        )?;
        recorder.record_text_style(self.text.created_at(), range, attributes, executed_at);
        Ok(())
    }

    /// Returns the length in UTF-16 code units (JS `Text.length`).
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Converts an index range to a position range that follows concurrent
    /// edits, for keeping a selection (JS `Text.indexRangeToPosRange`, through
    /// `CRDTText.indexRangeToPosRange`). Equal indexes give the same position
    /// twice. As in JS, the indexes are not checked against surrogate pairs
    /// or against each other.
    pub fn index_range_to_pos_range(
        &self,
        (from, to): (usize, usize),
    ) -> Result<TextPosStructRange> {
        let text = self.in_document()?;
        let from_pos = text.index_to_pos(from)?.to_struct();
        if from == to {
            return Ok((from_pos.clone(), from_pos));
        }
        Ok((from_pos, text.index_to_pos(to)?.to_struct()))
    }

    /// Converts a position range back to an index range (JS
    /// `Text.posRangeToIndexRange`, through
    /// `RGATreeSplit.findIndexesFromRange`). A position whose node this text
    /// does not hold fails with an `ErrInvalidArgument` error.
    ///
    /// As in JS, `from` resolves to the right and `to` to the left, so a
    /// `from` at the end of a node split off an insert comes back at the
    /// start of the next piece, past anything inserted between them.
    pub fn pos_range_to_index_range(&self, range: &TextPosStructRange) -> Result<(usize, usize)> {
        let text = self.in_document()?;
        let range = (
            RgaTreeSplitPos::from_struct(&range.0)?,
            RgaTreeSplitPos::from_struct(&range.1)?,
        );
        text.find_indexes_from_range(&range)
    }

    /// Returns the content-attributes runs of this text (JS `Text.values`).
    pub fn values(&self) -> Vec<TextChunk> {
        self.text.values()
    }

    /// Serializes this text as JSON (JS `Text.toJSON`).
    pub fn to_sorted_json(&self) -> String {
        self.text.to_sorted_json()
    }

    pub(crate) fn from_crdt(text: &CrdtText) -> Self {
        Self {
            text: text.deepcopy(),
            recorder: None,
        }
    }

    pub(super) fn attach_recorder(&mut self, recorder: JsonEditRecorderRef) {
        self.recorder = Some(recorder);
    }

    pub(super) fn detach_recorder(&mut self) {
        self.recorder = None;
    }

    fn recorder(&self) -> Result<JsonEditRecorderRef> {
        self.recorder.clone().ok_or_else(not_initialized)
    }

    /// Returns the CRDT text for a read that needs the document's node IDs.
    /// A text made by `new` holds none: every element of a document has a
    /// ticket its change issued, never the initial one.
    fn in_document(&self) -> Result<&CrdtText> {
        if self.text.created_at() == &TimeTicket::initial() {
            return Err(not_initialized());
        }
        Ok(&self.text)
    }
}

fn not_initialized() -> YorkieError {
    YorkieError::NotInitialized("Text is not initialized yet".to_owned())
}

/// Encodes attribute values for the CRDT (JS `stringifyObjectValues`). A
/// string that is not itself a JSON document is stored as-is, which is what
/// the Go SDK stores; any other value, including a string that is a JSON
/// document, is stored JSON-encoded so it keeps its type.
pub(super) fn stringify_attributes(
    attributes: BTreeMap<String, Value>,
) -> BTreeMap<String, String> {
    attributes
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                Value::String(value) if serde_json::from_str::<Value>(&value).is_err() => value,
                value => value.to_string(),
            };
            (key, value)
        })
        .collect()
}

// Cases follow JS `test/integration/text_test.ts` (the single-document ones)
// and `test/unit/document/surrogate_index_test.ts`.
#[cfg(test)]
mod tests {
    use super::{stringify_attributes, JsonText, TextChunk};
    use crate::{
        DocEvent, Document, EditOpValue, JsonObject, JsonValue, OperationInfo, Result, YorkieError,
    };
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    fn attrs(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).expect("attributes are an object")
    }

    fn text_of(doc: &Document) -> &JsonText {
        match doc.get_root().get("k1") {
            Some(JsonValue::Text(text)) => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn handles_edit_operations() -> Result<()> {
        let mut doc = Document::new("test-doc");
        assert_eq!("{}", doc.to_sorted_json());

        doc.update(|root| {
            let text = root.set_text("k1")?;
            text.edit(0, 0, "ABCD", None)?;
            text.edit(1, 3, "12", None)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"k1":[{"val":"A"},{"val":"12"},{"val":"D"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            // JS prints `[1:00:2:0 A]`; the Rust test string also shows the
            // node attributes.
            r#"[0:00:0:0 {} ""][1:00:2:0 {} "A"][1:00:3:0 {} "12"]{1:00:2:1 {} "BC"}[1:00:2:3 {} "D"]"#,
            text_of(&doc).text.to_test_string()
        );
        Ok(())
    }

    #[test]
    fn handles_edit_operations_with_newline() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("k1")?;
            text.edit(0, 0, "ABCD", None)?;
            text.edit(3, 3, "\n", None)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"k1":[{"val":"ABC"},{"val":"\n"},{"val":"D"}]}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    #[test]
    fn handles_korean_composition() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("k1")?;
            text.edit(0, 0, "ㅎ", None)?;
            text.edit(0, 1, "하", None)?;
            text.edit(0, 1, "한", None)?;
            text.edit(0, 1, "하", None)?;
            text.edit(1, 1, "느", None)?;
            text.edit(1, 2, "늘", None)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"k1":[{"val":"하"},{"val":"늘"}]}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    /// Rebuilds the text from the edit op infos of local changes (JS test
    /// helper `TextView`), so the op infos are checked against the document.
    #[derive(Default)]
    struct TextView(Vec<u16>);

    impl TextView {
        fn apply(&mut self, operations: &[OperationInfo]) {
            for op in operations {
                if let OperationInfo::Edit {
                    from, to, value, ..
                } = op
                {
                    let content = value.content.encode_utf16().collect::<Vec<_>>();
                    self.0.splice(from..to, content);
                }
            }
        }
    }

    /// Runs `(from, to, content)` edits on `$.text` and checks after each one
    /// that the op infos rebuild the same text.
    fn assert_view_follows_edits(commands: &[(usize, usize, &str)]) -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| root.set_text("text").map(|_| ()))?;
        let view = Arc::new(Mutex::new(TextView::default()));
        let sink = view.clone();
        doc.subscribe_path("$.text", move |event| {
            if let DocEvent::LocalChange(info) = event {
                sink.lock().unwrap().apply(&info.operations);
            }
        });

        for &(from, to, content) in commands {
            doc.update(|root| {
                root.get_text_mut("text")?
                    .edit(from, to, content, None)
                    .map(|_| ())
            })?;
            let Some(JsonValue::Text(text)) = doc.get_root().get("text") else {
                panic!("text is missing");
            };
            let view = String::from_utf16(&view.lock().unwrap().0).unwrap();
            assert_eq!(
                text.to_string(),
                view,
                "after edit({from}, {to}, {content:?})"
            );
        }
        Ok(())
    }

    #[test]
    fn handles_deletion_of_nested_nodes() -> Result<()> {
        assert_view_follows_edits(&[(0, 0, "ABC"), (3, 3, "DEF"), (2, 4, "1"), (1, 4, "2")])
    }

    #[test]
    fn handles_deletion_of_the_last_nodes() -> Result<()> {
        assert_view_follows_edits(&[
            (0, 0, "A"),
            (1, 1, "B"),
            (2, 2, "C"),
            (3, 3, "DE"),
            (5, 5, "F"),
            (6, 6, "GHI"),
            (9, 9, ""), // delete no last node
            (8, 9, ""), // delete one last node with split
            (6, 8, ""), // delete one last node without split
            (4, 6, ""), // delete last nodes with split
            (2, 4, ""), // delete last nodes without split
            (0, 2, ""), // delete last nodes containing the first
        ])
    }

    #[test]
    fn handles_deletion_with_boundary_nodes_already_removed() -> Result<()> {
        assert_view_follows_edits(&[
            (0, 0, "1A1BCXEF1"),
            (8, 9, ""),
            (2, 3, ""),
            (0, 1, ""), // ABCXEF
            (0, 1, ""), // delete A with two removed boundaries
            (0, 1, ""), // delete B with removed left boundary
            (3, 4, ""), // delete F with removed right boundary
            (1, 2, ""),
            (0, 2, ""), // delete CE with removed inner node X
        ])
    }

    // An edit across a tombstone reports one op info per live run it
    // removed, the last run first, and the insertion on the first run.
    #[test]
    fn edit_reports_one_op_info_per_changed_range() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("text")?;
            text.edit(0, 0, "ABCDE", Some(attrs(json!({"b": true}))))?;
            text.edit(2, 3, "", None).map(|_| ())
        })?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        doc.subscribe_path("$.text", move |event| {
            if let DocEvent::LocalChange(info) = event {
                sink.lock().unwrap().extend(info.operations.clone());
            }
        });

        doc.update(|root| {
            root.get_text_mut("text")?
                .edit(1, 3, "X", Some(attrs(json!({"i": 1}))))
                .map(|_| ())
        })?;

        let edit = |from, to, content: &str, attributes| OperationInfo::Edit {
            path: "$.text".to_owned(),
            from,
            to,
            value: EditOpValue {
                attributes,
                content: content.to_owned(),
            },
        };
        assert_eq!(
            vec![
                edit(2, 3, "", BTreeMap::new()),
                edit(1, 2, "X", attrs(json!({"i": 1}))),
            ],
            *events.lock().unwrap()
        );
        Ok(())
    }

    #[test]
    fn handles_edit_operations_with_attributes() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("k1")?;
            text.edit(0, 0, "ABCD", Some(attrs(json!({"b": "1"}))))?;
            text.edit(3, 3, "\n", None)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"k1":[{"attrs":{"b":"1"},"val":"ABC"},{"val":"\n"},{"attrs":{"b":"1"},"val":"D"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            vec![
                TextChunk {
                    content: "ABC".to_owned(),
                    attributes: Some(attrs(json!({"b": "1"}))),
                },
                TextChunk {
                    content: "\n".to_owned(),
                    attributes: None,
                },
                TextChunk {
                    content: "D".to_owned(),
                    attributes: Some(attrs(json!({"b": "1"}))),
                },
            ],
            text_of(&doc).values()
        );
        Ok(())
    }

    #[test]
    fn handles_delete_and_empty_operations() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| root.set_text("k1")?.edit(0, 0, "ABCD", None).map(|_| ()))?;
        assert_eq!("ABCD", text_of(&doc).to_string());

        doc.update(|root| root.get_text_mut("k1")?.delete(1, 3).map(|_| ()))?;
        assert_eq!("AD", text_of(&doc).to_string());

        doc.update(|root| root.get_text_mut("k1")?.empty().map(|_| ()))?;
        assert_eq!("", text_of(&doc).to_string());
        assert!(text_of(&doc).is_empty());
        Ok(())
    }

    #[test]
    fn edit_returns_the_caret_range() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("k1")?;
            assert_eq!((4, 4), text.edit(0, 0, "ABCD", None)?);
            assert_eq!((3, 3), text.edit(1, 3, "12", None)?);
            assert_eq!((1, 1), text.delete(1, 3)?);
            Ok(())
        })?;

        assert_eq!("AD", text_of(&doc).to_string());
        Ok(())
    }

    #[test]
    fn set_style_writes_attributes_as_json() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("k1")?;
            text.edit(0, 0, "The fox jumped.", None)?;
            text.set_style(0, 7, attrs(json!({"bold": true})))?;
            text.set_style(4, 7, attrs(json!({"color": "red", "size": 12})))?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"k1":[{"attrs":{"bold":true},"val":"The "},{"attrs":{"bold":true,"color":"red","size":12},"val":"fox"},{"val":" jumped."}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            Some(attrs(json!({"bold": true, "color": "red", "size": 12}))),
            text_of(&doc).values()[1].attributes
        );
        Ok(())
    }

    // Review Focus 3: indexes count UTF-16 code units, so the emoji after two
    // Hangul syllables spans indexes 2..4.
    #[test]
    fn text_edit_uses_utf16_indexes() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let text = root.set_text("k1")?;
            text.edit(0, 0, "안녕😀", None)?;
            assert_eq!(4, text.len());
            text.edit(2, 4, "", None)?;
            Ok(())
        })?;

        assert_eq!("안녕", text_of(&doc).to_string());
        assert_eq!(2, text_of(&doc).len());
        Ok(())
    }

    // JS `surrogate_index_test.ts`: an index inside a surrogate pair is
    // rejected, for edit and for style.
    #[test]
    fn rejects_an_index_inside_a_surrogate_pair() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| root.set_text("k1")?.edit(0, 0, "a😀b", None).map(|_| ()))?;

        let split = YorkieError::InvalidTextPosition(
            "index must not split a UTF-16 surrogate pair".to_owned(),
        );
        let err = doc
            .update(|root| root.get_text_mut("k1")?.edit(2, 2, "x", None).map(|_| ()))
            .unwrap_err();
        assert_eq!(split, err);
        let err = doc
            .update(|root| {
                root.get_text_mut("k1")?
                    .set_style(0, 2, attrs(json!({"b": true})))
            })
            .unwrap_err();
        assert_eq!(split, err);

        doc.update(|root| root.get_text_mut("k1")?.edit(1, 3, "", None).map(|_| ()))?;
        assert_eq!("ab", text_of(&doc).to_string());
        Ok(())
    }

    #[test]
    fn edits_outside_an_update_are_not_initialized() {
        let mut text = JsonText::new();
        assert_eq!(
            YorkieError::NotInitialized("Text is not initialized yet".to_owned()),
            text.edit(0, 0, "a", None).unwrap_err()
        );

        let mut root = JsonObject::new();
        let text = root.set_text("k1").unwrap();
        assert!(text.set_style(0, 0, BTreeMap::new()).is_err());
    }

    #[test]
    fn texts_inside_arrays_record_edits() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let list = root
                .set("list", crate::JsonArray::new())?
                .get_array_mut("list")?;
            list.push_text()?.edit(0, 0, "hi", None)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("list")?
                .get_text_mut(0)?
                .edit(2, 2, "!", None)
                .map(|_| ())
        })?;

        assert_eq!(
            r#"{"list":[[{"val":"hi"},{"val":"!"}]]}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    // JS `stringifyObjectValues`: a string that is not a JSON document is
    // stored raw; everything else is JSON-encoded.
    #[test]
    fn stringifies_attributes_like_js() {
        assert_eq!(
            BTreeMap::from([
                ("bold".to_owned(), "true".to_owned()),
                ("color".to_owned(), "red".to_owned()),
                ("one".to_owned(), r#""1""#.to_owned()),
                ("size".to_owned(), "12".to_owned()),
                ("tags".to_owned(), r#"["a"]"#.to_owned()),
            ]),
            stringify_attributes(attrs(json!({
                "bold": true,
                "color": "red",
                "one": "1",
                "size": 12,
                "tags": ["a"],
            })))
        );
    }
}
