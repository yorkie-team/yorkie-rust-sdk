//! The public tree facade (JS `src/document/json/tree.ts`, Go
//! `pkg/document/json/tree.go`).

use super::text::stringify_attributes;
use super::{escape_json_string, lock_recorder, JsonEditRecorderRef};
use crate::crdt::rht::Rht;
use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId, TreePos};
use crate::event::parse_attr_value;
use crate::operation::TreeEditOperation;
use crate::{Result, TimeTicket, YorkieError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// The node type JS and Go reserve for text nodes (JS `DefaultTextType`).
const TEXT_NODE_TYPE: &str = "text";

/// A tree node given to `JsonTree` or reported in a tree edit op info (JS
/// `TreeNode`, the union of `ElementNode` and `TextNode`). JS uses one type
/// for both directions, and so does this port.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeNodeInput {
    /// An element node (JS `ElementNode`). `node_type` must not be `"text"`,
    /// which JS reserves for text nodes.
    Element {
        node_type: String,
        attributes: BTreeMap<String, Value>,
        children: Vec<TreeNodeInput>,
    },
    /// A text node (JS `TextNode`). Its value must not be empty.
    Text { value: String },
}

impl TreeNodeInput {
    /// Creates an element node without attributes.
    pub fn element(node_type: impl Into<String>, children: Vec<TreeNodeInput>) -> Self {
        Self::element_with_attributes(node_type, BTreeMap::new(), children)
    }

    /// Creates an element node with attributes.
    pub fn element_with_attributes(
        node_type: impl Into<String>,
        attributes: BTreeMap<String, Value>,
        children: Vec<TreeNodeInput>,
    ) -> Self {
        Self::Element {
            node_type: node_type.into(),
            attributes,
            children,
        }
    }

    /// Creates a text node.
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
        }
    }

    fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    /// Converts a CRDT node to the JS shape (JS `toTreeNode`): only live
    /// children, and attribute values decoded like JS `parseObjectValues`.
    pub(crate) fn from_crdt(node: &TreeNode) -> Self {
        if node.is_text() {
            return Self::text(node.value());
        }

        let attributes = node
            .attrs()
            .map(|attrs| {
                attrs
                    .to_object()
                    .into_iter()
                    .map(|(key, value)| {
                        let value = parse_attr_value(&value);
                        (key, value)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::element_with_attributes(
            node.node_type(),
            attributes,
            node.children().map(Self::from_crdt).collect(),
        )
    }

    /// Writes this node in the key order of JS `JSON.stringify(TreeNode)`.
    fn write_json(&self, out: &mut String) {
        match self {
            Self::Text { value } => {
                out.push_str(&format!(
                    "{{\"type\":\"{TEXT_NODE_TYPE}\",\"value\":\"{}\"}}",
                    escape_json_string(value)
                ));
            }
            Self::Element {
                node_type,
                attributes,
                children,
            } => {
                out.push_str(&format!(
                    "{{\"type\":\"{}\",\"children\":[",
                    escape_json_string(node_type)
                ));
                for (index, child) in children.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    child.write_json(out);
                }
                out.push(']');
                if !attributes.is_empty() {
                    out.push_str(",\"attributes\":");
                    out.push_str(
                        &Value::Object(attributes.clone().into_iter().collect()).to_string(),
                    );
                }
                out.push('}');
            }
        }
    }
}

/// The tree a `JsonTree` reads and edits: the root given to `JsonTree::new`
/// until the tree belongs to a document, then a copy of the CRDT tree.
#[derive(Clone)]
enum TreeState {
    Initial(TreeNodeInput),
    Crdt(Box<CrdtTree>),
}

/// A CRDT-based tree for the document of a structured editor such as
/// ProseMirror (JS `Tree`).
///
/// Indexes count element boundaries and UTF-16 code units, as in JS. A
/// `JsonTree` reads and edits only once it belongs to a document, and edits
/// only inside `Document::update`; otherwise its methods fail with
/// `YorkieError::NotInitialized`, like an uninitialized JS `Tree`.
#[derive(Clone)]
pub struct JsonTree {
    state: TreeState,
    recorder: Option<JsonEditRecorderRef>,
}

impl fmt::Debug for JsonTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("JsonTree")
            .field(&self.to_json_string())
            .finish()
    }
}

/// Trees are equal when they show the same nodes, like the other JSON values,
/// which compare by value rather than CRDT identity.
impl PartialEq for JsonTree {
    fn eq(&self, other: &Self) -> bool {
        self.to_json_string() == other.to_json_string()
    }
}

impl JsonTree {
    /// Creates a tree to set into a document (JS `new Tree(root)`). As in JS
    /// and Go, the attributes of `root` itself are not kept; its children
    /// keep theirs.
    pub fn new(root: TreeNodeInput) -> Self {
        Self {
            state: TreeState::Initial(root),
            recorder: None,
        }
    }

    /// Replaces the range `from..to` with `contents` (JS `Tree.editBulk`, and
    /// `Tree.edit` for one or no node). `contents` must be all text nodes or
    /// all element nodes.
    ///
    /// A `split_level` above zero first splits that many elements at `from`,
    /// innermost first, and inserts `contents` where the split opened. Two
    /// elements merge when the range deletes the boundary between them.
    pub fn edit(
        &mut self,
        from: usize,
        to: usize,
        contents: Vec<TreeNodeInput>,
        split_level: usize,
    ) -> Result<()> {
        let recorder = self.recorder()?;
        if from > to {
            return Err(YorkieError::InvalidArgument(
                "from should be less than or equal to to".to_owned(),
            ));
        }
        let tree = self.tree()?;
        let from_pos = tree.find_pos(from, true)?;
        let to_pos = tree.find_pos(to, true)?;
        self.edit_internal(&recorder, from_pos, to_pos, contents, split_level)
    }

    /// Replaces the range between two paths with `contents` (JS
    /// `Tree.editBulkByPath`, and `Tree.editByPath` for one or no node).
    pub fn edit_by_path(
        &mut self,
        from_path: &[usize],
        to_path: &[usize],
        contents: Vec<TreeNodeInput>,
        split_level: usize,
    ) -> Result<()> {
        let recorder = self.recorder()?;
        if from_path.len() != to_path.len() {
            return Err(YorkieError::InvalidArgument(
                "path length should be equal".to_owned(),
            ));
        }
        if from_path.is_empty() {
            return Err(YorkieError::InvalidArgument(
                "path should not be empty".to_owned(),
            ));
        }
        let tree = self.tree()?;
        let from_pos = tree.path_to_pos(from_path)?;
        let to_pos = tree.path_to_pos(to_path)?;
        self.edit_internal(&recorder, from_pos, to_pos, contents, split_level)
    }

    /// Splits the element holding the position at `path` in two (JS
    /// `Tree.splitByPath`): a text position splits the element holding the
    /// text, a child position the element holding the children.
    pub fn split_by_path(&mut self, path: &[usize]) -> Result<()> {
        let recorder = self.recorder()?;
        if path.is_empty() {
            return Err(YorkieError::InvalidArgument(
                "path should not be empty".to_owned(),
            ));
        }
        // A split needs a parent to hold the two halves.
        let tree = self.tree()?;
        if tree.split_target_is_root(path)? {
            return Err(YorkieError::InvalidArgument(
                "the root node cannot be split".to_owned(),
            ));
        }
        let pos = tree.path_to_pos(path)?;
        self.edit_internal(&recorder, pos.clone(), pos, Vec::new(), 1)
    }

