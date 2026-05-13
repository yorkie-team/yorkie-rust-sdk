use crate::{JsonObject, Result, YorkieError};

/// A local Yorkie document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    key: String,
    root: JsonObject,
}

impl Document {
    /// Creates a document with the given Yorkie resource key.
    pub fn new(key: impl Into<String>) -> Result<Self> {
        let key = key.into();
        validate_key(&key)?;

        Ok(Self {
            key,
            root: JsonObject::new(),
        })
    }

    /// Returns this document's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Updates the document root.
    ///
    /// This is intentionally local-only for the first scaffold. Change capture,
    /// logical clocks, and synchronization will be added behind this API.
    pub fn update<F>(&mut self, update_fn: F) -> Result<()>
    where
        F: FnOnce(&mut JsonObject) -> Result<()>,
    {
        update_fn(&mut self.root)
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

fn validate_key(key: &str) -> Result<()> {
    let valid_len = (4..=120).contains(&key.chars().count());
    let valid_chars = key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~'));

    if valid_len && valid_chars {
        return Ok(());
    }

    Err(YorkieError::InvalidKey(key.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::Document;

    #[test]
    fn rejects_invalid_document_keys() {
        assert!(Document::new("abc").is_err());
        assert!(Document::new("invalid key").is_err());
        assert!(Document::new("invalid-key-$").is_err());
    }

    #[test]
    fn accepts_yorkie_resource_keys() {
        assert!(Document::new("valid-key").is_ok());
        assert!(Document::new("Capital-Character-Key").is_ok());
        assert!(Document::new("valid.key_~").is_ok());
    }
}
