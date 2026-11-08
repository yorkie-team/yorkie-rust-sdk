//! Tree schema validation, mirroring JS
//! `src/document/schema/tree-validator.ts`.

use super::content_expression::{match_content_expression, parse_content_expression};
use super::TreeNodeRule;
use crate::crdt::tree::{CrdtTree, TreeNode};
use crate::Result;
use std::collections::BTreeMap;

/// Builds a resolver that turns a group name into the node types in that
/// group, and any other name into itself (JS `buildGroupResolver`).
pub(crate) fn build_group_resolver(tree_nodes: &[TreeNodeRule]) -> impl Fn(&str) -> Vec<String> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in tree_nodes {
        for group in node.group.split_whitespace() {
            groups
                .entry(group.to_owned())
                .or_default()
                .push(node.node_type.clone());
        }
    }

    move |name| {
        groups
            .get(name)
            .cloned()
            .unwrap_or_else(|| vec![name.to_owned()])
    }
}

/// Validates a tree's structure against tree node rules and returns the
/// first violation, if any (JS `validateTreeAgainstSchema`; `None` is
/// `valid: true`). It checks that every node type has a rule, that the
/// children of each element node match its content expression, and that
/// text children carry only the marks their parent allows.
///
/// A content expression that does not parse is an error, where JS throws.
pub(crate) fn validate_tree_against_schema(
    tree: &CrdtTree,
    tree_nodes: &[TreeNodeRule],
) -> Result<Option<String>> {
    let rules: BTreeMap<&str, &TreeNodeRule> = tree_nodes
        .iter()
        .map(|rule| (rule.node_type.as_str(), rule))
        .collect();
    let resolver = build_group_resolver(tree_nodes);
    validate_node(tree.root(), &rules, &resolver)
}

/// Validates a node and the element nodes under it (JS `validateNode`).
fn validate_node(
    node: &TreeNode,
    rules: &BTreeMap<&str, &TreeNodeRule>,
    resolver: &dyn Fn(&str) -> Vec<String>,
) -> Result<Option<String>> {
    let Some(rule) = rules.get(node.node_type()) else {
        return Ok(Some(unknown_node_type(node)));
    };

    // A text node is a leaf; its parent's marks rule covers it.
    if node.is_text() {
        return Ok(None);
    }

    let children: Vec<&TreeNode> = node.children().collect();
    if let Some(child) = children
        .iter()
        .find(|child| !child.is_text() && !rules.contains_key(child.node_type()))
    {
        return Ok(Some(unknown_node_type(child)));
    }

    // An empty content expression allows no children.
    let child_types: Vec<&str> = children.iter().map(|child| child.node_type()).collect();
    let expr = parse_content_expression(&rule.content)?;
    if let Some(error) = match_content_expression(&expr, &child_types, resolver) {
        return Ok(Some(format!("Node \"{}\": {error}", node.node_type())));
    }

    if !rule.marks.is_empty() {
        let allowed: Vec<&str> = rule.marks.split_whitespace().collect();
        if let Some(error) = validate_child_marks(node, &children, &allowed) {
            return Ok(Some(error));
        }
    }

    for child in children.into_iter().filter(|child| !child.is_text()) {
        if let Some(error) = validate_node(child, rules, resolver)? {
            return Ok(Some(error));
        }
    }
    Ok(None)
}

fn unknown_node_type(node: &TreeNode) -> String {
    format!("Unknown node type: \"{}\"", node.node_type())
}

/// Checks that the text children of `parent` carry only `allowed` marks (JS
/// `validateChildMarks`). Marks are attribute keys; removed ones are
/// skipped.
///
/// When several marks are disallowed, the one reported is the first by key;
/// JS reports the first set.
fn validate_child_marks(
    parent: &TreeNode,
    children: &[&TreeNode],
    allowed: &[&str],
) -> Option<String> {
    children
        .iter()
        .filter(|child| child.is_text())
        .filter_map(|child| child.attrs())
        .flat_map(|attrs| attrs.iter())
        .filter(|attr| !attr.is_removed())
        .find(|attr| !allowed.contains(&attr.key()))
        .map(|attr| {
            format!(
                "Node \"{}\": text child has disallowed mark \"{}\". Allowed marks: {}",
                parent.node_type(),
                attr.key(),
                allowed.join(", ")
            )
        })
}