    /// Merges the element at `path` into its previous sibling (JS
    /// `Tree.mergeByPath`) by deleting the boundary between them, so two
    /// replicas merging the same boundary merge it once.
    pub fn merge_by_path(&mut self, path: &[usize]) -> Result<()> {
        let recorder = self.recorder()?;
        if path.is_empty() {
            return Err(YorkieError::InvalidArgument(
                "path should not be empty".to_owned(),
            ));
        }
        let tree = self.tree()?;
        let (from_path, to_path) = tree.merge_boundary_paths(path)?;
        let from_pos = tree.path_to_pos(&from_path)?;
        let to_pos = tree.path_to_pos(&to_path)?;
        self.edit_internal(&recorder, from_pos, to_pos, Vec::new(), 0)
    }

    /// Sets `attributes` on the elements in `from..to` (JS `Tree.style`).
    /// Text nodes in the range are not styled.
    pub fn style(
        &mut self,
        from: usize,
        to: usize,
        attributes: BTreeMap<String, Value>,
    ) -> Result<()> {
        let recorder = self.recorder()?;
        let (from_pos, to_pos) = self.style_range(from, to)?;
        let attributes = stringify_attributes(attributes);

        let mut recorder = lock_recorder(&recorder);
        let executed_at = recorder.issue_time_ticket();
        let tree = self.tree_mut()?;
        tree.style_by_range_with_changes(
            (from_pos.clone(), to_pos.clone()),
            attributes.clone(),
            executed_at.clone(),
            None,
        )?;
        // The recorded operation registers these with the document root.
        tree.drain_pending_gc_pairs();
        recorder.record_tree_style(tree.created_at(), from_pos, to_pos, attributes, executed_at);
        Ok(())
    }

    /// Removes the attributes named in `keys` from the elements in
    /// `from..to` (JS `Tree.removeStyle`).
    pub fn remove_style<I, S>(&mut self, from: usize, to: usize, keys: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let recorder = self.recorder()?;
        let (from_pos, to_pos) = self.style_range(from, to)?;
        let keys = keys.into_iter().map(Into::into).collect::<Vec<_>>();

        let mut recorder = lock_recorder(&recorder);
        let executed_at = recorder.issue_time_ticket();
        let tree = self.tree_mut()?;
        tree.remove_style_by_range_with_changes(
            (from_pos.clone(), to_pos.clone()),
            &keys,
            executed_at.clone(),
            None,
        )?;
        tree.drain_pending_gc_pairs();
        recorder.record_tree_remove_style(tree.created_at(), from_pos, to_pos, keys, executed_at);
        Ok(())
    }

    /// Returns the XML of this tree (JS `Tree.toXML`).
    pub fn to_xml(&self) -> Result<String> {
        Ok(self.tree()?.to_xml())
    }

    /// Returns the JSON of this tree (JS `Tree.toJSON`).
    pub fn to_json(&self) -> Result<String> {
        Ok(self.tree()?.to_json())
    }

    /// Returns the size of the root's content (JS `Tree.getSize`).
    pub fn size(&self) -> Result<usize> {
        Ok(self.tree()?.root().len())
    }

    /// Converts an index to a path (JS `Tree.indexToPath`).
    pub fn index_to_path(&self, index: usize) -> Result<Vec<usize>> {
        self.tree()?.index_to_path(index)
    }

    /// Converts a path to an index (JS `Tree.pathToIndex`).
    pub fn path_to_index(&self, path: &[usize]) -> Result<usize> {
        self.tree()?.path_to_index(path)
    }

    /// Serializes this tree as JSON, like `to_json`, for `JsonValue`. A tree
    /// that does not belong to a document yet is written from its root.
    pub(crate) fn to_json_string(&self) -> String {
        match &self.state {
            TreeState::Initial(root) => {
                let mut json = String::new();
                root.write_json(&mut json);
                json
            }
            TreeState::Crdt(tree) => tree.to_json(),
        }
    }

    /// Builds the CRDT root this tree creates when it is set into a document
    /// (JS `Tree.buildRoot`), issuing one ticket per node. A tree read from a
    /// document is copied from its current nodes.
    pub(crate) fn build_root(
        &self,
        issue_time_ticket: &mut dyn FnMut() -> TimeTicket,
    ) -> Result<TreeNode> {
        let input;
        let root = match &self.state {
            TreeState::Initial(root) => root,
            TreeState::Crdt(tree) => {
                input = TreeNodeInput::from_crdt(tree.root());
                &input
            }
        };
        let TreeNodeInput::Element {
            node_type,
            children,
            ..
        } = root
        else {
            return Err(YorkieError::InvalidArgument(
                "the root of a tree must be an element node".to_owned(),
            ));
        };
        validate_element_type(node_type)?;

        let mut node = TreeNode::create_element(
            TreeNodeId::new(issue_time_ticket(), 0),
            node_type.clone(),
            None,
            Vec::new(),
        );
        for child in children {
            node.append(build_descendant(child, issue_time_ticket)?);
        }
        Ok(node)
    }

    pub(crate) fn from_crdt(tree: &CrdtTree) -> Self {
        Self {
            state: TreeState::Crdt(Box::new(tree.deepcopy())),
            recorder: None,
        }
    }

    pub(super) fn attach_recorder(&mut self, recorder: JsonEditRecorderRef) {
        self.recorder = Some(recorder);
    }

    pub(super) fn detach_recorder(&mut self) {
        self.recorder = None;
    }

    /// Validates and creates the content nodes, applies the edit to the local
    /// copy, and records it (JS `Tree.editInternal`).
    fn edit_internal(
        &mut self,
        recorder: &JsonEditRecorderRef,
        from_pos: TreePos,
        to_pos: TreePos,
        contents: Vec<TreeNodeInput>,
        split_level: usize,
    ) -> Result<()> {
        validate_tree_nodes(&contents)?;
        if contents.first().is_some_and(|content| !content.is_text()) {
            for content in &contents {
                if let TreeNodeInput::Element { children, .. } = content {
                    validate_tree_nodes(children)?;
                }
            }
        }

        let mut recorder = lock_recorder(recorder);
        // As in JS, the edit runs at the last ticket issued before its
        // content nodes are created.
        let executed_at = recorder.last_time_ticket();
        let mut issue_time_ticket = || recorder.issue_time_ticket();
        let nodes = create_content_nodes(&contents, &mut issue_time_ticket)?;
        let nodes = (!nodes.is_empty()).then_some(nodes);

        // The elements a split creates take tickets of their own. They are
        // recorded so every replica uses them rather than rebuilding them.
        let mut split_tickets = Vec::new();
        let tree = self.tree_mut()?;
        tree.edit(
            (from_pos.clone(), to_pos.clone()),
            nodes.clone(),
            split_level,
            executed_at.clone(),
            &mut || {
                let ticket = recorder.issue_time_ticket();
                split_tickets.push(ticket.clone());
                ticket
            },
            None,
        )?;
        let mut operation = TreeEditOperation::create(
            tree.created_at().clone(),
            from_pos,
            to_pos,
            nodes,
            split_level,
            Some(executed_at),
        );
        operation.set_split_tickets(split_tickets);
        recorder.record_tree_edit(operation);
        Ok(())
    }

    fn style_range(&self, from: usize, to: usize) -> Result<(TreePos, TreePos)> {
        if from > to {
            return Err(YorkieError::InvalidArgument(
                "from should be less than or equal to to".to_owned(),
            ));
        }
        let tree = self.tree()?;
        Ok((tree.find_pos(from, true)?, tree.find_pos(to, true)?))
    }

    fn tree(&self) -> Result<&CrdtTree> {
        match &self.state {
            TreeState::Crdt(tree) => Ok(tree),
            TreeState::Initial(_) => Err(not_initialized()),
        }
    }

    fn tree_mut(&mut self) -> Result<&mut CrdtTree> {
        match &mut self.state {
            TreeState::Crdt(tree) => Ok(tree),
            TreeState::Initial(_) => Err(not_initialized()),
        }
    }

    fn recorder(&self) -> Result<JsonEditRecorderRef> {
        self.recorder.clone().ok_or_else(not_initialized)
    }
}

