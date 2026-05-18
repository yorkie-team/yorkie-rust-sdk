use crate::change::{Change, ChangeContext, ChangeId, ChangePack, Checkpoint};
use crate::crdt::array::CrdtArray;
use crate::crdt::counter::CounterValue;
use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::crdt::root::CrdtRoot;
use crate::crdt::tree::{attribute_value_to_json_value, TreeNode};
use crate::json::{counter_operand, JsonEditRecorder, JsonEditRecorderRef, RecordedJsonValue};
use crate::operation::{
    AddOperation, ArraySetOperation, IncreaseOperation, MoveOperation, OpSource, Operation,
    RemoveOperation, SetOperation,
};
use crate::{JsonArray, JsonObject, JsonValue, Result, TimeTicket, YorkieError};
use std::cell::RefCell;
use std::rc::Rc;

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
        let recorder = Rc::new(RefCell::new(DocumentEditRecorder::new(
            self.change_id.clone(),
        )));
        let recorder_ref: JsonEditRecorderRef = recorder.clone();
        next_root.attach_recorder(recorder_ref);

        update_fn(&mut next_root)?;

        let mut context = recorder.borrow().context.clone();
        if !context.has_change() {
            collect_object_changes(
                &mut context,
                &self.crdt_root,
                &TimeTicket::initial(),
                &self.root,
                &next_root,
            )?;
        }

        if context.has_change() {
            let change = context.to_change();
            change.execute(&mut self.crdt_root, OpSource::Local)?;
            self.local_changes.push(change);
            self.change_id = context.next_id();
            self.root = crdt_object_to_json_object(self.crdt_root.object())?;
            return Ok(());
        }

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
            self.apply_snapshot(pack)?;
        } else {
            self.apply_changes(pack.changes(), OpSource::Remote)?;
            self.remove_pushed_local_changes(pack.checkpoint().client_seq());
        }

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

    fn apply_snapshot(&mut self, pack: &ChangePack) -> Result<()> {
        let snapshot_root = pack
            .snapshot_root()
            .ok_or(YorkieError::UnsupportedSnapshot)?
            .clone();

        self.crdt_root = CrdtRoot::new(snapshot_root);
        self.root = crdt_object_to_json_object(self.crdt_root.object())?;

        if let Some(version_vector) = pack.version_vector() {
            self.change_id = self
                .change_id
                .set_clocks(version_vector.max_lamport(), version_vector.clone());
        }

        self.remove_pushed_local_changes(pack.checkpoint().client_seq());
        let local_changes = self.local_changes.clone();
        self.apply_changes(&local_changes, OpSource::Local)?;
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

#[derive(Debug, Clone)]
struct DocumentEditRecorder {
    context: ChangeContext,
}

impl DocumentEditRecorder {
    fn new(change_id: ChangeId) -> Self {
        Self {
            context: ChangeContext::create(change_id, None),
        }
    }

    fn record_new_value(&mut self, value: &JsonValue) -> Result<(TimeTicket, CrdtElement)> {
        let created_at = self.context.issue_time_ticket();
        let element = json_value_to_crdt_element(value, created_at.clone(), &mut self.context)?;
        Ok((created_at, element))
    }
}

impl JsonEditRecorder for DocumentEditRecorder {
    fn record_object_set(
        &mut self,
        parent_created_at: &TimeTicket,
        key: &str,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue> {
        let (created_at, element) = self.record_new_value(value)?;
        self.context.push(Operation::Set(SetOperation::create(
            key.to_owned(),
            element.deepcopy(),
            parent_created_at.clone(),
            Some(created_at.clone()),
        )));

        recorded_json_value_from_crdt_element(&element)
    }

    fn record_object_remove(
        &mut self,
        parent_created_at: &TimeTicket,
        _key: &str,
        created_at: &TimeTicket,
    ) {
        let executed_at = self.context.issue_time_ticket();
        self.context.push(Operation::Remove(RemoveOperation::new(
            parent_created_at.clone(),
            created_at.clone(),
            Some(executed_at),
        )));
    }

    fn record_array_insert(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue> {
        let (created_at, element) = self.record_new_value(value)?;
        self.context.push(Operation::Add(AddOperation::create(
            parent_created_at.clone(),
            prev_created_at.clone(),
            element.deepcopy(),
            Some(created_at.clone()),
        )));

        recorded_json_value_from_crdt_element(&element)
    }

    fn record_array_set(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue> {
        let (executed_at, element) = self.record_new_value(value)?;
        self.context
            .push(Operation::ArraySet(ArraySetOperation::create(
                parent_created_at.clone(),
                created_at.clone(),
                element.deepcopy(),
                Some(executed_at.clone()),
            )));

        recorded_json_value_from_crdt_element(&element)
    }

    fn record_array_remove(&mut self, parent_created_at: &TimeTicket, created_at: &TimeTicket) {
        let executed_at = self.context.issue_time_ticket();
        self.context.push(Operation::Remove(RemoveOperation::new(
            parent_created_at.clone(),
            created_at.clone(),
            Some(executed_at),
        )));
    }

    fn record_array_move(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> TimeTicket {
        let executed_at = self.context.issue_time_ticket();
        self.context.push(Operation::Move(MoveOperation::create(
            parent_created_at.clone(),
            prev_created_at.clone(),
            created_at.clone(),
            Some(executed_at.clone()),
        )));
        executed_at
    }

    fn record_counter_increase(
        &mut self,
        parent_created_at: &TimeTicket,
        value: CounterValue,
        actor: Option<&str>,
    ) -> Result<()> {
        let executed_at = self.context.issue_time_ticket();
        let value = CrdtElement::primitive(counter_operand(value, executed_at.clone()));
        let operation = if let Some(actor) = actor {
            IncreaseOperation::create_with_actor(
                parent_created_at.clone(),
                value,
                Some(executed_at),
                actor,
            )
        } else {
            IncreaseOperation::create(parent_created_at.clone(), value, Some(executed_at))
        };

        self.context.push(Operation::Increase(operation));
        Ok(())
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

        if let (JsonValue::Array(before_array), JsonValue::Array(after_array)) =
            (before_value, after_value)
        {
            if let Some(CrdtElement::Array(array)) =
                crdt_root.get_object_member(parent_created_at, key)?
            {
                collect_array_changes(
                    context,
                    crdt_root,
                    array.created_at(),
                    before_array,
                    after_array,
                )?;
                continue;
            }
        }

        push_set_or_remove_unsupported(context, parent_created_at, key, after_value)?;
    }

    Ok(())
}

fn collect_array_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonArray,
    after: &JsonArray,
) -> Result<()> {
    if before == after {
        return Ok(());
    }

    if before.len() == after.len() {
        let pairs = lcs_pairs(before.as_slice(), after.as_slice());
        if pairs
            .iter()
            .any(|(before_index, after_index)| before_index != after_index)
        {
            return collect_sequence_array_changes(
                context,
                crdt_root,
                parent_created_at,
                before,
                after,
            );
        }

        collect_same_length_array_changes(context, crdt_root, parent_created_at, before, after)?;
        return Ok(());
    }

    collect_sequence_array_changes(context, crdt_root, parent_created_at, before, after)
}

fn collect_same_length_array_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonArray,
    after: &JsonArray,
) -> Result<()> {
    for index in 0..before.len() {
        let before_value = before.get(index).ok_or_else(|| {
            YorkieError::InvalidIndex(format!("missing before array index {index}"))
        })?;
        let after_value = after.get(index).ok_or_else(|| {
            YorkieError::InvalidIndex(format!("missing after array index {index}"))
        })?;
        if before_value == after_value {
            continue;
        }

        let current = crdt_root
            .array_by_created_at(parent_created_at)
            .and_then(|array| array.get(index))
            .ok_or_else(|| {
                YorkieError::InvalidIndex(format!("missing CRDT array index {index}"))
            })?;

        match (before_value, after_value, current) {
            (
                JsonValue::Object(before_object),
                JsonValue::Object(after_object),
                CrdtElement::Object(object),
            ) => {
                collect_object_changes(
                    context,
                    crdt_root,
                    object.created_at(),
                    before_object,
                    after_object,
                )?;
            }
            (
                JsonValue::Array(before_array),
                JsonValue::Array(after_array),
                CrdtElement::Array(array),
            ) => {
                collect_array_changes(
                    context,
                    crdt_root,
                    array.created_at(),
                    before_array,
                    after_array,
                )?;
            }
            _ => {
                push_array_set_operation(
                    context,
                    parent_created_at,
                    current.created_at().clone(),
                    after_value,
                )?;
            }
        }
    }

    Ok(())
}

fn collect_sequence_array_changes(
    context: &mut ChangeContext,
    crdt_root: &CrdtRoot,
    parent_created_at: &TimeTicket,
    before: &JsonArray,
    after: &JsonArray,
) -> Result<()> {
    let pairs = lcs_pairs(before.as_slice(), after.as_slice());
    let mut before_matched = vec![false; before.len()];
    let mut after_created_at = vec![None; after.len()];

    for (before_index, after_index) in pairs {
        before_matched[before_index] = true;
        let created_at = crdt_root
            .array_by_created_at(parent_created_at)
            .and_then(|array| array.get(before_index))
            .map(CrdtElement::created_at)
            .ok_or_else(|| {
                YorkieError::InvalidIndex(format!("missing CRDT array index {before_index}"))
            })?
            .clone();
        after_created_at[after_index] = Some(created_at);
    }

    for (index, matched) in before_matched.iter().enumerate() {
        if *matched {
            continue;
        }

        let created_at = crdt_root
            .array_by_created_at(parent_created_at)
            .and_then(|array| array.get(index))
            .map(CrdtElement::created_at)
            .ok_or_else(|| YorkieError::InvalidIndex(format!("missing CRDT array index {index}")))?
            .clone();
        let executed_at = context.issue_time_ticket();
        context.push(Operation::Remove(RemoveOperation::new(
            parent_created_at.clone(),
            created_at,
            Some(executed_at),
        )));
    }

    let mut prev_created_at = TimeTicket::initial();
    for (index, after_value) in after.as_slice().iter().enumerate() {
        if let Some(created_at) = &after_created_at[index] {
            prev_created_at = crdt_root
                .array_by_created_at(parent_created_at)
                .and_then(|array| array.pos_created_at(created_at).ok())
                .unwrap_or_else(|| created_at.clone());
            continue;
        }

        let created_at = context.issue_time_ticket();
        let element = json_value_to_crdt_element(after_value, created_at.clone(), context)?;
        context.push(Operation::Add(AddOperation::create(
            parent_created_at.clone(),
            prev_created_at,
            element,
            Some(created_at.clone()),
        )));
        prev_created_at = created_at;
    }

    Ok(())
}

fn push_array_set_operation(
    context: &mut ChangeContext,
    parent_created_at: &TimeTicket,
    created_at: TimeTicket,
    value: &JsonValue,
) -> Result<()> {
    let executed_at = context.issue_time_ticket();
    let element = json_value_to_crdt_element(value, executed_at.clone(), context)?;
    context.push(Operation::ArraySet(ArraySetOperation::create(
        parent_created_at.clone(),
        created_at,
        element,
        Some(executed_at),
    )));
    Ok(())
}

fn lcs_pairs(before: &[JsonValue], after: &[JsonValue]) -> Vec<(usize, usize)> {
    let mut lengths = vec![vec![0; after.len() + 1]; before.len() + 1];
    for before_index in (0..before.len()).rev() {
        for after_index in (0..after.len()).rev() {
            lengths[before_index][after_index] = if before[before_index] == after[after_index] {
                lengths[before_index + 1][after_index + 1] + 1
            } else {
                lengths[before_index + 1][after_index].max(lengths[before_index][after_index + 1])
            };
        }
    }

    let mut pairs = Vec::new();
    let mut before_index = 0;
    let mut after_index = 0;
    while before_index < before.len() && after_index < after.len() {
        if before[before_index] == after[after_index] {
            pairs.push((before_index, after_index));
            before_index += 1;
            after_index += 1;
        } else if lengths[before_index + 1][after_index] >= lengths[before_index][after_index + 1] {
            before_index += 1;
        } else {
            after_index += 1;
        }
    }

    pairs
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
        JsonValue::Counter(value) => CrdtElement::counter(value.to_crdt_counter(created_at)),
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

fn recorded_json_value_from_crdt_element(element: &CrdtElement) -> Result<RecordedJsonValue> {
    let created_at = element.created_at().clone();
    Ok(RecordedJsonValue::new(
        crdt_element_to_json_value(element)?,
        created_at.clone(),
        created_at,
    ))
}

fn crdt_object_to_json_object(object: &CrdtObject) -> Result<JsonObject> {
    let mut json_object = JsonObject::with_created_at(object.created_at().clone());

    for (key, element) in object.iter() {
        json_object.set_tracked_unchecked(
            key,
            crdt_element_to_json_value(element)?,
            element.created_at().clone(),
        );
    }

    Ok(json_object)
}

fn crdt_element_to_json_value(element: &CrdtElement) -> Result<JsonValue> {
    let value = match element {
        CrdtElement::Primitive(value) => value.to_json_value(),
        CrdtElement::Counter(value) => JsonValue::Counter(crate::JsonCounter::from_crdt(value)),
        CrdtElement::Object(value) => JsonValue::Object(crdt_object_to_json_object(value)?),
        CrdtElement::Array(value) => {
            let mut array = crate::JsonArray::with_created_at(value.created_at().clone());
            for node in value.iter_all_nodes() {
                let Some(element) = node.element() else {
                    continue;
                };
                if node.is_removed() {
                    continue;
                }
                array.push_tracked_unchecked(
                    crdt_element_to_json_value(element)?,
                    element.created_at().clone(),
                    node.position_created_at().clone(),
                );
            }
            JsonValue::Array(array)
        }
        CrdtElement::Text(value) => JsonValue::Array(value.to_json_array()?),
        CrdtElement::Tree(value) => crdt_tree_node_to_json_value(value.root())?,
    };

    Ok(value)
}

fn crdt_tree_node_to_json_value(node: &TreeNode) -> Result<JsonValue> {
    let mut object = JsonObject::new();
    object.set_unchecked("type", node.node_type().to_owned());

    if node.is_text() {
        object.set_unchecked("value", node.value().to_owned());
        return Ok(JsonValue::Object(object));
    }

    let mut children = crate::JsonArray::new();
    for child in node.children() {
        children.push(crdt_tree_node_to_json_value(child)?)?;
    }
    object.set_unchecked("children", children);

    if let Some(attrs) = node.attrs() {
        let values = attrs.to_object();
        if !values.is_empty() {
            let mut attr_object = JsonObject::new();
            for (key, value) in values {
                attr_object.set_unchecked(key, attribute_value_to_json_value(&value));
            }
            object.set_unchecked("attributes", attr_object);
        }
    }

    Ok(JsonValue::Object(object))
}

#[cfg(test)]
mod tests {
    use super::{crdt_element_to_json_value, Document};
    use crate::change::ChangePack;
    use crate::crdt::counter::{CounterType, CounterValue, CrdtCounter};
    use crate::crdt::element::CrdtElement;
    use crate::crdt::rht::Rht;
    use crate::crdt::text::CrdtText;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::{Checkpoint, JsonArray, JsonObject, JsonValue, Result, VersionVector, YorkieError};

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
    fn converts_crdt_counter_to_public_json_counter() -> Result<()> {
        let counter = CrdtCounter::create(
            CounterType::Long,
            CounterValue::Long(10),
            crate::TimeTicket::new(1, 0, "a"),
        );

        let value = crdt_element_to_json_value(&CrdtElement::counter(counter))?;

        assert_eq!("10", value.to_sorted_json());
        match value {
            JsonValue::Counter(counter) => {
                assert_eq!(CounterType::Long, counter.value_type());
                assert_eq!(CounterValue::Long(10), counter.value());
            }
            _ => panic!("expected counter"),
        }
        Ok(())
    }

    #[test]
    fn converts_crdt_tree_to_public_json_object() -> Result<()> {
        let mut attrs = Rht::new();
        attrs.set("bold", "\"true\"", crate::TimeTicket::new(4, 0, "a"));
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(crate::TimeTicket::new(1, 0, "a"), 0),
                "root",
                None,
                vec![TreeNode::create_element(
                    TreeNodeId::new(crate::TimeTicket::new(2, 0, "a"), 0),
                    "p",
                    Some(attrs),
                    vec![TreeNode::create_text(
                        TreeNodeId::new(crate::TimeTicket::new(3, 0, "a"), 0),
                        "Hi",
                    )],
                )],
            ),
            crate::TimeTicket::new(1, 0, "a"),
        );

        let value = crdt_element_to_json_value(&CrdtElement::tree(tree))?;

        assert_eq!(
            r#"{"children":[{"attributes":{"bold":"true"},"children":[{"type":"text","value":"Hi"}],"type":"p"}],"type":"root"}"#,
            value.to_sorted_json()
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
            "1:00:1.SET.active=true,1:00:1.REMOVE.1:00:2",
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
            todos.push("write tests")?.push(profile)?;
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
    fn records_counter_creation_and_increase_in_the_same_update() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_counter("count", 1)?
                .increase(2i32)?
                .increase(3.5)?;
            Ok(())
        })?;

        assert_eq!(r#"{"count":6}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.count=1,1:00:1.INCREASE.2,1:00:1.INCREASE.3.5"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_counter_increases() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_counter("count", 10)?;
            Ok(())
        })?;
        doc.update(|root| {
            let count = root.get_counter_mut("count")?;
            assert_eq!(CounterValue::Integer(10), count.value());

            count.increase(5i32)?;
            Ok(())
        })?;

        assert_eq!(r#"{"count":15}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.INCREASE.5"#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_counter_increases_inside_arrays() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set("items", JsonArray::new())?;
            root.get_array_mut("items")?
                .push_counter(1)?
                .increase(4i32)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?
                .get_counter_mut(0)?
                .increase(5i32)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":[10]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.items=[],1:00:1.ADD.1,1:00:2.INCREASE.4"#,
            doc.local_changes[0].to_test_string()
        );
        assert_eq!(
            r#"1:00:2.INCREASE.5"#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_long_counter_overflow() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_long_counter("longCount", i64::MAX)?
                .increase(1i64)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"longCount":-9223372036854775808}"#,
            doc.to_sorted_json()
        );
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.longCount=9223372036854775807,1:00:1.INCREASE.1"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_dedup_counter_adds_with_actor() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set_dedup_counter("uv")?
                .add("user-1")?
                .add("user-1")?
                .add("user-2")?;
            Ok(())
        })?;

        assert_eq!(r#"{"uv":2}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.uv=0,1:00:1.INCREASE.1,1:00:1.INCREASE.1,1:00:1.INCREASE.1"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_pushes_as_add_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.push("two")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","two"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(2, doc.local_changes.len());
        assert_eq!(r#"1:00:1.ADD."two""#, doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_array_push_after_object_set_in_the_same_update() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set("items", JsonArray::new())?;
            root.get_array_mut("items")?.push("one")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one"]}"#, doc.to_sorted_json());
        assert_eq!(
            r#"0:00:0.SET.items=[],1:00:1.ADD."one""#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_removes_as_remove_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            assert_eq!(
                Some(JsonValue::String("two".to_owned())),
                root.get_array_mut("items")?.remove(1)
            );
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.REMOVE.1:00:3",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_delete_and_push_as_remove_and_add_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            assert_eq!(Some(JsonValue::String("two".to_owned())), items.remove(1));
            items.push("four")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","three","four"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.REMOVE.1:00:3,1:00:1.ADD."four""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_sets_as_array_set_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.set(1, "updated")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","updated"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.ARRAY_SET.1:00:3="updated""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_array_set_even_when_visible_value_is_unchanged() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.set(0, "one")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one"]}"#, doc.to_sorted_json());
        assert_eq!(
            r#"1:00:1.ARRAY_SET.1:00:2="one""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_existing_array_inserts_as_add_operations() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.insert(1, "two")?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","two","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(r#"1:00:1.ADD."two""#, doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_public_array_insert_after_and_before_by_id() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("four")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let four_id = items.element_id(1).unwrap().clone();

            assert_eq!(
                Some(&JsonValue::String("one".to_owned())),
                items.get_by_id(&one_id)
            );

            items.insert_after(&one_id, "two")?;
            items.insert_before(&four_id, "three")?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"items":["one","two","three","four"]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.ADD."two",1:00:1.ADD."three""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn exposes_public_array_elements_with_ids() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let first = items.get_element_by_index(0).unwrap();
            let first_id = first.id().clone();

            assert_eq!(&JsonValue::String("one".to_owned()), first.value());
            assert_eq!(Some(&first_id), items.element_id(0));
            assert!(items.contains_id(&first_id));
            assert_eq!(Some(0), items.index_of_id(&first_id, None));
            assert_eq!(Some(0), items.last_index_of_id(&first_id, None));
            assert_eq!(
                Some(&JsonValue::String("one".to_owned())),
                items.get_by_id(&first_id)
            );
            assert_eq!(
                &JsonValue::String("one".to_owned()),
                items.get_element_by_id(&first_id).unwrap().value()
            );
            assert_eq!(
                &JsonValue::String("two".to_owned()),
                items.get_last().unwrap().value()
            );
            Ok(())
        })?;

        assert_eq!(1, doc.local_changes.len());
        Ok(())
    }

    #[test]
    fn records_public_array_insert_after_index() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push(1i32)?.push(3i32)?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?.insert_integer_after(0, 2)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":[1,2,3]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!("1:00:1.ADD.2", doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_public_array_delete_by_id() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let two_id = items.element_id(1).unwrap().clone();

            assert_eq!(
                Some(JsonValue::String("two".to_owned())),
                items.delete_by_id(&two_id)
            );
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.REMOVE.1:00:3",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_splice_remove_and_insert() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("a")?.push("b")?.push("c")?.push("d")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let removed = root
                .get_array_mut("items")?
                .splice(1, Some(2), ["x", "y"])?;

            assert_eq!(r#"["b","c"]"#, removed.to_sorted_json());
            Ok(())
        })?;

        assert_eq!(r#"{"items":["a","x","y","d"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.REMOVE.1:00:3,1:00:1.REMOVE.1:00:4,1:00:1.ADD."x",1:00:1.ADD."y""#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_splice_with_negative_start_and_open_delete_count() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("a")?.push("b")?.push("c")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let removed = root
                .get_array_mut("items")?
                .splice(-2, None, Vec::<JsonValue>::new())?;

            assert_eq!(r#"["b","c"]"#, removed.to_sorted_json());
            Ok(())
        })?;

        assert_eq!(r#"{"items":["a"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            r#"1:00:1.REMOVE.1:00:3,1:00:1.REMOVE.1:00:4"#,
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_changes_after_array_splice_insert() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            root.set("items", JsonArray::new())?;
            let mut inserted = JsonObject::new();
            inserted.set("name", "yorkie")?;
            let removed = root
                .get_array_mut("items")?
                .splice(0, Some(0), [inserted])?;
            assert_eq!("[]", removed.to_sorted_json());
            root.get_array_mut("items")?
                .get_object_mut(0)?
                .set("active", true)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"items":[{"active":true,"name":"yorkie"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            r#"0:00:0.SET.items=[],1:00:1.ADD.{"name":"yorkie"},1:00:2.SET.active=true"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_move_after_by_id() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let three_id = items.element_id(2).unwrap().clone();

            items.move_after(&three_id, &one_id)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["two","three","one"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!("1:00:1.MOVE", doc.local_changes[1].to_test_string());
        Ok(())
    }

    #[test]
    fn records_public_array_move_front_and_last() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let three_id = items.last_id().unwrap().clone();

            items.move_last(&one_id)?;
            items.move_front(&three_id)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["three","two","one"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.MOVE,1:00:1.MOVE",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_public_array_move_before_and_after_by_index() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut items = JsonArray::new();
            items.push("one")?.push("two")?.push("three")?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            let items = root.get_array_mut("items")?;
            let one_id = items.element_id(0).unwrap().clone();
            let three_id = items.element_id(2).unwrap().clone();

            items.move_before(&one_id, &three_id)?;
            items.move_after_by_index(2, 0)?;
            Ok(())
        })?;

        assert_eq!(r#"{"items":["one","two","three"]}"#, doc.to_sorted_json());
        assert_eq!(doc.to_sorted_json(), doc.crdt_root.to_sorted_json());
        assert_eq!(
            "1:00:1.MOVE,1:00:1.MOVE",
            doc.local_changes[1].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_set_after_parent_set_in_the_same_update() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            root.set("profile", profile)?;
            root.get_object_mut("profile")?.set("active", true)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"profile":{"active":true,"name":"yorkie"}}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            r#"0:00:0.SET.profile={"name":"yorkie"},1:00:1.SET.active=true"#,
            doc.local_changes[0].to_test_string()
        );
        Ok(())
    }

    #[test]
    fn records_nested_object_changes_inside_arrays() -> Result<()> {
        let mut doc = Document::new("doc-key");

        doc.update(|root| {
            let mut profile = JsonObject::new();
            profile.set("name", "yorkie")?;
            let mut items = JsonArray::new();
            items.push(profile)?;
            root.set("items", items)?;
            Ok(())
        })?;
        doc.update(|root| {
            root.get_array_mut("items")?
                .get_object_mut(0)?
                .set("active", true)?;
            Ok(())
        })?;

        assert_eq!(
            r#"{"items":[{"active":true,"name":"yorkie"}]}"#,
            doc.to_sorted_json()
        );
        assert_eq!(
            r#"{"items":[{"name":"yorkie","active":true}]}"#,
            doc.crdt_root.to_json()
        );
        assert_eq!(
            "1:00:2.SET.active=true",
            doc.local_changes[1].to_test_string()
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
            array.push("sync")?.push(false)?;
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
    fn applies_remote_counter_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            root.set_counter("count", 1)?.increase(2i32)?;
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

        assert_eq!(r#"{"count":3}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn applies_remote_dedup_counter_changes_from_change_pack() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            root.set_dedup_counter("uv")?
                .add("user-1")?
                .add("user-1")?
                .add("user-2")?;
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

        assert_eq!(r#"{"uv":2}"#, target.to_sorted_json());
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        Ok(())
    }

    #[test]
    fn applies_snapshot_change_pack_to_root() -> Result<()> {
        let mut source = Document::new("source-doc");
        let mut target = Document::new("target-doc");

        source.update(|root| {
            root.set("title", "snapshot")?;
            let mut items = JsonArray::new();
            items.push("one")?.push(2i32)?;
            root.set("items", items)?;
            Ok(())
        })?;

        let source_pack = source.create_change_pack();
        let snapshot_pack = ChangePack::create_with_snapshot_root(
            "target-doc",
            Checkpoint::new(7, 0),
            false,
            Vec::new(),
            source_pack.version_vector().cloned(),
            Some(vec![1]),
            Some(source.crdt_root.object().clone()),
        );

        target.apply_change_pack(&snapshot_pack)?;

        assert_eq!(Checkpoint::new(7, 0), target.checkpoint());
        assert_eq!(
            r#"{"items":["one",2],"title":"snapshot"}"#,
            target.to_sorted_json()
        );
        assert_eq!(target.crdt_root.to_sorted_json(), target.to_sorted_json());
        assert!(target.change_id.lamport() > source_pack.version_vector().unwrap().max_lamport());
        Ok(())
    }

    #[test]
    fn reapplies_remaining_local_changes_after_snapshot() -> Result<()> {
        let mut source = Document::new("source-doc");
        source.update(|root| root.set("title", "remote").map(|_| ()))?;

        let mut target = Document::new("target-doc");
        target.update(|root| root.set("acked1", "old").map(|_| ()))?;
        target.update(|root| root.set("acked2", "old").map(|_| ()))?;
        target.update(|root| root.set("draft", "local").map(|_| ()))?;

        let source_pack = source.create_change_pack();
        let snapshot_pack = ChangePack::create_with_snapshot_root(
            "target-doc",
            Checkpoint::new(7, 2),
            false,
            Vec::new(),
            source_pack.version_vector().cloned(),
            Some(vec![1]),
            Some(source.crdt_root.object().clone()),
        );

        target.apply_change_pack(&snapshot_pack)?;

        assert_eq!(1, target.local_changes.len());
        assert_eq!(
            r#"{"draft":"local","title":"remote"}"#,
            target.to_sorted_json()
        );
        Ok(())
    }

    #[test]
    fn rejects_raw_snapshot_bytes_without_decoded_root() {
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