/// JS reference: `test/unit/document/schema/tree_validator_test.ts`.
#[cfg(test)]
mod tests {
    use super::{build_group_resolver, validate_tree_against_schema};
    use crate::crdt::rht::Rht;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::schema::TreeNodeRule;
    use crate::TimeTicket;
    use std::cell::Cell;

    thread_local! {
        static LAMPORT: Cell<i64> = const { Cell::new(0) };
    }

    /// A fresh ticket for each node, like JS `posT`/`timeT`.
    fn ticket() -> TimeTicket {
        let lamport = LAMPORT.with(|lamport| {
            lamport.set(lamport.get() + 1);
            lamport.get()
        });
        TimeTicket::new(lamport, 0, "000000000000000000000000")
    }

    fn text(value: &str) -> TreeNode {
        TreeNode::create_text(TreeNodeId::new(ticket(), 0), value)
    }

    fn marked_text(value: &str, marks: &[&str]) -> TreeNode {
        let mut attrs = Rht::new();
        for mark in marks {
            attrs.set(*mark, "\"true\"", ticket());
        }
        TreeNode::new(
            TreeNodeId::new(ticket(), 0),
            "text",
            Some(attrs),
            value,
            Vec::new(),
        )
    }

    fn element(node_type: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode::create_element(TreeNodeId::new(ticket(), 0), node_type, None, children)
    }

    fn rule(node_type: &str, content: &str, marks: &str, group: &str) -> TreeNodeRule {
        TreeNodeRule::new(node_type, content, marks, group)
    }

    fn doc_rules() -> Vec<TreeNodeRule> {
        vec![
            rule("doc", "paragraph+", "", ""),
            rule("paragraph", "text*", "", ""),
        ]
    }

    fn validate(root: TreeNode, rules: &[TreeNodeRule]) -> Option<String> {
        validate_tree_against_schema(&CrdtTree::new(root, ticket()), rules).unwrap()
    }

    #[test]
    fn resolves_group_names_to_node_types() {
        let resolver = build_group_resolver(&[
            rule("paragraph", "text*", "", "block"),
            rule("heading", "text*", "", "block"),
            rule("blockquote", "block+", "", "block"),
        ]);
        assert_eq!(
            vec!["paragraph", "heading", "blockquote"],
            resolver("block")
        );
    }

    #[test]
    fn resolves_other_names_to_themselves() {
        let resolver = build_group_resolver(&[rule("paragraph", "text*", "", "block")]);
        assert_eq!(vec!["paragraph"], resolver("paragraph"));
        assert_eq!(vec!["unknown"], resolver("unknown"));

        let resolver = build_group_resolver(&[rule("paragraph", "text*", "", "")]);
        assert_eq!(vec!["paragraph"], resolver("paragraph"));
    }

    #[test]
    fn resolves_nodes_in_several_groups() {
        let resolver = build_group_resolver(&[rule("paragraph", "text*", "", "block flow")]);
        assert_eq!(vec!["paragraph"], resolver("block"));
        assert_eq!(vec!["paragraph"], resolver("flow"));
    }

    #[test]
    fn accepts_valid_trees() {
        let tree = element("doc", vec![element("paragraph", vec![text("hello")])]);
        assert_eq!(None, validate(tree, &doc_rules()));

        let tree = element(
            "doc",
            vec![
                element("paragraph", vec![text("hello")]),
                element("paragraph", vec![text("world")]),
            ],
        );
        assert_eq!(None, validate(tree, &doc_rules()));
    }

