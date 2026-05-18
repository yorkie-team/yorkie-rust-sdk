use crate::crdt::counter::{CounterType, CounterValue, CrdtCounter};
use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::{Result, TimeTicket, YorkieError};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::{self, Write};
use std::rc::Rc;

pub(crate) type JsonEditRecorderRef = Rc<RefCell<dyn JsonEditRecorder>>;

pub(crate) trait JsonEditRecorder {
    fn record_object_set(
        &mut self,
        parent_created_at: &TimeTicket,
        key: &str,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue>;

    fn record_object_remove(
        &mut self,
        parent_created_at: &TimeTicket,
        key: &str,
        created_at: &TimeTicket,
    );

    fn record_array_insert(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue>;

    fn record_array_set(
        &mut self,
        parent_created_at: &TimeTicket,
        created_at: &TimeTicket,
        value: &JsonValue,
    ) -> Result<RecordedJsonValue>;

    fn record_array_remove(&mut self, parent_created_at: &TimeTicket, created_at: &TimeTicket);

    fn record_array_move(
        &mut self,
        parent_created_at: &TimeTicket,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
    ) -> TimeTicket;

    fn record_counter_increase(
        &mut self,
        parent_created_at: &TimeTicket,
        value: CounterValue,
        actor: Option<&str>,
    ) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordedJsonValue {
    value: JsonValue,
    created_at: TimeTicket,
    position_created_at: TimeTicket,
}

impl RecordedJsonValue {
    pub(crate) fn new(
        value: JsonValue,
        created_at: TimeTicket,
        position_created_at: TimeTicket,
    ) -> Self {
        Self {
            value,
            created_at,
            position_created_at,
        }
    }
}

/// A JSON-like value stored in a Yorkie document root.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Integer(i32),
    Long(i64),
    Double(f64),
    String(String),
    Counter(JsonCounter),
    Object(JsonObject),
    Array(JsonArray),
}

/// A JSON array element together with its CRDT identity.
#[derive(Debug, Clone, Copy)]
pub struct JsonArrayElement<'a> {
    id: &'a TimeTicket,
    value: &'a JsonValue,
}

impl<'a> JsonArrayElement<'a> {
    pub fn new(id: &'a TimeTicket, value: &'a JsonValue) -> Self {
        Self { id, value }
    }

    /// Returns the element ID.
    pub fn id(&self) -> &'a TimeTicket {
        self.id
    }

    /// Returns the JSON value of this element.
    pub fn value(&self) -> &'a JsonValue {
        self.value
    }
}

impl JsonValue {
    /// Serializes this value as JSON.
    pub fn to_sorted_json(&self) -> String {
        match self {
            Self::Null => "null".to_owned(),
            Self::Bool(value) => value.to_string(),
            Self::Integer(value) => value.to_string(),
            Self::Long(value) => value.to_string(),
            Self::Double(value) if value.is_finite() => value.to_string(),
            Self::Double(_) => "null".to_owned(),
            Self::String(value) => format!("\"{}\"", escape_json_string(value)),
            Self::Counter(value) => value.to_sorted_json(),
            Self::Object(value) => value.to_sorted_json(),
            Self::Array(value) => value.to_sorted_json(),
        }
    }
}

/// A JSON counter stored in a Yorkie document root.
#[derive(Clone)]
pub struct JsonCounter {
    value_type: CounterType,
    value: CounterValue,
    hll_bytes: Option<Vec<u8>>,
    created_at: TimeTicket,
    recorder: Option<JsonEditRecorderRef>,
}

impl fmt::Debug for JsonCounter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonCounter")
            .field("value_type", &self.value_type)
            .field("value", &self.value)
            .finish()
    }
}

impl PartialEq for JsonCounter {
    fn eq(&self, other: &Self) -> bool {
        self.value_type == other.value_type
            && self.value == other.value
            && self.hll_bytes == other.hll_bytes
    }
}

impl JsonCounter {
    pub fn new(value: i64) -> Self {
        if let Ok(value) = i32::try_from(value) {
            return Self::integer(value);
        }

        Self::long(value)
    }

    pub fn integer(value: i32) -> Self {
        Self::new_with_type(CounterType::Integer, CounterValue::Integer(value))
    }

    pub fn long(value: i64) -> Self {
        Self::new_with_type(CounterType::Long, CounterValue::Long(value))
    }

    pub fn dedup() -> Self {
        Self::new_with_type(CounterType::IntegerDedup, CounterValue::Integer(0))
    }

    pub fn id(&self) -> &TimeTicket {
        &self.created_at
    }

    pub fn value_type(&self) -> CounterType {
        self.value_type
    }

