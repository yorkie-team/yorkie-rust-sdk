//! Tests for `validate_yorkie_ruleset`.
//!
//! JS reference: `test/unit/schema/ruleset_validator_test.ts`. JS fills a
//! document and validates its root; these build the CRDT root directly, as
//! JS `tree_schema_integration_test.ts` does, so every primitive type
//! (including date and bytes) can be set.

use super::{validate_yorkie_ruleset, SchemaRule, TreeNodeRule, ValidationError};
use crate::crdt::array::CrdtArray;
use crate::crdt::counter::{CounterType, CounterValue, CrdtCounter};
use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::crdt::text::CrdtText;
use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
use crate::{TimeTicket, YorkieError};
use std::cell::Cell;

thread_local! {
    static LAMPORT: Cell<i64> = const { Cell::new(0) };
}

fn ticket() -> TimeTicket {
    let lamport = LAMPORT.with(|lamport| {
        lamport.set(lamport.get() + 1);
        lamport.get()
    });
    TimeTicket::new(lamport, 0, "000000000000000000000000")
}

fn rule(path: &str, rule_type: &str) -> SchemaRule {
    SchemaRule::new(path, rule_type, Vec::new())
}

fn primitive(value: PrimitiveValue) -> CrdtElement {
    CrdtElement::primitive(CrdtPrimitive::new(value, ticket()))
}

fn object(members: Vec<(&str, CrdtElement)>) -> CrdtObject {
    let mut object = CrdtObject::create(ticket());
    for (key, value) in members {
        object.set(key, value, ticket());
    }
    object
}

fn errors(root: &CrdtObject, ruleset: &[SchemaRule]) -> Vec<(String, String)> {
    validate_yorkie_ruleset(root, ruleset)
        .unwrap()
        .into_iter()
        .map(|ValidationError { path, message }| (path, message))
        .collect()
}

fn expected(path: &str, rule_type: &str) -> (String, String) {
    (
        path.to_owned(),
        format!("expected {rule_type} at path {path}"),
    )
}

#[test]
fn validates_primitive_types() {
    let types = [
        "null", "boolean", "integer", "double", "long", "string", "date", "bytes",
    ];
    let mut ruleset = vec![rule("$", "object")];
    ruleset.extend(
        types
            .iter()
            .enumerate()
            .map(|(i, t)| rule(&format!("$.field{}", i + 1), t)),
    );

    let values = [
        PrimitiveValue::Null,
        PrimitiveValue::Boolean(true),
        PrimitiveValue::Integer(123),
        PrimitiveValue::Double(123.456),
        PrimitiveValue::Long(i64::MAX),
        PrimitiveValue::String("test".to_owned()),
        PrimitiveValue::Date(1_700_000_000_000),
        PrimitiveValue::Bytes(vec![1, 2, 3]),
    ];
    let fields = |values: Vec<PrimitiveValue>| {
        let names: Vec<String> = (1..=values.len()).map(|i| format!("field{i}")).collect();
        let members = names
            .iter()
            .map(String::as_str)
            .zip(values.into_iter().map(primitive))
            .collect();
        object(members)
    };

    assert!(errors(&fields(values.to_vec()), &ruleset).is_empty());

    // Each field takes the next field's value, so none has its own type.
    let mut shifted = values.to_vec();
    shifted.rotate_left(1);
    assert_eq!(
        types
            .iter()
            .enumerate()
            .map(|(i, t)| expected(&format!("$.field{}", i + 1), t))
            .collect::<Vec<_>>(),
        errors(&fields(shifted), &ruleset)
    );
}

#[test]
fn validates_object_type() {
    let ruleset = [rule("$.user", "object")];
    let root = object(vec![(
        "user",
        CrdtElement::object(object(vec![(
            "name",
            primitive(PrimitiveValue::String("test".to_owned())),
        )])),
    )]);
    assert!(errors(&root, &ruleset).is_empty());

    let root = object(vec![(
        "user",
        primitive(PrimitiveValue::String("not an object".to_owned())),
    )]);
    assert_eq!(vec![expected("$.user", "object")], errors(&root, &ruleset));
}

