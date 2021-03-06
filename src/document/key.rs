use std::fmt;

const SPLITTER: &str = "$";
const TOKEN_LEN: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct InvalidCombinedKeyError;

impl fmt::Display for InvalidCombinedKeyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "invalid combined key")
    }
}

/// Key represents the key of the Document.
#[derive(Debug)]
pub struct Key {
    collection: String,
    document: String,
}

impl Key {
    /// from_combined_key creates an instance of Key from the received combined key.
    pub fn from_combined_key(combined_key: &str) -> Result<Key, InvalidCombinedKeyError> {
        let splits = combined_key.split(SPLITTER).collect::<Vec<_>>();
        if splits.len() != TOKEN_LEN {
            return Err(InvalidCombinedKeyError);
        }

        let collection = splits[0].to_string();
        let document = splits[1].to_string();
        Ok(Key {
            collection,
            document,
        })
    }

    /// combined_key returns the string of this key.
    pub fn combined_key(&self) -> String {
        format!("{}{}{}", self.collection, SPLITTER, self.document)
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    pub fn document(&self) -> &str {
        &self.document
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_combined_key() {
        // create success
        let key = Key::from_combined_key("collection$document").unwrap();
        assert_eq!(key.collection, "collection");
        assert_eq!(key.document, "document");

        // invalid combined key
        let err = Key::from_combined_key("collection").unwrap_err();
        assert_eq!(err, InvalidCombinedKeyError);
        let err = Key::from_combined_key("collection$document$bb").unwrap_err();
        assert_eq!(err, InvalidCombinedKeyError);
    }

    #[test]
    fn combined_key() {
        // create success
        let combined_key = "collection$document";
        let key = Key::from_combined_key(combined_key).unwrap();
        assert_eq!(key.combined_key(), combined_key);
    }
}
