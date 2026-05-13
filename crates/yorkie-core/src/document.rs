use crate::{JsonObject, Result};

/// A local Yorkie document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    key: String,
    root: JsonObject,
}

impl Document {
    /// Creates a document with the given Yorkie resource key.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            root: JsonObject::new(),
        }
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

#[cfg(test)]
mod tests {
    use super::Document;

    #[test]
    fn creates_document_with_the_given_key() {
        let doc = Document::new("doc-key");
        assert_eq!("doc-key", doc.key());
    }
}