fn not_initialized() -> YorkieError {
    YorkieError::NotInitialized("Tree is not initialized yet".to_owned())
}

/// Creates the CRDT nodes an edit inserts (JS `Tree.editInternal`): text
/// contents become one text node holding their joined values, and each
/// element content becomes its own subtree.
fn create_content_nodes(
    contents: &[TreeNodeInput],
    issue_time_ticket: &mut dyn FnMut() -> TimeTicket,
) -> Result<Vec<TreeNode>> {
    if contents.first().is_some_and(TreeNodeInput::is_text) {
        let value = contents
            .iter()
            .map(|content| match content {
                TreeNodeInput::Text { value } => value.as_str(),
                TreeNodeInput::Element { .. } => "",
            })
            .collect::<String>();
        return Ok(vec![TreeNode::create_text(
            TreeNodeId::new(issue_time_ticket(), 0),
            value,
        )]);
    }

    contents
        .iter()
        .map(|content| create_element_node(content, issue_time_ticket))
        .collect()
}

/// Creates an element content node (JS `createCRDTTreeNode`), which, unlike
/// its descendants, takes one ticket for its attributes and another for its
/// ID.
fn create_element_node(
    content: &TreeNodeInput,
    issue_time_ticket: &mut dyn FnMut() -> TimeTicket,
) -> Result<TreeNode> {
    let TreeNodeInput::Element {
        node_type,
        attributes,
        children,
    } = content
    else {
        return build_descendant(content, issue_time_ticket);
    };
    validate_element_type(node_type)?;

    let attrs_ticket = issue_time_ticket();
    let attrs = build_attrs(attributes, &attrs_ticket);
    let mut node = TreeNode::create_element(
        TreeNodeId::new(issue_time_ticket(), 0),
        node_type.clone(),
        attrs,
        Vec::new(),
    );
    for child in children {
        node.append(build_descendant(child, issue_time_ticket)?);
    }
    Ok(node)
}

/// Creates a CRDT subtree with one ticket per node (JS `buildDescendants`).
fn build_descendant(
    input: &TreeNodeInput,
    issue_time_ticket: &mut dyn FnMut() -> TimeTicket,
) -> Result<TreeNode> {
    let ticket = issue_time_ticket();
    match input {
        TreeNodeInput::Text { value } => {
            validate_text_node(value)?;
            Ok(TreeNode::create_text(TreeNodeId::new(ticket, 0), value))
        }
        TreeNodeInput::Element {
            node_type,
            attributes,
            children,
        } => {
            validate_element_type(node_type)?;
            let attrs = build_attrs(attributes, &ticket);
            let mut node = TreeNode::create_element(
                TreeNodeId::new(ticket, 0),
                node_type.clone(),
                attrs,
                Vec::new(),
            );
            for child in children {
                node.append(build_descendant(child, issue_time_ticket)?);
            }
            Ok(node)
        }
    }
}

fn build_attrs(attributes: &BTreeMap<String, Value>, ticket: &TimeTicket) -> Option<Rht> {
    if attributes.is_empty() {
        return None;
    }

    let mut attrs = Rht::new();
    for (key, value) in stringify_attributes(attributes.clone()) {
        attrs.set(key, value, ticket.clone());
    }
    Some(attrs)
}

/// Rejects mixing text and element nodes in one list, and empty text nodes
/// in a text list (JS `validateTreeNodes`).
fn validate_tree_nodes(nodes: &[TreeNodeInput]) -> Result<()> {
    let Some(first) = nodes.first() else {
        return Ok(());
    };

    let first_is_text = first.is_text();
    for node in nodes {
        if node.is_text() != first_is_text {
            return Err(YorkieError::InvalidArgument(
                "element node and text node cannot be passed together".to_owned(),
            ));
        }
        if let TreeNodeInput::Text { value } = node {
            validate_text_node(value)?;
        }
    }
    Ok(())
}

/// Rejects an empty text node (JS `validateTextNode`). A Rust string cannot
/// hold the lone UTF-16 surrogate JS also rejects here.
fn validate_text_node(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(YorkieError::InvalidArgument(
            "text node cannot have empty value".to_owned(),
        ));
    }
    Ok(())
}

/// Rejects an element that uses the text node type. In JS the type alone
/// tells the two apart, so such a node would be a text node there.
fn validate_element_type(node_type: &str) -> Result<()> {
    if node_type == TEXT_NODE_TYPE {
        return Err(YorkieError::InvalidArgument(format!(
            "element node cannot have the type {TEXT_NODE_TYPE:?}"
        )));
    }
    Ok(())
}