#[test]
fn validates_array_type() {
    let ruleset = [rule("$.items", "array")];
    let root = object(vec![(
        "items",
        CrdtElement::array(CrdtArray::create(ticket())),
    )]);
    assert!(errors(&root, &ruleset).is_empty());

    let root = object(vec![(
        "items",
        primitive(PrimitiveValue::String("not an array".to_owned())),
    )]);
    assert_eq!(vec![expected("$.items", "array")], errors(&root, &ruleset));
}

#[test]
fn validates_nested_paths() {
    let ruleset = [
        rule("$", "object"),
        rule("$.user", "object"),
        rule("$.user.name", "string"),
        rule("$.user.age", "integer"),
    ];
    let user = |name: PrimitiveValue| {
        object(vec![(
            "user",
            CrdtElement::object(object(vec![
                ("name", primitive(name)),
                ("age", primitive(PrimitiveValue::Integer(25))),
            ])),
        )])
    };
    assert!(errors(&user(PrimitiveValue::String("test".to_owned())), &ruleset).is_empty());
    assert_eq!(
        vec![expected("$.user.name", "string")],
        errors(&user(PrimitiveValue::Integer(123)), &ruleset)
    );
}

#[test]
fn reports_missing_values_as_type_mismatches() {
    let ruleset = [rule("$.title", "string"), rule("$.title.inner", "object")];
    assert_eq!(
        vec![
            expected("$.title", "string"),
            expected("$.title.inner", "object")
        ],
        errors(&object(Vec::new()), &ruleset)
    );
}

#[test]
fn validates_yorkie_types() {
    let ruleset = [
        rule("$", "object"),
        rule("$.text", "yorkie.Text"),
        rule("$.tree", "yorkie.Tree"),
        rule("$.counter", "yorkie.Counter"),
    ];
    let tree = CrdtTree::new(
        TreeNode::create_element(TreeNodeId::new(ticket(), 0), "doc", None, Vec::new()),
        ticket(),
    );
    let root = object(vec![
        ("text", CrdtElement::text(CrdtText::create(ticket()))),
        ("tree", CrdtElement::tree(tree)),
        (
            "counter",
            CrdtElement::counter(CrdtCounter::create(
                CounterType::Integer,
                CounterValue::Integer(0),
                ticket(),
            )),
        ),
    ]);
    assert!(errors(&root, &ruleset).is_empty());

    let root = object(vec![
        ("text", primitive(PrimitiveValue::String("text".to_owned()))),
        ("tree", primitive(PrimitiveValue::String("doc".to_owned()))),
        ("counter", primitive(PrimitiveValue::Integer(1))),
    ]);
    assert_eq!(
        vec![
            expected("$.text", "yorkie.Text"),
            expected("$.tree", "yorkie.Tree"),
            expected("$.counter", "yorkie.Counter"),
        ],
        errors(&root, &ruleset)
    );
}

#[test]
fn reports_tree_node_rule_violations_at_the_tree_path() {
    let ruleset = [SchemaRule::new(
        "$.content",
        "yorkie.Tree",
        vec![TreeNodeRule::new("doc", "paragraph+", "", "")],
    )];
    let tree = CrdtTree::new(
        TreeNode::create_element(TreeNodeId::new(ticket(), 0), "doc", None, Vec::new()),
        ticket(),
    );
    let root = object(vec![("content", CrdtElement::tree(tree))]);
    assert_eq!(
        vec![(
            "$.content".to_owned(),
            "Node \"doc\": Children do not match content expression".to_owned()
        )],
        errors(&root, &ruleset)
    );
}

#[test]
fn rejects_rules_it_cannot_apply() {
    let root = object(Vec::new());
    for (ruleset, message) in [
        (
            vec![rule("title", "string")],
            "Path must start with $, got title",
        ),
        (vec![rule("$.title", "uuid")], "Unknown rule type: uuid"),
    ] {
        assert_eq!(
            Err(YorkieError::InvalidArgument(message.to_owned())),
            validate_yorkie_ruleset(&root, &ruleset)
        );
    }
}
