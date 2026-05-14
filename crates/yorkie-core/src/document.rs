use crate::change::{Change, ChangeContext, ChangeId};
use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::crdt::root::CrdtRoot;
use crate::operation::{OpSource, Operation, RemoveOperation, SetOperation};
use crate::{JsonObject, JsonValue, Result, TimeTicket};

/// A local Yorkie document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    key: String,
    root: JsonObject,
    crdt_root: CrdtRoot,
    change_id: ChangeId,
    local_changes: Vec<Change>,
}

impl Document {
    /// Creates a document with the given Yorkie resource key.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            root: JsonObject::new(),
            crdt_root: CrdtRoot::create(),
            change_id: ChangeId::initial(),
            local_changes: Vec::new(),
        }
    }

    /// Returns this document's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Updates the document root.
    pub fn update<F>(&mut self, update_fn: F) -> Result<()>
    where
        F: FnOnce(&mut JsonObject) -> Result<()>,
    {
        let mut next_root = self.root.clone();
        update_fn(&mut next_root)?;

        let mut context = ChangeContext::create(self.change_id.clone(), None);
        collect_object_changes(
            &mut context,
            &self.crdt_root,
            &TimeTicket::initial(),
            &self.root,
            &next_root,
        )?;

        if context.has_change() {
            let change = context.to_change();
            change.execute(&mut self.crdt_root, OpSource::Local)?;
            self.local_changes.push(change);
            self.change_id = context.next_id();
        }

        self.root = next_root;
        Ok(())
    }

    /// Returns an immutable view of the document root.
    pub fn get_root(&self) -> &JsonObject {
        &self.root
    }

    /// Serializes the document root with object keys sorted lexicographically.
    pub fn to_sorted_json(&self) -> String {
        self.root.to_sorted_json()
    }
}

fn collect_object_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonObject,
    after: &JsonObject,
) -> Result<()> {
    for (key, _) in before.iter() {
        if after.get(key).is_none() {
            push_remove_operation(context, crdt_root, parent_created_at, key)?;
        }
    }

    for (key, after_value) in after.iter() {
        let Some(before_value) = before.get(key) else {
            push_set_or_remove_unsupported(
                context,
                crdt_root,
                parent_created_at,
                key,
                after_value,
            )?;
            continue;
        };

        if before_value == after_value {
            continue;
        }

        if let (JsonValue::Object(before_object), JsonValue::Object(after_object)) =
            (before_value, after_value)
        {
            if let Some(CrdtElement::Object(object)) =
                crdt_root.get_object_member(parent_created_at, key)?
            {
                collect_object_changes(
                    context,
                    crdt_root,
                    object.created_at(),
                    before_object,
                    after_object,
                )?;
                continue;
            }
        }

        push_set_or_remove_unsupported(context, crdt_root, parent_created_at, key, after_value)?;
    }

    Ok(())
}

fn push_set_or_remove_unsupported(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    key: &str,
    value: &JsonValue,
) -> Result<()> {
    if matches!(value, JsonValue::Array(_)) {
        push_remove_operation(context, crdt_root, parent_created_at, key)?;
        return Ok(());
    }

    let created_at = context.issue_time_ticket();
    let Some(element) = json_value_to_crdt_element(value, created_at.clone(), context)? else {
        push_remove_operation(context, crdt_root, parent_created_at, key)?;
        return Ok(());
    };

    context.push(Operation::Set(SetOperation::create(
        key.to_owned(),
        element,
        parent_created_at.clone(),
        Some(created_at),
    )));

    Ok(())
}

fn push_remove_operation(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    key: &str,
) -> Result<()> {
    let Some(element) = crdt_root.get_object_member(parent_created_at, key)? else {
        return Ok(());
    };

    let executed_at = context.issue_time_ticket();
    context.push(Operation::Remove(RemoveOperation::new(
        parent_created_at.clone(),
        element.created_at().clone(),
        Some(executed_at),
    )));

    Ok(())
}

fn json_value_to_crdt_element(
    value: &JsonValue,
    created_at: TimeTicket,
    context: &mut ChangeContext,
) -> Result<Option<CrdtElement>> {
    let element = match value {
        JsonValue::Null => primitive(PrimitiveValue::Null, created_at),
        JsonValue::Bool(value) => primitive(PrimitiveValue::Boolean(*value), created_at),
        JsonValue::Integer(value) => primitive(PrimitiveValue::Integer(*value), created_at),
        JsonValue::Long(value) => primitive(PrimitiveValue::Long(*value), created_at),
        JsonValue::Double(value) => primitive(PrimitiveValue::Double(*value), created_at),
        JsonValue::String(value) => primitive(PrimitiveValue::String(value.clone()), created_at),
        JsonValue::Object(value) => {
            let mut members = Vec::new();
            for (key, value) in value.iter() {
                if matches!(value, JsonValue::Array(_)) {
                    continue;
                }

                let member_created_at = context.issue_time_ticket();
                if let Some(element) =
                    json_value_to_crdt_element(value, member_created_at, context)?
                {
                    members.push((key.to_owned(), element));
                }
            }
            CrdtElement::object(CrdtObject::create_with_members(created_at, members))
        }
        JsonValue::Array(_) => return Ok(None),
    };

    Ok(Some(element))
}

fn primitive(value: PrimitiveValue, created_at: TimeTicket) -> CrdtElement {
    CrdtElement::primitive(CrdtPrimitive::new(value, created_at))
}

#[cfg(test)]
mod tests {
    use super::Document;
    use crate::{JsonObject, Result};

    #[test]
    fn creates_document_with_the_given_key() {
        let doc = Document::new("doc-key");
        assert_eq!("doc-key", doc.key());
    }

    #[test]
    fn records_supported_root_sets_as_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;

        assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
        assert_eq!(r#"{"title":"hello"}"#, doc.crdt_root.to_sorted_json());
        assert_eq!(1, doc.local_changes.len());
        assert_eq!(1, doc.change_id.client_seq());
        assert_eq!(1, doc.change_id.lamport());
        assert_eq!(
            "0:00:0.SET.title=\"hello\"",
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_supported_root_removes_as_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| {
            root.remove("title");
            Ok(())
        })?;

        assert_eq!("{}", doc.to_sorted_json());
        assert_eq!("{}", doc.crdt_root.to_sorted_json());
        assert_eq!(2, doc.local_changes.len());
        assert_eq!(2, doc.change_id.client_seq());
        assert_eq!(1, doc.crdt_root.get_garbage_len());
        assert_eq!(
            "0:00:0.REMOVE.1:00:1",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_member_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            Ok(())
        })?;
        doc.update(|root| {
            let profile = root.get_object_mut("profile")?;
            profile.set("active", true)?;
            profile.remove("name");
            Ok(())
        })?;

        assert_eq!(
            r#"{"profile":{"active":true}}"#,
            doc.crdt_root.to_sorted_json()
        );
        assert_eq!(
            "1:00:1.REMOVE.1:00:2,1:00:1.SET.active=true",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }
}
