/// A tree node rule inside a document schema rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNodeRule {
    pub node_type: String,
    pub content: String,
    pub marks: String,
    pub group: String,
}

impl TreeNodeRule {
    pub fn new(
        node_type: impl Into<String>,
        content: impl Into<String>,
        marks: impl Into<String>,
        group: impl Into<String>,
    ) -> Self {
        Self {
            node_type: node_type.into(),
            content: content.into(),
            marks: marks.into(),
            group: group.into(),
        }
    }
}

/// A document schema rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRule {
    pub path: String,
    pub rule_type: String,
    pub tree_nodes: Vec<TreeNodeRule>,
}

impl SchemaRule {
    pub fn new(
        path: impl Into<String>,
        rule_type: impl Into<String>,
        tree_nodes: Vec<TreeNodeRule>,
    ) -> Self {
        Self {
            path: path.into(),
            rule_type: rule_type.into(),
            tree_nodes,
        }
    }
}
