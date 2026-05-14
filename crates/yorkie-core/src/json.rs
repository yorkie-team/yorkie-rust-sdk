use crate::{Result, YorkieError};
use std::collections::BTreeMap;
use std::fmt::Write;

/// A JSON-like value stored in a Yorkie document root.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Integer(i32),
    Long(i64),
    Double(f64),
    String(String),
    Object(JsonObject),
    Array(JsonArray),
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
            Self::Object(value) => value.to_sorted_json(),
            Self::Array(value) => value.to_sorted_json(),
        }
    }
}

/// A JSON object whose keys are emitted in sorted order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonObject {
    members: BTreeMap<String, JsonValue>,
}

impl JsonObject {
    pub fn new() -> Self {
        Self::default()
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

        self.members.insert(key, value.into());
        Ok(self)
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        self.members.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut JsonValue> {
        self.members.get_mut(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<JsonValue> {
        self.members.remove(key)
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
}

/// A JSON array stored in a Yorkie document root.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonArray {
    elements: Vec<JsonValue>,
}

impl JsonArray {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, value: impl Into<JsonValue>) -> &mut Self {
        self.elements.push(value.into());
        self
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
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

impl<T> From<Vec<T>> for JsonValue
where
    T: Into<JsonValue>,
{
    fn from(values: Vec<T>) -> Self {
        let mut array = JsonArray::new();
        for value in values {
            array.push(value);
        }
        Self::Array(array)
    }
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
    use super::{JsonArray, JsonObject};
    use crate::{Result, YorkieError};

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
        array.push("one").push(child);

        assert_eq!(r#"["one",{"name":"yorkie"}]"#, array.to_sorted_json());

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
