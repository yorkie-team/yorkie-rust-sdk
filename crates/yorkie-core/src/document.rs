use crate::change::{Change, ChangeContext, ChangeId, ChangePack, Checkpoint};
use crate::crdt::array::CrdtArray;
use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::crdt::root::CrdtRoot;
use crate::operation::{OpSource, Operation, RemoveOperation, SetOperation};
use crate::{JsonObject, JsonValue, Result, TimeTicket, YorkieError};

/// A local Yorkie document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    key: String,
    root: JsonObject,
    crdt_root: CrdtRoot,
    checkpoint: Checkpoint,
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
            checkpoint: Checkpoint::initial(),
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

    /// Returns this document's checkpoint.
    pub fn checkpoint(&self) -> Checkpoint {
        self.checkpoint
    }

    /// Returns whether this document has local changes waiting to be synced.
    pub fn has_local_changes(&self) -> bool {
        !self.local_changes.is_empty()
    }

    /// Creates a pack of local changes to send to the remote.
    pub fn create_change_pack(&self) -> ChangePack {
        let changes = self.local_changes.clone();
        let checkpoint = self.checkpoint.increase_client_seq(changes.len() as u32);
        ChangePack::create(
            self.key.clone(),
            checkpoint,
            false,
            changes,
            Some(self.change_id.version_vector().clone()),
            None,
        )
    }

    /// Applies the given change pack to this document.
    pub fn apply_change_pack(&mut self, pack: &ChangePack) -> Result<()> {
        if pack.has_snapshot() {
            return Err(YorkieError::UnsupportedSnapshot);
        }

        self.apply_changes(pack.changes(), OpSource::Remote)?;
        self.remove_pushed_local_changes(pack.checkpoint().client_seq());

        self.checkpoint = self.checkpoint.forward(pack.checkpoint());
        Ok(())
    }

    /// Serializes the document root with object keys sorted lexicographically.
    pub fn to_sorted_json(&self) -> String {
        self.root.to_sorted_json()
    }

    fn apply_changes(&mut self, changes: &[Change], source: OpSource) -> Result<()> {
        for change in changes {
            change.execute(&mut self.crdt_root, source)?;
            self.change_id = self.change_id.sync_clocks(change.id());
        }

        self.root = crdt_object_to_json_object(self.crdt_root.object())?;
        Ok(())
    }

    fn remove_pushed_local_changes(&mut self, client_seq: u32) {
        while self
            .local_changes
            .first()
            .map(|change| change.id().client_seq() <= client_seq)
            .unwrap_or(false)
        {
            self.local_changes.remove(0);
        }
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
            push_set_or_remove_unsupported(context, parent_created_at, key, after_value)?;
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

        push_set_or_remove_unsupported(context, parent_created_at, key, after_value)?;
    }

    Ok(())
}

fn push_set_or_remove_unsupported(
    context: &mut ChangeContext,
    parent_created_at: &TimeTicket,
    key: &str,
    value: &JsonValue,
) -> Result<()> {
    let created_at = context.issue_time_ticket();
    let element = json_value_to_crdt_element(value, created_at.clone(), context)?;

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
) -> Result<CrdtElement> {
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
                let member_created_at = context.issue_time_ticket();
                let element = json_value_to_crdt_element(value, member_created_at, context)?;
                members.push((key.to_owned(), element));
            }
            CrdtElement::object(CrdtObject::create_with_members(created_at, members))
        }
        JsonValue::Array(value) => {
            let mut elements = Vec::new();
            for value in value.iter() {
                let element_created_at = context.issue_time_ticket();
                elements.push(json_value_to_crdt_element(
                    value,
                    element_created_at,
                    context,
                )?);
            }

            CrdtElement::array(CrdtArray::create_with_elements(created_at, elements)?)
        }
    };

    Ok(element)
}

fn primitive(value: PrimitiveValue, created_at: TimeTicket) -> CrdtElement {
    CrdtElement::primitive(CrdtPrimitive::new(value, created_at))
}

fn crdt_object_to_json_object(object: &CrdtObject) -> Result<JsonObject> {
    let mut json_object = JsonObject::new();

    for (key, element) in object.iter() {
        json_object.set(key, crdt_element_to_json_value(element)?)?;
    }

    Ok(json_object)
}