    pub fn value(&self) -> CounterValue {
        self.value
    }

    pub fn increase(&mut self, value: impl Into<CounterValue>) -> Result<&mut Self> {
        let value = value.into();
        self.apply_increase(value, None)?;
        if let Some(recorder) = self.recorder.clone() {
            recorder
                .borrow_mut()
                .record_counter_increase(&self.created_at, value, None)?;
        }

        Ok(self)
    }

    pub fn add(&mut self, actor: impl AsRef<str>) -> Result<&mut Self> {
        if self.value_type != CounterType::IntegerDedup {
            return Err(YorkieError::InvalidCounterOperation(
                "add is only supported on dedup counters".to_owned(),
            ));
        }

        let actor = actor.as_ref();
        if actor.is_empty() {
            return Err(YorkieError::InvalidCounterOperation(
                "actor is required".to_owned(),
            ));
        }

        let value = CounterValue::Integer(1);
        self.apply_increase(value, Some(actor))?;
        if let Some(recorder) = self.recorder.clone() {
            recorder
                .borrow_mut()
                .record_counter_increase(&self.created_at, value, Some(actor))?;
        }

        Ok(self)
    }

    pub fn to_sorted_json(&self) -> String {
        self.to_crdt_counter(self.created_at.clone()).to_json()
    }

    pub(crate) fn from_crdt(counter: &CrdtCounter) -> Self {
        Self {
            value_type: counter.counter_type(),
            value: counter.value(),
            hll_bytes: counter.hll_bytes(),
            created_at: counter.created_at().clone(),
            recorder: None,
        }
    }

    pub(crate) fn to_crdt_counter(&self, created_at: TimeTicket) -> CrdtCounter {
        let mut counter = CrdtCounter::create(self.value_type, self.value, created_at);
        if let Some(bytes) = &self.hll_bytes {
            counter
                .restore_hll(bytes)
                .expect("stored counter HLL registers should be valid");
        }
        counter
    }

    fn new_with_type(value_type: CounterType, value: CounterValue) -> Self {
        let counter = CrdtCounter::create(value_type, value, TimeTicket::initial());
        Self {
            value_type,
            value: counter.value(),
            hll_bytes: counter.hll_bytes(),
            created_at: TimeTicket::initial(),
            recorder: None,
        }
    }

    fn apply_increase(&mut self, value: CounterValue, actor: Option<&str>) -> Result<()> {
        let mut counter = self.to_crdt_counter(self.created_at.clone());
        let operand = counter_operand(value, TimeTicket::initial());
        if let Some(actor) = actor {
            counter.increase_dedup(&operand, actor)?;
        } else {
            counter.increase(&operand)?;
        }

        self.value = counter.value();
        self.hll_bytes = counter.hll_bytes();
        Ok(())
    }

    fn attach_recorder(&mut self, recorder: JsonEditRecorderRef) {
        self.recorder = Some(recorder);
    }

    fn detach_recorder(&mut self) {
        self.recorder = None;
    }
}

/// A JSON object whose keys are emitted in sorted order.
#[derive(Clone)]
pub struct JsonObject {
    members: BTreeMap<String, JsonValue>,
    member_created_at: BTreeMap<String, TimeTicket>,
    created_at: TimeTicket,
    recorder: Option<JsonEditRecorderRef>,
}

impl fmt::Debug for JsonObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonObject")
            .field("members", &self.members)
            .finish()
    }
}

impl Default for JsonObject {
    fn default() -> Self {
        Self {
            members: BTreeMap::new(),
            member_created_at: BTreeMap::new(),
            created_at: TimeTicket::initial(),
            recorder: None,
        }
    }
}

impl PartialEq for JsonObject {
    fn eq(&self, other: &Self) -> bool {
        self.members == other.members
    }
}

