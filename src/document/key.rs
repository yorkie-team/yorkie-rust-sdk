use std::fmt;

const BSON_SPLITTER: &str = "$";
const TOKEN_LEN: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct InvalidBSONKeyError;

impl fmt::Display for InvalidBSONKeyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "invalid bson key")
    }
}

/// Key represents the key of the Document.
#[derive(Debug)]
pub struct Key {
    collection: String,
    document: String,
}

impl Key {
    /// from_bson_key creates an instance of Key from the received bsonKey.
    pub fn from_bson_key(bson_key: &str) -> Result<Key, InvalidBSONKeyError> {
        let mut splits = bson_key.split(BSON_SPLITTER).collect::<Vec<_>>();
        if splits.len() != TOKEN_LEN {
            return Err(InvalidBSONKeyError);
        }

        let collection = splits[0].to_string();
        let document = splits[1].to_string();
        Ok(Key {
            collection,
            document,
        })
    }

    /// bson_key returns the string of this key.
    pub fn bson_key(&self) -> String {
        format!("{}{}{}", self.collection, BSON_SPLITTER, self.document)
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
    fn from_bson_key() {
        // create success
        let key = Key::from_bson_key("collection$document").unwrap();
        assert_eq!(key.collection, "collection");
        assert_eq!(key.document, "document");

        // invalid bson
        let err = Key::from_bson_key("collection").unwrap_err();
        assert_eq!(err, InvalidBSONKeyError);
        let err = Key::from_bson_key("collection$document$bb").unwrap_err();
        assert_eq!(err, InvalidBSONKeyError);
    }

    #[test]
    fn bson_key() {
        // create success
        let bson_key = "collection$document";
        let key = Key::from_bson_key(bson_key).unwrap();
        assert_eq!(key.bson_key(), bson_key);
    }
}