    #[test]
    fn rejects_unknown_node_types() {
        let tree = element("doc", vec![element("div", vec![text("hello")])]);
        assert_eq!(
            Some("Unknown node type: \"div\"".to_owned()),
            validate(tree, &doc_rules())
        );
    }

    #[test]
    fn rejects_children_that_break_the_content_expression() {
        assert_eq!(
            Some("Node \"doc\": Children do not match content expression".to_owned()),
            validate(element("doc", vec![]), &doc_rules())
        );

        let rules = [
            rule("doc", "paragraph+", "", ""),
            rule("paragraph", "text*", "", ""),
            rule("heading", "text*", "", ""),
        ];
        let tree = element("doc", vec![element("heading", vec![text("hello")])]);
        let error = validate(tree, &rules).unwrap();
        assert!(error.contains("doc"), "{error}");
    }

    #[test]
    fn resolves_groups_in_content_expressions() {
        let rules = [
            rule("doc", "block+", "", ""),
            rule("paragraph", "text*", "", "block"),
            rule("heading", "text*", "", "block"),
        ];
        let tree = element(
            "doc",
            vec![
                element("paragraph", vec![text("hello")]),
                element("heading", vec![text("world")]),
            ],
        );
        assert_eq!(None, validate(tree, &rules));
    }

    #[test]
    fn accepts_an_empty_content_expression_without_children() {
        let rules = [
            rule("doc", "paragraph+", "", ""),
            rule("paragraph", "", "", ""),
        ];
        let tree = element("doc", vec![element("paragraph", vec![])]);
        assert_eq!(None, validate(tree, &rules));
    }

    #[test]
    fn validates_marks_on_text_children() {
        let rules = [
            rule("doc", "heading+", "", ""),
            rule("heading", "text*", "bold", ""),
        ];
        let tree = element(
            "doc",
            vec![element("heading", vec![marked_text("hello", &["bold"])])],
        );
        assert_eq!(None, validate(tree, &rules));

        let tree = element(
            "doc",
            vec![element("heading", vec![marked_text("hello", &["italic"])])],
        );
        assert_eq!(
            Some(
                "Node \"heading\": text child has disallowed mark \"italic\". Allowed marks: bold"
                    .to_owned()
            ),
            validate(tree, &rules)
        );
    }

    #[test]
    fn accepts_several_allowed_marks() {
        let rules = [
            rule("doc", "paragraph+", "", ""),
            rule("paragraph", "text*", "bold italic underline", ""),
        ];
        let tree = element(
            "doc",
            vec![element(
                "paragraph",
                vec![marked_text("hello", &["bold", "italic"])],
            )],
        );
        assert_eq!(None, validate(tree, &rules));
    }

    #[test]
    fn skips_mark_validation_without_a_marks_rule() {
        let tree = element(
            "doc",
            vec![element("paragraph", vec![marked_text("hello", &["bold"])])],
        );
        assert_eq!(None, validate(tree, &doc_rules()));
    }

    #[test]
    fn validates_nested_trees() {
        let rules = [
            rule("doc", "section+", "", ""),
            rule("section", "paragraph+", "", ""),
            rule("paragraph", "text*", "", ""),
        ];
        let tree = element(
            "doc",
            vec![element(
                "section",
                vec![element("paragraph", vec![text("hello")])],
            )],
        );
        assert_eq!(None, validate(tree, &rules));

        let tree = element("doc", vec![element("section", vec![])]);
        assert_eq!(
            Some("Node \"section\": Children do not match content expression".to_owned()),
            validate(tree, &rules)
        );
    }

    #[test]
    fn accepts_alternative_content_expressions() {
        let rules = [
            rule("doc", "(paragraph | heading)+", "", ""),
            rule("paragraph", "text*", "", ""),
            rule("heading", "text*", "", ""),
        ];
        let tree = element(
            "doc",
            vec![
                element("paragraph", vec![text("hello")]),
                element("heading", vec![text("title")]),
            ],
        );
        assert_eq!(None, validate(tree, &rules));
    }
}