impl JsonObject {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_created_at(created_at: TimeTicket) -> Self {
        Self {
            created_at,
            ..Self::default()
        }
    }

    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<JsonValue>,
    ) -> Result<&mut Self> {
        let key = key.into();
        if key.contains('.') {
            return Err(YorkieError::InvalidObjectKey(key));
        }

        let value = value.into();
        let value = if let Some(recorder) = self.recorder.clone() {
            let mut recorded =
                recorder
                    .borrow_mut()
                    .record_object_set(&self.created_at, &key, &value)?;
            recorded.value.attach_recorder(recorder);
            self.member_created_at
                .insert(key.clone(), recorded.created_at);
            recorded.value
        } else {
            self.member_created_at.remove(&key);
            value
        };

        self.members.insert(key, value);
        Ok(self)
    }

    pub(crate) fn set_unchecked(
        &mut self,
        key: impl Into<String>,
        value: impl Into<JsonValue>,
    ) -> &mut Self {
        self.members.insert(key.into(), value.into());
        self
    }

    pub(crate) fn set_tracked_unchecked(
        &mut self,
        key: impl Into<String>,
        value: impl Into<JsonValue>,
        created_at: TimeTicket,
    ) -> &mut Self {
        let key = key.into();
        self.members.insert(key.clone(), value.into());
        self.member_created_at.insert(key, created_at);
        self
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        self.members.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut JsonValue> {
        self.members.get_mut(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<JsonValue> {
        if let (Some(recorder), Some(created_at)) = (
            self.recorder.clone(),
            self.member_created_at.get(key).cloned(),
        ) {
            recorder
                .borrow_mut()
                .record_object_remove(&self.created_at, key, &created_at);
        }

        self.member_created_at.remove(key);
        let mut removed = self.members.remove(key)?;
        removed.detach_recorder();
        Some(removed)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &JsonValue)> {
        self.members
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    pub fn get_array_mut(&mut self, key: &str) -> Result<&mut JsonArray> {
        match self.members.get_mut(key) {
            Some(JsonValue::Array(value)) => Ok(value),
            Some(_) => Err(YorkieError::UnexpectedType {
                key: key.to_owned(),
                expected: "array",
            }),
            None => Err(YorkieError::MissingKey(key.to_owned())),
        }
    }

    pub fn get_counter_mut(&mut self, key: &str) -> Result<&mut JsonCounter> {
        match self.members.get_mut(key) {
            Some(JsonValue::Counter(value)) => Ok(value),
            Some(_) => Err(YorkieError::UnexpectedType {
                key: key.to_owned(),
                expected: "counter",
            }),
            None => Err(YorkieError::MissingKey(key.to_owned())),
        }
    }

    pub fn get_object_mut(&mut self, key: &str) -> Result<&mut JsonObject> {
        match self.members.get_mut(key) {
            Some(JsonValue::Object(value)) => Ok(value),
            Some(_) => Err(YorkieError::UnexpectedType {
                key: key.to_owned(),
                expected: "object",
            }),
            None => Err(YorkieError::MissingKey(key.to_owned())),
        }
    }

    pub fn set_counter(&mut self, key: impl Into<String>, value: i64) -> Result<&mut JsonCounter> {
        let key = key.into();
        self.set(key.clone(), JsonCounter::new(value))?;
        self.get_counter_mut(&key)
    }

    pub fn set_long_counter(
        &mut self,
        key: impl Into<String>,
        value: i64,
    ) -> Result<&mut JsonCounter> {
        let key = key.into();
        self.set(key.clone(), JsonCounter::long(value))?;
        self.get_counter_mut(&key)
    }

    pub fn set_dedup_counter(&mut self, key: impl Into<String>) -> Result<&mut JsonCounter> {
        let key = key.into();
        self.set(key.clone(), JsonCounter::dedup())?;
        self.get_counter_mut(&key)
    }

    pub fn to_sorted_json(&self) -> String {
        let members = self
            .members
            .iter()
            .map(|(key, value)| {
                format!("\"{}\":{}", escape_json_string(key), value.to_sorted_json())
            })
            .collect::<Vec<_>>()
            .join(",");

        format!("{{{members}}}")
    }

    pub(crate) fn attach_recorder(&mut self, recorder: JsonEditRecorderRef) {
        self.recorder = Some(recorder.clone());
        for value in self.members.values_mut() {
            value.attach_recorder(recorder.clone());
        }
    }
}

/// A JSON array stored in a Yorkie document root.
#[derive(Clone)]
pub struct JsonArray {
    elements: Vec<JsonValue>,
    element_created_at: Vec<TimeTicket>,
    position_created_at: Vec<TimeTicket>,
    created_at: TimeTicket,
    recorder: Option<JsonEditRecorderRef>,
}

impl fmt::Debug for JsonArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonArray")
            .field("elements", &self.elements)
            .finish()
    }
}

impl Default for JsonArray {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            element_created_at: Vec::new(),
            position_created_at: Vec::new(),
            created_at: TimeTicket::initial(),
            recorder: None,
        }
    }
}

impl PartialEq for JsonArray {
    fn eq(&self, other: &Self) -> bool {
        self.elements == other.elements
    }
}

impl JsonArray {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_created_at(created_at: TimeTicket) -> Self {
        Self {
            created_at,
            ..Self::default()
        }
    }

    pub fn push(&mut self, value: impl Into<JsonValue>) -> Result<&mut Self> {
        self.insert(self.elements.len(), value)
    }