fn crdt_element_to_json_value(element: &CrdtElement) -> Result<JsonValue> {
    let value = match element {
        CrdtElement::Primitive(value) => value.to_json_value(),
        CrdtElement::Object(value) => JsonValue::Object(crdt_object_to_json_object(value)?),
        CrdtElement::Array(value) => {
            let mut array = crate::JsonArray::new();
            for element in value.iter() {
                array.push(crdt_element_to_json_value(element)?);
            }
            JsonValue::Array(array)
        }
        CrdtElement::Text(value) => JsonValue::Array(value.to_json_array()?),
    };

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{crdt_element_to_json_value, Document};
    use crate::change::ChangePack;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::text::CrdtText;
    use crate::{Checkpoint, JsonArray, JsonObject, Result, VersionVector, YorkieError};

    #[test]
    fn creates_document_with_the_given_key() {
        let doc = Document::new("doc-key");
        assert_eq!("doc-key", doc.key());
        assert_eq!(Checkpoint::initial(), doc.checkpoint());
        assert!(!doc.has_local_changes());
    }

    #[test]
    fn records_supported_root_sets_as_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;

        assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
        assert_eq!(r#"{"title":"hello"}"#, doc.crdt_root.to_sorted_json());
        assert_eq!(1, doc.local_changes.len());
        assert!(doc.has_local_changes());
        assert_eq!(1, doc.change_id.client_seq());
        assert_eq!(1, doc.change_id.lamport());
        assert_eq!(
            "0:00:0.SET.title=\"hello\"",
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn converts_crdt_text_to_public_json_array() -> Result<()> {
        let mut text = CrdtText::create(crate::TimeTicket::new(1, 0, "a"));
        text.edit_by_index(0, 0, "Hi", None, crate::TimeTicket::new(2, 0, "a"), None)?;

        let value = crdt_element_to_json_value(&CrdtElement::text(text))?;

        assert_eq!(r#"[{"val":"Hi"}]"#, value.to_sorted_json());
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

    #[test]
    fn records_arrays_as_crdt_elements() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;

            let mut todos = JsonArray::new();
            todos.push("write tests").push(profile);
            root.set("todos", todos)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"todos":["write tests",{"name":"yorkie"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(5, doc.crdt_root.get_element_map_size());
        assert_eq!(
            r#"0:00:0.SET.todos=["write tests",{"name":"yorkie"}]"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn creates_change_pack_from_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| root.set("done", false).map(|_| ()))?;

        let pack = doc.create_change_pack();

        assert_eq!("doc-key", pack.document_key());
        assert_eq!(Checkpoint::new(0, 2), pack.checkpoint());
        assert!(!pack.is_removed());
        assert!(pack.has_changes());
        assert_eq!(2, pack.change_size());
        assert_eq!(2, pack.changes().len());
        assert_eq!(2, pack.operations_len());
        assert_eq!(Checkpoint::initial(), doc.checkpoint());
        assert!(doc.has_local_changes());
        assert_eq!(
            Some(2),
            pack.version_vector()
                .and_then(|vector| vector.get(doc.change_id.actor_id().as_str()))
        );
        Ok(())
    }

    #[test]
    fn applies_ack_change_pack_and_removes_pushed_local_changes() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        doc.update(|root| root.set("done", false).map(|_| ()))?;

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(3, 1),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        );
        doc.apply_change_pack(&pack)?;

        assert_eq!(Checkpoint::new(3, 1), doc.checkpoint());
        assert!(doc.has_local_changes());
        assert_eq!(1, doc.local_changes.len());
        assert_eq!(2, doc.local_changes[0].id().client_seq());
        assert_eq!(r#"{"done":false,"title":"hello"}"#, doc.to_sorted_json());

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(4, 2),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            None,
        );
        doc.apply_change_pack(&pack)?;

        assert_eq!(Checkpoint::new(4, 2), doc.checkpoint());
        assert!(!doc.has_local_changes());
        Ok(())
    }

    #[test]
    fn applies_remote_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let remote_pack = ChangePack::create(
            "target-doc",
            Checkpoint::new(1, 0),
            false,
            source_pack.changes().to_vec(),
            source_pack.version_vector().cloned(),
            None,
        );

        target.apply_change_pack(&remote_pack)?;

        assert_eq!(Checkpoint::new(1, 0), target.checkpoint());
        assert_eq!(2, target.change_id.lamport());
        assert_eq!(r#"{"profile":{"name":"yorkie"}}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn applies_remote_array_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            let mut array = JsonArray::new();
            array.push("sync").push(false);
            root.set("items", array)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let remote_pack = ChangePack::create(
            "target-doc",
            Checkpoint::new(1, 0),
            false,
            source_pack.changes().to_vec(),
            source_pack.version_vector().cloned(),
            None,
        );

        target.apply_change_pack(&remote_pack)?;

        assert_eq!(r#"{"items":["sync",false]}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn reports_snapshot_change_pack_until_snapshot_apply_is_supported() {
        let mut doc = Document::new("doc-key");
        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(1, 0),
            false,
            Vec::new(),
            Some(VersionVector::new()),
            Some(vec![1]),
        );

        let err = doc.apply_change_pack(&pack).unwrap_err();

        assert_eq!(YorkieError::UnsupportedSnapshot, err);
        assert_eq!(Checkpoint::initial(), doc.checkpoint());
    }
}