// Cases follow the single-document tests of JS
// `test/integration/tree_test.ts`: "Tree", "Tree.edit", "Tree.style",
// "Tree.splitByPath/mergeByPath" and "Tree(edge cases)", leaving out
// styleByPath and the position-range conversions.
#[cfg(test)]
mod tests {
    use super::{JsonTree, TreeNodeInput};
    use crate::{
        DocEvent, Document, JsonValue, OperationInfo, Result, TreeStyleOpValue, YorkieError,
    };
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
        TreeNodeInput::element(node_type, children)
    }

    fn element_with(node_type: &str, attrs: Value, children: Vec<TreeNodeInput>) -> TreeNodeInput {
        TreeNodeInput::element_with_attributes(node_type, attributes(attrs), children)
    }

    fn text(value: &str) -> TreeNodeInput {
        TreeNodeInput::text(value)
    }

    fn attributes(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).expect("attributes are an object")
    }

    fn tree_of(doc: &Document) -> &JsonTree {
        match doc.get_root().get("t") {
            Some(JsonValue::Tree(tree)) => tree,
            other => panic!("expected tree, got {other:?}"),
        }
    }

    fn xml(doc: &Document) -> String {
        tree_of(doc)
            .to_xml()
            .expect("a document tree is initialized")
    }

    /// `<doc><p>ab</p></doc>`, the starting tree of most JS cases.
    fn doc_with_p_ab() -> Result<Document> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            root.set_tree("t", element("doc", vec![element("p", vec![text("ab")])]))
                .map(|_| ())
        })?;
        assert_eq!("<doc><p>ab</p></doc>", xml(&doc));
        Ok(doc)
    }

    /// Collects the op infos of local changes under `$.t`.
    fn collect_local_ops(doc: &mut Document) -> Arc<Mutex<Vec<OperationInfo>>> {
        let ops = Arc::new(Mutex::new(Vec::new()));
        let sink = ops.clone();
        doc.subscribe_path("$.t", move |event| {
            if let DocEvent::LocalChange(info) = event {
                sink.lock().unwrap().extend(info.operations.iter().cloned());
            }
        });
        ops
    }

    #[test]
    fn can_be_created() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree("t", element("root", vec![]))?;
            tree.edit(0, 0, vec![element("p", vec![])], 0)?;
            assert_eq!("<root><p></p></root>", tree.to_xml()?);
            assert_eq!(
                r#"{"t":{"type":"root","children":[{"type":"p","children":[]}]}}"#,
                root.to_sorted_json()
            );

            let tree = root.get_tree_mut("t")?;
            tree.edit(1, 1, vec![text("AB")], 0)?;
            assert_eq!("<root><p>AB</p></root>", tree.to_xml()?);
            assert_eq!(
                r#"{"t":{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"AB"}]}]}}"#,
                root.to_sorted_json()
            );

            let tree = root.get_tree_mut("t")?;
            tree.edit(3, 3, vec![text("CD")], 0)?;
            assert_eq!("<root><p>ABCD</p></root>", tree.to_xml()?);
            assert_eq!(
                r#"{"t":{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"AB"},{"type":"text","value":"CD"}]}]}}"#,
                root.to_sorted_json()
            );

            let tree = root.get_tree_mut("t")?;
            tree.edit(1, 5, vec![text("Yorkie")], 0)?;
            assert_eq!("<root><p>Yorkie</p></root>", tree.to_xml()?);
            assert_eq!(
                r#"{"t":{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"Yorkie"}]}]}}"#,
                root.to_sorted_json()
            );
            Ok(())
        })?;
        assert_eq!("<root><p>Yorkie</p></root>", xml(&doc));
        assert_eq!(
            r#"{"t":{"type":"root","children":[{"type":"p","children":[{"type":"text","value":"Yorkie"}]}]}}"#,
            doc.to_sorted_json()
        );
        Ok(())
    }

    #[test]
    fn can_be_created_from_json() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree(
                "t",
                element(
                    "doc",
                    vec![
                        element("p", vec![text("ab")]),
                        element(
                            "ng",
                            vec![
                                element("note", vec![text("cd")]),
                                element("note", vec![text("ef")]),
                            ],
                        ),
                        element("bp", vec![text("gh")]),
                    ],
                ),
            )?;
            assert_eq!(
                "<doc><p>ab</p><ng><note>cd</note><note>ef</note></ng><bp>gh</bp></doc>",
                tree.to_xml()?
            );
            assert_eq!(18, tree.size()?);
            Ok(())
        })
    }

    #[test]
    fn can_be_created_from_json_with_attributes() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree(
                "t",
                element(
                    "doc",
                    vec![element(
                        "p",
                        vec![element_with(
                            "span",
                            json!({"bold": true}),
                            vec![text("hello")],
                        )],
                    )],
                ),
            )?;
            assert_eq!(
                r#"<doc><p><span bold="true">hello</span></p></doc>"#,
                tree.to_xml()?
            );
            Ok(())
        })?;
        assert_eq!(
            r#"<doc><p><span bold="true">hello</span></p></doc>"#,
            xml(&doc)
        );
        Ok(())
    }

    #[test]
    fn can_edit_its_content() -> Result<()> {
        let mut doc = Document::new("test-doc");
        let p_ab = || element("doc", vec![element("p", vec![text("ab")])]);

        doc.update(|root| {
            let tree = root.set_tree("t", p_ab())?;
            tree.edit(1, 1, vec![text("X")], 0)?;
            assert_eq!("<doc><p>Xab</p></doc>", tree.to_xml()?);
            tree.edit(1, 2, vec![], 0)?;
            assert_eq!("<doc><p>ab</p></doc>", tree.to_xml()?);
            tree.edit(2, 2, vec![text("X")], 0)?;
            assert_eq!("<doc><p>aXb</p></doc>", tree.to_xml()?);
            tree.edit(2, 3, vec![], 0)?;
            assert_eq!("<doc><p>ab</p></doc>", tree.to_xml()?);
            Ok(())
        })?;
        assert_eq!("<doc><p>ab</p></doc>", xml(&doc));

        doc.update(|root| {
            let tree = root.set_tree("t", p_ab())?;
            tree.edit(2, 2, vec![text("X")], 0)?;
            assert_eq!("<doc><p>aXb</p></doc>", tree.to_xml()?);
            tree.edit(1, 4, vec![], 0)?;
            assert_eq!("<doc><p></p></doc>", tree.to_xml()?);
            Ok(())
        })?;
        assert_eq!("<doc><p></p></doc>", xml(&doc));

        doc.update(|root| {
            let tree = root.set_tree("t", p_ab())?;
            tree.edit(3, 3, vec![text("X")], 0)?;
            assert_eq!("<doc><p>abX</p></doc>", tree.to_xml()?);
            tree.edit(3, 4, vec![], 0)?;
            assert_eq!("<doc><p>ab</p></doc>", tree.to_xml()?);
            tree.edit(2, 3, vec![], 0)?;
            assert_eq!("<doc><p>a</p></doc>", tree.to_xml()?);
            Ok(())
        })?;
        assert_eq!("<doc><p>a</p></doc>", xml(&doc));
        Ok(())
    }

    #[test]
    fn can_be_subscribed_by_handler() -> Result<()> {
        let mut doc = doc_with_p_ab()?;
        let ops = collect_local_ops(&mut doc);

        doc.update(|root| root.get_tree_mut("t")?.edit(1, 1, vec![text("X")], 0))?;
        assert_eq!("<doc><p>Xab</p></doc>", xml(&doc));

        assert_eq!(
            vec![OperationInfo::TreeEdit {
                path: "$.t".to_owned(),
                from: 1,
                to: 1,
                value: Some(vec![text("X")]),
                split_level: None,
                from_path: vec![0, 0],
                to_path: vec![0, 0],
            }],
            *ops.lock().unwrap()
        );
        Ok(())
    }

    fn doc_tc_p_tn_ab() -> Result<Document> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            root.set_tree(
                "t",
                element(
                    "doc",
                    vec![element(
                        "tc",
                        vec![element("p", vec![element("tn", vec![text("ab")])])],
                    )],
                ),
            )
            .map(|_| ())
        })?;
        assert_eq!("<doc><tc><p><tn>ab</tn></p></tc></doc>", xml(&doc));
        Ok(doc)
    }

    #[test]
    fn can_be_subscribed_by_handler_with_path() -> Result<()> {
        let mut doc = doc_tc_p_tn_ab()?;
        let ops = collect_local_ops(&mut doc);

        doc.update(|root| {
            root.get_tree_mut("t")?
                .edit_by_path(&[0, 0, 0, 1], &[0, 0, 0, 1], vec![text("X")], 0)
        })?;
        assert_eq!("<doc><tc><p><tn>aXb</tn></p></tc></doc>", xml(&doc));

        let ops = ops.lock().unwrap();
        assert_eq!(1, ops.len());
        let OperationInfo::TreeEdit {
            from_path,
            to_path,
            value,
            ..
        } = &ops[0]
        else {
            panic!("expected a tree edit, got {:?}", ops[0]);
        };
        assert_eq!(&vec![0, 0, 0, 1], from_path);
        assert_eq!(&vec![0, 0, 0, 1], to_path);
        assert_eq!(&Some(vec![text("X")]), value);
        Ok(())
    }

    #[test]
    fn can_edit_its_content_with_path() -> Result<()> {
        let mut doc = doc_tc_p_tn_ab()?;
        doc.update(|root| {
            let tree = root.get_tree_mut("t")?;
            tree.edit_by_path(&[0, 0, 0, 1], &[0, 0, 0, 1], vec![text("X")], 0)?;
            assert_eq!("<doc><tc><p><tn>aXb</tn></p></tc></doc>", tree.to_xml()?);

            tree.edit_by_path(&[0, 0, 0, 3], &[0, 0, 0, 3], vec![text("!")], 0)?;
            assert_eq!("<doc><tc><p><tn>aXb!</tn></p></tc></doc>", tree.to_xml()?);

            tree.edit_by_path(
                &[0, 0, 1],
                &[0, 0, 1],
                vec![element("tn", vec![text("cd")])],
                0,
            )?;
            assert_eq!(
                "<doc><tc><p><tn>aXb!</tn><tn>cd</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(
                &[0, 1],
                &[0, 1],
                vec![element("p", vec![element("tn", vec![text("q")])])],
                0,
            )?;
            assert_eq!(
                "<doc><tc><p><tn>aXb!</tn><tn>cd</tn></p><p><tn>q</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(&[0, 1, 0, 0], &[0, 1, 0, 0], vec![text("a")], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>aXb!</tn><tn>cd</tn></p><p><tn>aq</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(&[0, 1, 0, 2], &[0, 1, 0, 2], vec![text("B")], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>aXb!</tn><tn>cd</tn></p><p><tn>aqB</tn></p></tc></doc>",
                tree.to_xml()?
            );

            // JS: 'unacceptable path'.
            assert!(matches!(
                tree.edit_by_path(&[0, 0, 4], &[0, 0, 4], vec![element("tn", vec![])], 0),
                Err(YorkieError::InvalidTreePosition(_))
            ));
            Ok(())
        })
    }

    #[test]
    fn can_edit_its_content_with_path_2() -> Result<()> {
        let mut doc = Document::new("test-doc");
        let empty_p = || element("p", vec![element("tn", vec![])]);
        doc.update(|root| {
            let tree = root.set_tree("t", element("doc", vec![element("tc", vec![empty_p()])]))?;
            assert_eq!("<doc><tc><p><tn></tn></p></tc></doc>", tree.to_xml()?);

            tree.edit_by_path(&[0, 0, 0, 0], &[0, 0, 0, 0], vec![text("a")], 0)?;
            assert_eq!("<doc><tc><p><tn>a</tn></p></tc></doc>", tree.to_xml()?);

            tree.edit_by_path(&[0, 1], &[0, 1], vec![empty_p()], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>a</tn></p><p><tn></tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(&[0, 1, 0, 0], &[0, 1, 0, 0], vec![text("b")], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>a</tn></p><p><tn>b</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(&[0, 2], &[0, 2], vec![empty_p()], 0)?;
            tree.edit_by_path(&[0, 2, 0, 0], &[0, 2, 0, 0], vec![text("c")], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>a</tn></p><p><tn>b</tn></p><p><tn>c</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(&[0, 3], &[0, 3], vec![empty_p()], 0)?;
            tree.edit_by_path(&[0, 3, 0, 0], &[0, 3, 0, 0], vec![text("d")], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>a</tn></p><p><tn>b</tn></p><p><tn>c</tn></p><p><tn>d</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(&[0, 3], &[0, 3], vec![empty_p()], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>a</tn></p><p><tn>b</tn></p><p><tn>c</tn></p><p><tn></tn></p><p><tn>d</tn></p></tc></doc>",
                tree.to_xml()?
            );
            Ok(())
        })
    }

    #[test]
    fn can_insert_multiple_text_nodes() -> Result<()> {
        let mut doc = doc_with_p_ab()?;
        doc.update(|root| {
            root.get_tree_mut("t")?
                .edit(3, 3, vec![text("c"), text("d")], 0)
        })?;
        assert_eq!("<doc><p>abcd</p></doc>", xml(&doc));
        Ok(())
    }

    #[test]
    fn can_insert_multiple_element_nodes() -> Result<()> {
        let mut doc = doc_with_p_ab()?;
        doc.update(|root| {
            root.get_tree_mut("t")?.edit(
                4,
                4,
                vec![
                    element("p", vec![text("cd")]),
                    element("i", vec![text("fg")]),
                ],
                0,
            )
        })?;
        assert_eq!("<doc><p>ab</p><p>cd</p><i>fg</i></doc>", xml(&doc));
        Ok(())
    }

    #[test]
    fn can_edit_its_content_with_path_when_multi_tree_nodes_passed() -> Result<()> {
        let mut doc = doc_tc_p_tn_ab()?;
        let p_tn = |a: &str, b: &str| element("p", vec![element("tn", vec![text(a), text(b)])]);
        doc.update(|root| {
            let tree = root.get_tree_mut("t")?;
            tree.edit_by_path(&[0, 0, 0, 1], &[0, 0, 0, 1], vec![text("X"), text("X")], 0)?;
            assert_eq!("<doc><tc><p><tn>aXXb</tn></p></tc></doc>", tree.to_xml()?);

            tree.edit_by_path(&[0, 1], &[0, 1], vec![p_tn("te", "st"), p_tn("te", "xt")], 0)?;
            assert_eq!(
                "<doc><tc><p><tn>aXXb</tn></p><p><tn>test</tn></p><p><tn>text</tn></p></tc></doc>",
                tree.to_xml()?
            );

            tree.edit_by_path(
                &[0, 3],
                &[0, 3],
                vec![
                    p_tn("te", "st"),
                    element("tn", vec![text("te"), text("xt")]),
                ],
                0,
            )?;
            assert_eq!(
                "<doc><tc><p><tn>aXXb</tn></p><p><tn>test</tn></p><p><tn>text</tn></p><p><tn>test</tn></p><tn>text</tn></tc></doc>",
                tree.to_xml()?
            );
            Ok(())
        })
    }

    /// Runs one `edit(3, 3, contents)` on `<doc><p>ab</p></doc>` and returns
    /// its error, checking that the failed update left the tree unchanged.
    fn edit_error(contents: Vec<TreeNodeInput>) -> Result<YorkieError> {
        let mut doc = doc_with_p_ab()?;
        let err = doc
            .update(|root| root.get_tree_mut("t")?.edit(3, 3, contents, 0))
            .expect_err("the edit should be rejected");
        assert_eq!("<doc><p>ab</p></doc>", xml(&doc));
        Ok(err)
    }

    fn invalid_argument(message: &str) -> YorkieError {
        YorkieError::InvalidArgument(message.to_owned())
    }

    #[test]
    fn detects_error_for_empty_text() -> Result<()> {
        assert_eq!(
            invalid_argument("text node cannot have empty value"),
            edit_error(vec![text("c"), text("")])?
        );
        Ok(())
    }

    #[test]
    fn detects_error_for_mixed_type_insertion() -> Result<()> {
        assert_eq!(
            invalid_argument("element node and text node cannot be passed together"),
            edit_error(vec![element("p", vec![]), text("d")])?
        );
        Ok(())
    }

    #[test]
    fn detects_correct_error_order() -> Result<()> {
        // [1]
        assert_eq!(
            invalid_argument("element node and text node cannot be passed together"),
            edit_error(vec![element("p", vec![text("c"), text("")]), text("d"),])?
        );
        // [2]
        assert_eq!(
            invalid_argument("text node cannot have empty value"),
            edit_error(vec![
                element("p", vec![text("c")]),
                element("p", vec![text("")]),
            ])?
        );
        // [3]
        assert_eq!(
            invalid_argument("element node and text node cannot be passed together"),
            edit_error(vec![text("d"), element("p", vec![text("c")])])?
        );
        Ok(())
    }

    #[test]
    fn rejects_inputs_rust_cannot_tell_apart_from_text() -> Result<()> {
        // In JS the type alone makes a node a text node.
        assert!(matches!(
            edit_error(vec![element("text", vec![])])?,
            YorkieError::InvalidArgument(_)
        ));

        let mut doc = Document::new("test-doc");
        let err = doc
            .update(|root| root.set_tree("t", text("ab")).map(|_| ()))
            .expect_err("a text root should be rejected");
        assert!(matches!(err, YorkieError::InvalidArgument(_)));
        let err = doc
            .update(|root| {
                root.set_tree("t", element("doc", vec![element("p", vec![text("")])]))
                    .map(|_| ())
            })
            .expect_err("an empty text node should be rejected");
        assert_eq!(invalid_argument("text node cannot have empty value"), err);
        Ok(())
    }

    #[test]
    fn rejects_backward_ranges_and_bad_paths() -> Result<()> {
        let mut doc = doc_with_p_ab()?;
        doc.update(|root| {
            let tree = root.get_tree_mut("t")?;
            let backward = invalid_argument("from should be less than or equal to to");
            assert_eq!(Err(backward.clone()), tree.edit(2, 1, vec![], 0));
            assert_eq!(Err(backward.clone()), tree.style(2, 1, BTreeMap::new()));
            assert_eq!(Err(backward), tree.remove_style(2, 1, ["bold"]));
            assert_eq!(
                Err(invalid_argument("path length should be equal")),
                tree.edit_by_path(&[0], &[0, 1], vec![], 0)
            );
            assert_eq!(
                Err(invalid_argument("path should not be empty")),
                tree.edit_by_path(&[], &[], vec![], 0)
            );
            Ok(())
        })
    }

    fn doc_with(root: TreeNodeInput) -> Result<Document> {
        let mut doc = Document::new("test-doc");
        doc.update(|r| r.set_tree("t", root).map(|_| ()))?;
        Ok(doc)
    }

    fn edit_tree(doc: &mut Document, edit: impl FnOnce(&mut JsonTree) -> Result<()>) -> Result<()> {
        doc.update(|root| edit(root.get_tree_mut("t")?))
    }

    /// `<doc><tc><p><tn>1234</tn></p><p><tn>5678</tn></p></tc></doc>`.
    fn table_cell() -> TreeNodeInput {
        let paragraph = |value| element("p", vec![element("tn", vec![text(value)])]);
        element(
            "doc",
            vec![element("tc", vec![paragraph("1234"), paragraph("5678")])],
        )
    }

    /// JS "Can split content using editByPath with splitLevel".
    #[test]
    fn can_split_content_using_edit_by_path_with_split_level() -> Result<()> {
        let mut doc = doc_with(table_cell())?;
        for (path, expected) in [
            (
                vec![0, 0, 0, 2],
                "<doc><tc><p><tn>12</tn><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            (
                vec![0, 0, 1],
                "<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p></tc></doc>",
            ),
            (
                vec![0, 2, 0, 4],
                "<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn><tn></tn></p></tc></doc>",
            ),
            (
                vec![0, 2, 1],
                "<doc><tc><p><tn>12</tn></p><p><tn>34</tn></p><p><tn>5678</tn></p><p><tn></tn></p></tc></doc>",
            ),
        ] {
            edit_tree(&mut doc, |tree| tree.edit_by_path(&path, &path, vec![], 1))?;
            assert_eq!(expected, xml(&doc));
        }
        Ok(())
    }

    /// JS "Can merge content using editByPath".
    #[test]
    fn can_merge_content_using_edit_by_path() -> Result<()> {
        let mut doc = doc_with(table_cell())?;
        edit_tree(&mut doc, |tree| {
            tree.edit_by_path(&[0, 0, 1], &[0, 1, 0], vec![], 0)
        })?;
        assert_eq!(
            "<doc><tc><p><tn>1234</tn><tn>5678</tn></p></tc></doc>",
            xml(&doc)
        );
        edit_tree(&mut doc, |tree| {
            tree.edit_by_path(&[0, 0, 0, 4], &[0, 0, 1, 0], vec![], 0)
        })?;
        assert_eq!("<doc><tc><p><tn>12345678</tn></p></tc></doc>", xml(&doc));
        Ok(())
    }

    /// JS "Can merge blocks using editByPath (wafflebase pattern)" and "Can
    /// split then merge blocks using editByPath (wafflebase backspace bug)".
    #[test]
    fn can_split_and_merge_blocks_using_edit_by_path() -> Result<()> {
        let block = |value| element("block", vec![element("inline", vec![text(value)])]);
        let mut doc = doc_with(element("doc", vec![block("as"), block("df")]))?;
        edit_tree(&mut doc, |tree| {
            tree.edit_by_path(&[0, 1], &[1, 0], vec![], 0)
        })?;
        assert_eq!(
            "<doc><block><inline>as</inline><inline>df</inline></block></doc>",
            xml(&doc)
        );

        let mut doc = doc_with(element("doc", vec![block("asdf")]))?;
        edit_tree(&mut doc, |tree| {
            tree.edit_by_path(&[0, 0, 2], &[0, 0, 2], vec![], 2)
        })?;
        assert_eq!(
            "<doc><block><inline>as</inline></block><block><inline>df</inline></block></doc>",
            xml(&doc)
        );
        edit_tree(&mut doc, |tree| {
            tree.edit_by_path(&[0, 1], &[1, 0], vec![], 0)
        })?;
        assert_eq!(
            "<doc><block><inline>as</inline><inline>df</inline></block></doc>",
            xml(&doc)
        );
        Ok(())
    }

    /// JS "Can delete nodes in a multi-level range".
    #[test]
    fn can_delete_nodes_in_a_multi_level_range() -> Result<()> {
        let mut doc = doc_with(element(
            "doc",
            vec![
                element("p", vec![text("ab"), element("p", vec![text("x")])]),
                element("p", vec![element("p", vec![text("cd")])]),
                element("p", vec![element("p", vec![text("y")]), text("ef")]),
            ],
        ))?;
        assert_eq!(
            "<doc><p>ab<p>x</p></p><p><p>cd</p></p><p><p>y</p>ef</p></doc>",
            xml(&doc)
        );
        edit_tree(&mut doc, |tree| tree.edit(2, 18, vec![], 0))?;
        assert_eq!("<doc><p>af</p></doc>", xml(&doc));
        Ok(())
    }

    /// `<doc><p><span>abc</span><span>de</span></p></doc>`, the tree of the
    /// JS "Tree.splitByPath/mergeByPath" cases.
    fn doc_with_two_spans() -> Result<Document> {
        doc_with_span(vec![
            element("span", vec![text("abc")]),
            element("span", vec![text("de")]),
        ])
    }

    /// JS "Can split a text position and a child position".
    #[test]
    fn can_split_a_text_position_and_a_child_position() -> Result<()> {
        let mut doc = doc_with_two_spans()?;
        edit_tree(&mut doc, |tree| tree.split_by_path(&[0, 0, 1]))?;
        assert_eq!(
            "<doc><p><span>a</span><span>bc</span><span>de</span></p></doc>",
            xml(&doc)
        );
        edit_tree(&mut doc, |tree| tree.split_by_path(&[0, 1]))?;
        assert_eq!(
            "<doc><p><span>a</span></p><p><span>bc</span><span>de</span></p></doc>",
            xml(&doc)
        );
        Ok(())
    }

    /// JS "Can merge a boundary back together".
    #[test]
    fn can_merge_a_boundary_back_together() -> Result<()> {
        let mut doc = doc_with_two_spans()?;
        edit_tree(&mut doc, |tree| tree.merge_by_path(&[0, 1]))?;
        assert_eq!("<doc><p><span>abcde</span></p></doc>", xml(&doc));
        Ok(())
    }

    /// JS "Should throw on empty paths for splitByPath and mergeByPath",
    /// "...when splitByPath targets the root", "...targets text held by the
    /// root", "...when mergeByPath targets a first child" and "...targets a
    /// text position". A rejected call changes nothing.
    #[test]
    fn rejects_split_and_merge_paths_js_rejects() -> Result<()> {
        let invalid = |message: &str| Err(YorkieError::InvalidArgument(message.to_owned()));
        let mut doc = doc_with_two_spans()?;
        assert_eq!(
            invalid("path should not be empty"),
            edit_tree(&mut doc, |tree| tree.split_by_path(&[]))
        );
        assert_eq!(
            invalid("path should not be empty"),
            edit_tree(&mut doc, |tree| tree.merge_by_path(&[]))
        );
        for path in [[0], [1]] {
            assert_eq!(
                invalid("the root node cannot be split"),
                edit_tree(&mut doc, |tree| tree.split_by_path(&path))
            );
        }
        for path in [&[0, 0][..], &[0]] {
            assert_eq!(
                invalid("the first child cannot be merged"),
                edit_tree(&mut doc, |tree| tree.merge_by_path(path))
            );
        }
        assert_eq!(
            invalid("text node cannot be merged"),
            edit_tree(&mut doc, |tree| tree.merge_by_path(&[0, 0, 1]))
        );
        assert_eq!(
            "<doc><p><span>abc</span><span>de</span></p></doc>",
            xml(&doc)
        );

        let mut doc = doc_with(element("doc", vec![text("abcde")]))?;
        assert_eq!(
            invalid("the root node cannot be split"),
            edit_tree(&mut doc, |tree| tree.split_by_path(&[2]))
        );
        assert_eq!("<doc>abcde</doc>", xml(&doc));
        Ok(())
    }

    /// JS "Can edit with splitLevel walking past root without throwing": the
    /// split stops at the root and the content is still inserted.
    #[test]
    fn splits_no_further_than_the_root() -> Result<()> {
        let seeds = [
            element("doc", vec![element("p", vec![text("x")])]),
            element(
                "doc",
                vec![element("p", vec![text("x")]), element("p", vec![text("y")])],
            ),
            element(
                "doc",
                vec![element("p", vec![element("b", vec![text("x")])])],
            ),
            element("doc", vec![element("p", vec![])]),
        ];
        for seed in seeds {
            let size = tree_of(&doc_with(seed.clone())?).size()?;
            for pos in 0..=size {
                for split_level in [1, 2] {
                    let mut doc = doc_with(seed.clone())?;
                    edit_tree(&mut doc, |tree| {
                        tree.edit(pos, pos, vec![text("a")], split_level)
                    })?;
                    let label = format!("{seed:?} pos={pos} split_level={split_level}");
                    assert!(tree_of(&doc).size()? > size, "{label}");
                    assert!(xml(&doc).contains('a'), "{label}");
                }
            }
        }
        Ok(())
    }

    /// JS reports a local split as one empty change where it opened, with no
    /// split level, and the content inserted with it on the same change.
    #[test]
    fn reports_a_split_as_an_empty_change_where_it_opened() -> Result<()> {
        let mut doc = doc_with_p_ab()?;
        let ops = collect_local_ops(&mut doc);
        edit_tree(&mut doc, |tree| tree.edit(2, 2, vec![], 1))?;
        edit_tree(&mut doc, |tree| tree.edit(1, 1, vec![text("c")], 1))?;
        // The content goes where the edit asked for it: the left half.
        assert_eq!("<doc><p>c</p><p>a</p><p>b</p></doc>", xml(&doc));
        assert_eq!(
            vec![
                OperationInfo::TreeEdit {
                    path: "$.t".to_owned(),
                    from: 2,
                    to: 2,
                    from_path: vec![0, 1],
                    to_path: vec![0, 1],
                    value: None,
                    split_level: None,
                },
                OperationInfo::TreeEdit {
                    path: "$.t".to_owned(),
                    from: 1,
                    to: 1,
                    from_path: vec![0, 0],
                    to_path: vec![0, 0],
                    value: Some(vec![text("c")]),
                    split_level: None,
                },
            ],
            *ops.lock().unwrap()
        );
        Ok(())
    }

    #[test]
    fn edits_only_inside_an_update() -> Result<()> {
        let not_initialized = YorkieError::NotInitialized("Tree is not initialized yet".to_owned());
        let detached = JsonTree::new(element(
            "doc",
            vec![element_with("p", json!({"b": 1}), vec![text("a")])],
        ));
        assert_eq!(Err(not_initialized.clone()), detached.to_xml());
        // A tree outside a document still serializes from its root.
        assert_eq!(
            r#"{"type":"doc","children":[{"type":"p","children":[{"type":"text","value":"a"}],"attributes":{"b":1}}]}"#,
            JsonValue::from(detached).to_sorted_json()
        );

        let doc = doc_with_p_ab()?;
        let mut tree = tree_of(&doc).clone();
        assert_eq!(vec![0, 1], tree.index_to_path(2)?);
        assert_eq!(2, tree.path_to_index(&[0, 1])?);
        assert_eq!(Err(not_initialized), tree.edit(1, 1, vec![text("X")], 0));
        Ok(())
    }

    #[test]
    fn copies_a_tree_read_from_a_document_when_set_again() -> Result<()> {
        let mut doc = doc_with_p_ab()?;
        let tree = tree_of(&doc).clone();
        doc.update(|root| root.set("copy", tree).map(|_| ()))?;
        doc.update(|root| root.get_tree_mut("copy")?.edit(1, 1, vec![text("X")], 0))?;

        assert_eq!("<doc><p>ab</p></doc>", xml(&doc));
        let Some(JsonValue::Tree(copy)) = doc.get_root().get("copy") else {
            panic!("expected the copied tree");
        };
        assert_eq!("<doc><p>Xab</p></doc>", copy.to_xml()?);
        Ok(())
    }

    #[test]
    fn deletes_very_first_text_when_a_tombstone_is_in_front() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree("t", element("root", vec![]))?;
            tree.edit(0, 0, vec![element("p", vec![text("abcdefghi")])], 0)?;
            assert_eq!("<root><p>abcdefghi</p></root>", tree.to_xml()?);

            tree.edit(1, 1, vec![text("12345")], 0)?;
            assert_eq!("<root><p>12345abcdefghi</p></root>", tree.to_xml()?);
            for (from, to, expected) in [
                (2, 5, "<root><p>15abcdefghi</p></root>"),
                (3, 5, "<root><p>15cdefghi</p></root>"),
                (2, 4, "<root><p>1defghi</p></root>"),
                (1, 3, "<root><p>efghi</p></root>"),
                (1, 2, "<root><p>fghi</p></root>"),
                (2, 5, "<root><p>f</p></root>"),
                (1, 2, "<root><p></p></root>"),
            ] {
                tree.edit(from, to, vec![], 0)?;
                assert_eq!(expected, tree.to_xml()?, "edit({from}, {to})");
            }
            Ok(())
        })?;
        assert_eq!("<root><p></p></root>", xml(&doc));
        Ok(())
    }

    #[test]
    fn deletes_a_node_after_several_text_nodes_longer_than_one() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree("t", element("root", vec![]))?;
            tree.edit(0, 0, vec![element("p", vec![text("abcde")])], 0)?;
            tree.edit(6, 6, vec![text("f")], 0)?;
            tree.edit(7, 7, vec![text("g")], 0)?;
            assert_eq!("<root><p>abcdefg</p></root>", tree.to_xml()?);
            for (from, to, expected) in [
                (7, 8, "<root><p>abcdef</p></root>"),
                (6, 7, "<root><p>abcde</p></root>"),
                (5, 6, "<root><p>abcd</p></root>"),
                (4, 5, "<root><p>abc</p></root>"),
                (3, 4, "<root><p>ab</p></root>"),
                (2, 3, "<root><p>a</p></root>"),
                (1, 2, "<root><p></p></root>"),
            ] {
                tree.edit(from, to, vec![], 0)?;
                assert_eq!(expected, tree.to_xml()?, "edit({from}, {to})");
            }
            Ok(())
        })?;
        assert_eq!("<root><p></p></root>", xml(&doc));
        Ok(())
    }

    fn doc_with_span(spans: Vec<TreeNodeInput>) -> Result<Document> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            root.set_tree("t", element("doc", vec![element("p", spans)]))
                .map(|_| ())
        })?;
        Ok(doc)
    }

    #[test]
    fn can_be_inserted_with_attributes_and_have_them_removed() -> Result<()> {
        let mut doc = doc_with_span(vec![element_with(
            "span",
            json!({"bold": true}),
            vec![text("hello")],
        )])?;
        assert_eq!(
            r#"<doc><p><span bold="true">hello</span></p></doc>"#,
            xml(&doc)
        );

        doc.update(|root| root.get_tree_mut("t")?.remove_style(1, 8, ["bold"]))?;
        assert_eq!("<doc><p><span>hello</span></p></doc>", xml(&doc));
        Ok(())
    }

    #[test]
    fn can_handle_removal_of_attributes_that_do_not_exist() -> Result<()> {
        let mut doc = doc_with_span(vec![
            element_with("span", json!({"bold": true}), vec![text("hello")]),
            element("span", vec![text("hi")]),
        ])?;
        assert_eq!(
            r#"<doc><p><span bold="true">hello</span><span>hi</span></p></doc>"#,
            xml(&doc)
        );

        let ops = collect_local_ops(&mut doc);
        doc.update(|root| root.get_tree_mut("t")?.remove_style(1, 12, ["italic"]))?;
        assert_eq!(
            r#"<doc><p><span bold="true">hello</span><span>hi</span></p></doc>"#,
            xml(&doc)
        );
        // As in JS, each span reports the removal: the attribute store records
        // a tombstone even for a key it did not hold.
        let removal = |from: usize, path: Vec<usize>, keys: &[&str]| {
            let mut to_path = path.clone();
            to_path.push(0);
            OperationInfo::TreeStyle {
                path: "$.t".to_owned(),
                from,
                to: from + 1,
                from_path: path,
                to_path,
                value: TreeStyleOpValue {
                    attributes: None,
                    attributes_to_remove: Some(keys.iter().map(|key| key.to_string()).collect()),
                },
            }
        };
        assert_eq!(
            vec![
                removal(1, vec![0, 0], &["italic"]),
                removal(8, vec![0, 1], &["italic"])
            ],
            std::mem::take(&mut *ops.lock().unwrap())
        );

        doc.update(|root| {
            root.get_tree_mut("t")?
                .remove_style(1, 8, ["italic", "bold"])
        })?;
        assert_eq!(
            "<doc><p><span>hello</span><span>hi</span></p></doc>",
            xml(&doc)
        );
        assert_eq!(
            vec![removal(1, vec![0, 0], &["italic", "bold"])],
            *ops.lock().unwrap()
        );
        Ok(())
    }

    #[test]
    fn can_be_edited_with_index() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree(
                "t",
                element(
                    "doc",
                    vec![element(
                        "tc",
                        vec![element_with(
                            "p",
                            json!({"a": "b"}),
                            vec![element("tn", vec![])],
                        )],
                    )],
                ),
            )?;
            assert_eq!(r#"<doc><tc><p a="b"><tn></tn></p></tc></doc>"#, tree.to_xml()?);

            tree.style(1, 2, attributes(json!({"c": "d"})))?;
            assert_eq!(
                r#"<doc><tc><p a="b" c="d"><tn></tn></p></tc></doc>"#,
                tree.to_xml()?
            );

            tree.style(1, 2, attributes(json!({"c": "q"})))?;
            assert_eq!(
                r#"<doc><tc><p a="b" c="q"><tn></tn></p></tc></doc>"#,
                tree.to_xml()?
            );

            tree.style(2, 3, attributes(json!({"z": "m"})))?;
            assert_eq!(
                r#"<doc><tc><p a="b" c="q"><tn z="m"></tn></p></tc></doc>"#,
                tree.to_xml()?
            );

            // From JS "Can be edited with path", which styles by path.
            assert_eq!(
                r#"{"t":{"type":"doc","children":[{"type":"tc","children":[{"type":"p","children":[{"type":"tn","children":[],"attributes":{"z":"m"}}],"attributes":{"a":"b","c":"q"}}]}]}}"#,
                root.to_sorted_json()
            );
            Ok(())
        })
    }

    #[test]
    fn can_style_nested_object() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            root.set_tree("t", element("doc", vec![element("p", vec![text("hello")])]))
                .map(|_| ())
        })?;
        doc.update(|root| {
            root.get_tree_mut("t")?.style(
                0,
                1,
                attributes(json!({"img": {"src": "yorkie.png"}, "rep": "false"})),
            )
        })?;
        assert_eq!(
            r#"<doc><p img="{&quot;src&quot;:&quot;yorkie.png&quot;}" rep="false">hello</p></doc>"#,
            xml(&doc)
        );
        Ok(())
    }

    #[test]
    fn can_style_node_with_element_attributes() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            let tree = root.set_tree(
                "t",
                element(
                    "doc",
                    vec![
                        element("p", vec![text("ab")]),
                        element("p", vec![text("cd")]),
                    ],
                ),
            )?;
            assert_eq!("<doc><p>ab</p><p>cd</p></doc>", tree.to_xml()?);

            // 01. style attributes to an element node: the opening tag, the
            // closing tag, and the whole.
            tree.style(0, 1, attributes(json!({"weight": "bold"})))?;
            assert_eq!(
                r#"<doc><p weight="bold">ab</p><p>cd</p></doc>"#,
                tree.to_xml()?
            );
            tree.style(3, 4, attributes(json!({"color": "red"})))?;
            assert_eq!(
                r#"<doc><p color="red" weight="bold">ab</p><p>cd</p></doc>"#,
                tree.to_xml()?
            );
            tree.style(0, 4, attributes(json!({"size": "small"})))?;
            assert_eq!(
                r#"<doc><p color="red" size="small" weight="bold">ab</p><p>cd</p></doc>"#,
                tree.to_xml()?
            );

            // 02. style attributes to elements.
            tree.style(0, 5, attributes(json!({"style": "italic"})))?;
            assert_eq!(
                r#"<doc><p color="red" size="small" style="italic" weight="bold">ab</p><p style="italic">cd</p></doc>"#,
                tree.to_xml()?
            );

            // 03. Ignore styling attributes to text nodes.
            tree.style(1, 3, attributes(json!({"bold": "true"})))?;
            assert_eq!(
                r#"<doc><p color="red" size="small" style="italic" weight="bold">ab</p><p style="italic">cd</p></doc>"#,
                tree.to_xml()?
            );
            Ok(())
        })
    }

    #[test]
    fn reports_style_and_typed_insert_op_infos() -> Result<()> {
        let mut doc = Document::new("test-doc");
        doc.update(|root| {
            root.set_tree("t", element("doc", vec![element("p", vec![])]))
                .map(|_| ())
        })?;
        let ops = collect_local_ops(&mut doc);

        doc.update(|root| {
            root.get_tree_mut("t")?
                .style(0, 1, attributes(json!({"key": "a", "n": 1})))
        })?;
        doc.update(|root| root.get_tree_mut("t")?.remove_style(0, 1, ["key"]))?;
        doc.update(|root| {
            root.get_tree_mut("t")?.edit(
                0,
                0,
                vec![element_with("p", json!({"bold": true}), vec![text("x")])],
                0,
            )
        })?;
        doc.update(|root| root.get_tree_mut("t")?.edit(0, 3, vec![], 0))?;
        assert_eq!(r#"<doc><p n="1"></p></doc>"#, xml(&doc));

        // JS "TreeChange" shapes: `value` holds the parsed attributes, or the
        // keys removed, or the inserted nodes.
        let style = |value: TreeStyleOpValue| OperationInfo::TreeStyle {
            path: "$.t".to_owned(),
            from: 0,
            to: 1,
            from_path: vec![0],
            to_path: vec![0, 0],
            value,
        };
        assert_eq!(
            vec![
                style(TreeStyleOpValue {
                    attributes: Some(attributes(json!({"key": "a", "n": 1}))),
                    attributes_to_remove: None,
                }),
                style(TreeStyleOpValue {
                    attributes: None,
                    attributes_to_remove: Some(vec!["key".to_owned()]),
                }),
                OperationInfo::TreeEdit {
                    path: "$.t".to_owned(),
                    from: 0,
                    to: 0,
                    value: Some(vec![element_with(
                        "p",
                        json!({"bold": true}),
                        vec![text("x")]
                    )]),
                    split_level: None,
                    from_path: vec![0],
                    to_path: vec![0],
                },
                OperationInfo::TreeEdit {
                    path: "$.t".to_owned(),
                    from: 0,
                    to: 3,
                    value: None,
                    split_level: None,
                    from_path: vec![0],
                    to_path: vec![1],
                },
            ],
            *ops.lock().unwrap()
        );
        Ok(())
    }
}