    pub fn push_counter(&mut self, value: i64) -> Result<&mut JsonCounter> {
        self.push(JsonCounter::new(value))?;
        self.get_counter_mut(self.elements.len() - 1)
    }

    pub fn push_long_counter(&mut self, value: i64) -> Result<&mut JsonCounter> {
        self.push(JsonCounter::long(value))?;
        self.get_counter_mut(self.elements.len() - 1)
    }

    pub fn push_dedup_counter(&mut self) -> Result<&mut JsonCounter> {
        self.push(JsonCounter::dedup())?;
        self.get_counter_mut(self.elements.len() - 1)
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn id(&self) -> &TimeTicket {
        &self.created_at
    }

    pub fn element_id(&self, index: usize) -> Option<&TimeTicket> {
        self.element_created_at.get(index)
    }

    pub fn last_id(&self) -> Option<&TimeTicket> {
        self.element_created_at.last()
    }

    pub fn get(&self, index: usize) -> Option<&JsonValue> {
        self.elements.get(index)
    }

    pub fn get_element_by_index(&self, index: usize) -> Option<JsonArrayElement<'_>> {
        Some(JsonArrayElement::new(
            self.element_created_at.get(index)?,
            self.elements.get(index)?,
        ))
    }

    pub fn get_by_id(&self, id: &TimeTicket) -> Option<&JsonValue> {
        self.index_by_element_id(id)
            .and_then(|index| self.elements.get(index))
    }

