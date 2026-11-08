//! Document schema rules and their validation, mirroring JS
//! `src/document/schema/ruleset_validator.ts`. `Document::update` checks
//! each local change against the rules (see `validate_yorkie_ruleset`).

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

mod content_expression;
mod tree_validator;

use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::{Result, YorkieError};

/// A rule a document breaks (JS `ValidationError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationError {
    pub(crate) path: String,
    pub(crate) message: String,
}

/// Validates the document root against `ruleset` and returns what it breaks,
/// in rule order (JS `validateYorkieRuleset`; an empty list is JS
/// `valid: true`).
///
/// A rule that cannot be applied at all (a path not starting with `$`, an
/// unknown rule type, or a tree content expression that does not parse) is
/// an error, where JS throws.
pub(crate) fn validate_yorkie_ruleset(
    root: &CrdtObject,
    ruleset: &[SchemaRule],
) -> Result<Vec<ValidationError>> {
    let mut errors = Vec::new();
    for rule in ruleset {
        let value = value_by_path(root, &rule.path)?;
        if let Some(message) = validate_value(value, rule)? {
            errors.push(ValidationError {
                path: rule.path.clone(),
                message,
            });
        }
    }
    Ok(errors)
}

/// A value a rule path names: the root object or an element under it.
#[derive(Clone, Copy)]
enum PathValue<'a> {
    Root(&'a CrdtObject),
    Element(&'a CrdtElement),
}

impl<'a> PathValue<'a> {
    fn as_object(self) -> Option<&'a CrdtObject> {
        match self {
            Self::Root(object) => Some(object),
            Self::Element(CrdtElement::Object(object)) => Some(object),
            Self::Element(_) => None,
        }
    }

    fn as_element(self) -> Option<&'a CrdtElement> {
        match self {
            Self::Root(_) => None,
            Self::Element(element) => Some(element),
        }
    }
}

/// Follows `path` (`$.a.b`) from the root through object members (JS
/// `getValueByPath`). A missing member, or a step into something that is
/// not an object, yields `None`.
fn value_by_path<'a>(root: &'a CrdtObject, path: &str) -> Result<Option<PathValue<'a>>> {
    if !path.starts_with('$') {
        return Err(YorkieError::InvalidArgument(format!(
            "Path must start with $, got {path}"
        )));
    }

    let mut current = Some(PathValue::Root(root));
    for key in path.split('.').skip(1) {
        current = current
            .and_then(PathValue::as_object)
            .and_then(|object| object.get(key))
            .map(PathValue::Element);
    }
    Ok(current)
}

/// Checks the value at a rule's path against the rule and returns the
/// message of the violation, if any (JS `validateValue`).
fn validate_value(value: Option<PathValue<'_>>, rule: &SchemaRule) -> Result<Option<String>> {
    let element = value.and_then(PathValue::as_element);
    let valid = match rule.rule_type.as_str() {
        "string" | "boolean" | "integer" | "double" | "long" | "date" | "bytes" | "null" => {
            matches!(
                element,
                Some(CrdtElement::Primitive(primitive))
                    if primitive.primitive_type().name() == rule.rule_type
            )
        }
        "object" => value.and_then(PathValue::as_object).is_some(),
        "array" => matches!(element, Some(CrdtElement::Array(_))),
        "yorkie.Text" => matches!(element, Some(CrdtElement::Text(_))),
        "yorkie.Tree" => {
            let Some(CrdtElement::Tree(tree)) = element else {
                return Ok(Some(type_mismatch(rule)));
            };
            if !rule.tree_nodes.is_empty() {
                if let Some(message) =
                    tree_validator::validate_tree_against_schema(tree, &rule.tree_nodes)?
                {
                    return Ok(Some(message));
                }
            }
            true
        }
        "yorkie.Counter" => matches!(element, Some(CrdtElement::Counter(_))),
        other => {
            return Err(YorkieError::InvalidArgument(format!(
                "Unknown rule type: {other}"
            )))
        }
    };

    Ok((!valid).then(|| type_mismatch(rule)))
}

fn type_mismatch(rule: &SchemaRule) -> String {
    format!("expected {} at path {}", rule.rule_type, rule.path)
}

#[cfg(test)]
mod tests;