    pub fn get_element_by_id(&self, id: &TimeTicket) -> Option<JsonArrayElement<'_>> {
        let index = self.index_by_element_id(id)?;
        self.get_element_by_index(index)
    }

    pub fn get_last(&self) -> Option<JsonArrayElement<'_>> {
        let index = self.elements.len().checked_sub(1)?;
        self.get_element_by_index(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut JsonValue> {
        self.elements.get_mut(index)
    }

    pub fn get_mut_by_id(&mut self, id: &TimeTicket) -> Option<&mut JsonValue> {
        let index = self.index_by_element_id(id)?;
        self.elements.get_mut(index)
    }

    pub fn contains(&self, value: impl Into<JsonValue>) -> bool {
        let value = value.into();
        self.elements.contains(&value)
    }

    pub fn index_of(
        &self,
        value: impl Into<JsonValue>,
        from_index: Option<isize>,
    ) -> Option<usize> {
        let value = value.into();
        let start = forward_search_start(from_index, self.elements.len())?;
        self.elements[start..]
            .iter()
            .position(|element| element == &value)
            .map(|index| start + index)
    }

    pub fn last_index_of(
        &self,
        value: impl Into<JsonValue>,
        from_index: Option<isize>,
    ) -> Option<usize> {
        let value = value.into();
        let start = reverse_search_start(from_index, self.elements.len())?;
        self.elements[..=start]
            .iter()
            .rposition(|element| element == &value)
    }

    pub fn contains_id(&self, id: &TimeTicket) -> bool {
        self.index_by_element_id(id).is_some()
    }

    pub fn index_of_id(&self, id: &TimeTicket, from_index: Option<isize>) -> Option<usize> {
        let start = forward_search_start(from_index, self.element_created_at.len())?;
        self.element_created_at[start..]
            .iter()
            .position(|element_id| element_id == id)
            .map(|index| start + index)
    }

    pub fn last_index_of_id(&self, id: &TimeTicket, from_index: Option<isize>) -> Option<usize> {
        let start = reverse_search_start(from_index, self.element_created_at.len())?;
        self.element_created_at[..=start]
            .iter()
            .rposition(|element_id| element_id == id)
    }

    pub fn set(&mut self, index: usize, value: impl Into<JsonValue>) -> Result<&mut Self> {
        let len = self.elements.len();
        if index >= len {
            return Err(YorkieError::InvalidIndex(format!(
                "array index {index} out of bounds for length {len}"
            )));
        };

        let value = value.into();
        let value = if let (Some(recorder), Some(created_at)) = (
            self.recorder.clone(),
            self.element_created_at.get(index).cloned(),
        ) {
            let mut recorded =
                recorder
                    .borrow_mut()
                    .record_array_set(&self.created_at, &created_at, &value)?;
            recorded.value.attach_recorder(recorder);
            self.element_created_at[index] = recorded.created_at;
            self.position_created_at[index] = recorded.position_created_at;
            recorded.value
        } else {
            value
        };

        self.elements[index] = value;
        Ok(self)
    }

    pub fn set_value(&mut self, index: usize, value: impl Into<JsonValue>) -> Result<&mut Self> {
        self.set(index, value)
    }

    pub fn insert(&mut self, index: usize, value: impl Into<JsonValue>) -> Result<&mut Self> {
        if index > self.elements.len() {
            return Err(YorkieError::InvalidIndex(format!(
                "array index {index} out of bounds for length {}",
                self.elements.len()
            )));
        }

        let value = value.into();
        let value = if let Some(recorder) = self.recorder.clone() {
            let prev_created_at = if index == 0 {
                TimeTicket::initial()
            } else {
                self.position_created_at
                    .get(index - 1)
                    .cloned()
                    .unwrap_or_else(TimeTicket::initial)
            };
            let mut recorded = recorder.borrow_mut().record_array_insert(
                &self.created_at,
                &prev_created_at,
                &value,
            )?;
            recorded.value.attach_recorder(recorder);
            self.element_created_at
                .insert(index, recorded.created_at.clone());
            self.position_created_at
                .insert(index, recorded.position_created_at);
            recorded.value
        } else {
            value
        };

        self.elements.insert(index, value);
        Ok(self)
    }

    pub fn insert_after(
        &mut self,
        prev_id: &TimeTicket,
        value: impl Into<JsonValue>,
    ) -> Result<&mut Self> {
        let index = self.insert_index_after_anchor(prev_id)?;
        self.insert(index, value)
    }

    pub fn insert_after_index(
        &mut self,
        index: usize,
        value: impl Into<JsonValue>,
    ) -> Result<&mut Self> {
        let len = self.elements.len();
        let prev_id = self.element_created_at.get(index).cloned().ok_or_else(|| {
            YorkieError::InvalidIndex(format!(
                "array index {index} out of bounds for length {len}"
            ))
        })?;

        self.insert_after(&prev_id, value)
    }

    pub fn insert_integer_after(&mut self, index: usize, value: i32) -> Result<&mut Self> {
        self.insert_after_index(index, value)
    }

    pub fn insert_before(
        &mut self,
        next_id: &TimeTicket,
        value: impl Into<JsonValue>,
    ) -> Result<&mut Self> {
        let index = self
            .index_by_element_id(next_id)
            .ok_or_else(|| YorkieError::MissingCrdtElement(next_id.to_id_string()))?;
        self.insert(index, value)
    }

    pub fn remove(&mut self, index: usize) -> Option<JsonValue> {
        if index >= self.elements.len() {
            return None;
        }

        if let (Some(recorder), Some(created_at)) = (
            self.recorder.clone(),
            self.element_created_at.get(index).cloned(),
        ) {
            recorder
                .borrow_mut()
                .record_array_remove(&self.created_at, &created_at);
        }

        if index < self.element_created_at.len() {
            self.element_created_at.remove(index);
        }
        if index < self.position_created_at.len() {
            self.position_created_at.remove(index);
        }
        let mut removed = self.elements.remove(index);
        removed.detach_recorder();
        Some(removed)
    }

    pub fn delete(&mut self, index: usize) -> Option<JsonValue> {
        self.remove(index)
    }

    pub fn delete_by_id(&mut self, id: &TimeTicket) -> Option<JsonValue> {
        let index = self.index_by_element_id(id)?;
        self.remove(index)
    }

    pub fn move_after(&mut self, prev_id: &TimeTicket, id: &TimeTicket) -> Result<&mut Self> {
        let insert_index = self.insert_index_after_anchor(prev_id)?;
        let prev_created_at = self.position_created_at_for_anchor(prev_id)?;
        self.move_to_index(prev_created_at, insert_index, id)
    }

    pub fn move_before(&mut self, next_id: &TimeTicket, id: &TimeTicket) -> Result<&mut Self> {
        let next_index = self
            .index_by_element_id(next_id)
            .ok_or_else(|| YorkieError::MissingCrdtElement(next_id.to_id_string()))?;
        let prev_created_at = if next_index == 0 {
            TimeTicket::initial()
        } else {
            self.position_created_at[next_index - 1].clone()
        };
        self.move_to_index(prev_created_at, next_index, id)
    }

    pub fn move_after_by_index(
        &mut self,
        prev_index: usize,
        target_index: usize,
    ) -> Result<&mut Self> {
        let len = self.elements.len();
        let prev_created_at = self
            .position_created_at
            .get(prev_index)
            .cloned()
            .ok_or_else(|| {
                YorkieError::InvalidIndex(format!(
                    "array index {prev_index} out of bounds for length {len}"
                ))
            })?;
        let target_id = self
            .element_created_at
            .get(target_index)
            .cloned()
            .ok_or_else(|| {
                YorkieError::InvalidIndex(format!(
                    "array index {target_index} out of bounds for length {len}"
                ))
            })?;

        self.move_to_index(prev_created_at, prev_index + 1, &target_id)
    }

    pub fn move_front(&mut self, id: &TimeTicket) -> Result<&mut Self> {
        self.move_to_index(TimeTicket::initial(), 0, id)
    }

    pub fn move_last(&mut self, id: &TimeTicket) -> Result<&mut Self> {
        let prev_created_at = self
            .position_created_at
            .last()
            .cloned()
            .ok_or_else(|| YorkieError::MissingCrdtElement(id.to_id_string()))?;
        self.move_to_index(prev_created_at, self.elements.len(), id)
    }

    pub fn splice<I, V>(
        &mut self,
        start: isize,
        delete_count: Option<usize>,
        items: I,
    ) -> Result<JsonArray>
    where
        I: IntoIterator<Item = V>,
        V: Into<JsonValue>,
    {
        let from = splice_start_index(start, self.elements.len());
        let to = delete_count
            .map(|count| from.saturating_add(count).min(self.elements.len()))
            .unwrap_or_else(|| self.elements.len());

        let mut removed_values = JsonArray::new();
        for _ in from..to {
            if let Some(removed) = self.remove(from) {
                removed_values.push(removed)?;
            }
        }

        let mut insert_at = from;
        for item in items {
            self.insert(insert_at, item)?;
            insert_at += 1;
        }

        Ok(removed_values)
    }

    pub fn get_array_mut(&mut self, index: usize) -> Result<&mut JsonArray> {
        let len = self.elements.len();
        match self.elements.get_mut(index) {
            Some(JsonValue::Array(value)) => Ok(value),
            Some(_) => Err(YorkieError::UnexpectedType {
                key: index.to_string(),
                expected: "array",
            }),
            None => Err(YorkieError::InvalidIndex(format!(
                "array index {index} out of bounds for length {len}"
            ))),
        }
    }

    pub fn get_counter_mut(&mut self, index: usize) -> Result<&mut JsonCounter> {
        let len = self.elements.len();
        match self.elements.get_mut(index) {
            Some(JsonValue::Counter(value)) => Ok(value),
            Some(_) => Err(YorkieError::UnexpectedType {
                key: index.to_string(),
                expected: "counter",
            }),
            None => Err(YorkieError::InvalidIndex(format!(
                "array index {index} out of bounds for length {len}"
            ))),
        }
    }

    pub fn get_object_mut(&mut self, index: usize) -> Result<&mut JsonObject> {
        let len = self.elements.len();
        match self.elements.get_mut(index) {
            Some(JsonValue::Object(value)) => Ok(value),
            Some(_) => Err(YorkieError::UnexpectedType {
                key: index.to_string(),
                expected: "object",
            }),
            None => Err(YorkieError::InvalidIndex(format!(
                "array index {index} out of bounds for length {len}"
            ))),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &JsonValue> {
        self.elements.iter()
    }

    pub fn as_slice(&self) -> &[JsonValue] {
        &self.elements
    }

    pub(crate) fn push_tracked_unchecked(
        &mut self,
        value: impl Into<JsonValue>,
        created_at: TimeTicket,
        position_created_at: TimeTicket,
    ) -> &mut Self {
        self.elements.push(value.into());
        self.element_created_at.push(created_at);
        self.position_created_at.push(position_created_at);
        self
    }

    pub fn to_sorted_json(&self) -> String {
        let elements = self
            .elements
            .iter()
            .map(JsonValue::to_sorted_json)
            .collect::<Vec<_>>()
            .join(",");

        format!("[{elements}]")
    }

    pub(crate) fn attach_recorder(&mut self, recorder: JsonEditRecorderRef) {
        self.recorder = Some(recorder.clone());
        for value in &mut self.elements {
            value.attach_recorder(recorder.clone());
        }
    }

    fn index_by_element_id(&self, id: &TimeTicket) -> Option<usize> {
        self.element_created_at
            .iter()
            .position(|created_at| created_at == id)
    }

    fn index_by_position_id(&self, id: &TimeTicket) -> Option<usize> {
        self.position_created_at
            .iter()
            .position(|created_at| created_at == id)
    }

    fn position_created_at_for_anchor(&self, id: &TimeTicket) -> Result<TimeTicket> {
        if id == &TimeTicket::initial() {
            return Ok(TimeTicket::initial());
        }

        if let Some(index) = self.index_by_element_id(id) {
            return Ok(self.position_created_at[index].clone());
        }

        if self.index_by_position_id(id).is_some() {
            return Ok(id.clone());
        }

        Err(YorkieError::MissingCrdtElement(id.to_id_string()))
    }

    fn insert_index_after_anchor(&self, id: &TimeTicket) -> Result<usize> {
        if id == &TimeTicket::initial() {
            return Ok(0);
        }

        if let Some(index) = self.index_by_element_id(id) {
            return Ok(index + 1);
        }

        if let Some(index) = self.index_by_position_id(id) {
            return Ok(index + 1);
        }

        Err(YorkieError::MissingCrdtElement(id.to_id_string()))
    }

    fn move_to_index(
        &mut self,
        prev_created_at: TimeTicket,
        insert_index: usize,
        id: &TimeTicket,
    ) -> Result<&mut Self> {
        let target_index = self
            .index_by_element_id(id)
            .ok_or_else(|| YorkieError::MissingCrdtElement(id.to_id_string()))?;
        let position_created_at = if let Some(recorder) = self.recorder.clone() {
            recorder
                .borrow_mut()
                .record_array_move(&self.created_at, &prev_created_at, id)
        } else {
            self.position_created_at[target_index].clone()
        };

        let value = self.elements.remove(target_index);
        let element_created_at = self.element_created_at.remove(target_index);
        self.position_created_at.remove(target_index);

        let mut adjusted_index = insert_index;
        if target_index < adjusted_index {
            adjusted_index -= 1;
        }

        self.elements.insert(adjusted_index, value);
        self.element_created_at
            .insert(adjusted_index, element_created_at);
        self.position_created_at
            .insert(adjusted_index, position_created_at);
        Ok(self)
    }
}

impl JsonValue {
    pub(crate) fn attach_recorder(&mut self, recorder: JsonEditRecorderRef) {
        match self {
            Self::Counter(value) => value.attach_recorder(recorder),
            Self::Object(value) => value.attach_recorder(recorder),
            Self::Array(value) => value.attach_recorder(recorder),
            Self::Null
            | Self::Bool(_)
            | Self::Integer(_)
            | Self::Long(_)
            | Self::Double(_)
            | Self::String(_) => {}
        }
    }

    fn detach_recorder(&mut self) {
        match self {
            Self::Counter(value) => value.detach_recorder(),
            Self::Object(value) => value.detach_recorder(),
            Self::Array(value) => value.detach_recorder(),
            Self::Null
            | Self::Bool(_)
            | Self::Integer(_)
            | Self::Long(_)
            | Self::Double(_)
            | Self::String(_) => {}
        }
    }
}

impl JsonObject {
    fn detach_recorder(&mut self) {
        self.recorder = None;
        for value in self.members.values_mut() {
            value.detach_recorder();
        }
    }
}

impl JsonArray {
    fn detach_recorder(&mut self) {
        self.recorder = None;
        for value in &mut self.elements {
            value.detach_recorder();
        }
    }
}

fn splice_start_index(start: isize, len: usize) -> usize {
    if start >= 0 {
        return (start as usize).min(len);
    }

    let offset = start
        .checked_abs()
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    len.saturating_sub(offset)
}

fn forward_search_start(from_index: Option<isize>, len: usize) -> Option<usize> {
    let start = match from_index {
        Some(index) if index >= 0 => index as usize,
        Some(index) => {
            let offset = index
                .checked_abs()
                .map(|value| value as usize)
                .unwrap_or(usize::MAX);
            len.saturating_sub(offset)
        }
        None => 0,
    };

    (start < len).then_some(start)
}

fn reverse_search_start(from_index: Option<isize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    match from_index {
        Some(index) if index >= 0 => Some((index as usize).min(len - 1)),
        Some(index) => {
            let offset = index
                .checked_abs()
                .map(|value| value as usize)
                .unwrap_or(usize::MAX);
            if offset <= len {
                Some(len - offset)
            } else {
                None
            }
        }
        None => Some(len - 1),
    }
}

impl From<()> for JsonValue {
    fn from(_: ()) -> Self {
        Self::Null
    }
}

impl From<bool> for JsonValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i32> for JsonValue {
    fn from(value: i32) -> Self {
        Self::Integer(value)
    }
}

impl From<i64> for JsonValue {
    fn from(value: i64) -> Self {
        Self::Long(value)
    }
}

impl From<f64> for JsonValue {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl From<&str> for JsonValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for JsonValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<JsonObject> for JsonValue {
    fn from(value: JsonObject) -> Self {
        Self::Object(value)
    }
}

impl From<JsonArray> for JsonValue {
    fn from(value: JsonArray) -> Self {
        Self::Array(value)
    }
}

impl From<JsonCounter> for JsonValue {
    fn from(value: JsonCounter) -> Self {
        Self::Counter(value)
    }
}

impl<T> From<Vec<T>> for JsonValue
where
    T: Into<JsonValue>,
{
    fn from(values: Vec<T>) -> Self {
        let mut array = JsonArray::new();
        for value in values {
            array
                .push(value)
                .expect("untracked array push should not fail");
        }
        Self::Array(array)
    }
}

pub(crate) fn counter_operand(value: CounterValue, created_at: TimeTicket) -> CrdtPrimitive {
    let value = match value {
        CounterValue::Integer(value) => PrimitiveValue::Integer(value),
        CounterValue::Long(value) => PrimitiveValue::Long(value),
        CounterValue::Double(value) => PrimitiveValue::Double(value),
    };
    CrdtPrimitive::new(value, created_at)
}

pub(crate) fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            ch if ch <= '\u{1f}' => {
                let _ = write!(escaped, "\\u{:04x}", ch as u32);
            }
            ch => escaped.push(ch),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::{JsonArray, JsonCounter, JsonObject, JsonValue};
    use crate::{CounterValue, Result, YorkieError};

    #[test]
    fn serializes_objects_with_sorted_keys() -> Result<()> {
        let mut object = JsonObject::new();
        object.set("z", 1i32)?;
        object.set("a", "first")?;

        assert_eq!(r#"{"a":"first","z":1}"#, object.to_sorted_json());

        Ok(())
    }

    #[test]
    fn serializes_nested_arrays_and_objects() -> Result<()> {
        let mut child = JsonObject::new();
        child.set("name", "yorkie")?;

        let mut array = JsonArray::new();
        array.push("one")?.push(child)?;

        assert_eq!(r#"["one",{"name":"yorkie"}]"#, array.to_sorted_json());

        Ok(())
    }

    #[test]
    fn updates_array_values_by_index() -> Result<()> {
        let mut array = JsonArray::new();
        array.push("one")?.push("three")?;
        array.insert(1, "two")?;
        array.set(2, "four")?;

        assert_eq!(Some(&JsonValue::String("two".to_owned())), array.get(1));
        assert_eq!(Some(JsonValue::String("one".to_owned())), array.remove(0));
        assert_eq!(r#"["two","four"]"#, array.to_sorted_json());

        Ok(())
    }

    #[test]
    fn splices_arrays_with_javascript_index_rules() -> Result<()> {
        let mut array = JsonArray::new();
        array.push("one")?.push("two")?.push("three")?;

        let removed = array.splice(-2, Some(1), ["inserted"])?;

        assert_eq!(r#"["two"]"#, removed.to_sorted_json());
        assert_eq!(r#"["one","inserted","three"]"#, array.to_sorted_json());

        let removed = array.splice(1, None, Vec::<JsonValue>::new())?;

        assert_eq!(r#"["inserted","three"]"#, removed.to_sorted_json());
        assert_eq!(r#"["one"]"#, array.to_sorted_json());

        Ok(())
    }

    #[test]
    fn searches_array_values_with_javascript_index_rules() -> Result<()> {
        let mut array = JsonArray::new();
        array.push("a")?.push("b")?.push("a")?;

        assert!(array.contains("a"));
        assert_eq!(Some(0), array.index_of("a", None));
        assert_eq!(Some(2), array.index_of("a", Some(-1)));
        assert_eq!(None, array.index_of("a", Some(3)));
        assert_eq!(Some(2), array.last_index_of("a", None));
        assert_eq!(Some(0), array.last_index_of("a", Some(1)));
        assert_eq!(None, array.last_index_of("a", Some(-4)));

        Ok(())
    }

    #[test]
    fn updates_counter_values_locally() -> Result<()> {
        let mut integer = JsonCounter::new(1);
        integer.increase(2i32)?.increase(3.5)?;

        assert_eq!(CounterValue::Integer(6), integer.value());
        assert_eq!("6", integer.to_sorted_json());

        assert_eq!(
            YorkieError::InvalidCounterOperation(
                "add is only supported on dedup counters".to_owned()
            ),
            integer.add("user-1").unwrap_err()
        );

        let mut dedup = JsonCounter::dedup();
        dedup.add("user-1")?.add("user-1")?.add("user-2")?;

        assert_eq!(CounterValue::Integer(2), dedup.value());
        assert_eq!("2", dedup.to_sorted_json());

        Ok(())
    }

    #[test]
    fn rejects_object_keys_with_dot() {
        let mut object = JsonObject::new();

        let err = object.set("nested.key", 1i32).unwrap_err();

        assert_eq!(YorkieError::InvalidObjectKey("nested.key".to_owned()), err);
        assert_eq!("{}", object.to_sorted_json());
    }
}
