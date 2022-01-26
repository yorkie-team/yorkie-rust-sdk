use std::cmp;
use hex::{FromHex, FromHexError};

const ACTOR_ID_SIZE: usize = 12;

/// ActorID is bytes represented by the hexadecimal string.
/// It should be generated by unique value.
#[derive(Debug, PartialEq, Clone)]
pub struct ActorID {
    bytes: [u8; ACTOR_ID_SIZE],
}

impl ActorID {
    pub fn new(bytes: [u8; ACTOR_ID_SIZE]) -> ActorID {
        ActorID{bytes}
    }

    /// from_hex returns the ActorID represented by the hexadecimal string str.
    pub fn from_hex(hex_str: &str) -> Result<ActorID, FromHexError> {
        if hex_str == "" {
            return Err(FromHexError::InvalidStringLength);
        }

        let bytes = <[u8; 12]>::from_hex(hex_str)?;

        Ok(ActorID{bytes})
    }

    /// to_string returns the hexadecimal encoding of ActorID.
    pub fn to_string(&self) -> String {
        hex::encode(&self.bytes)
    }

    pub fn bytes(&self) -> &[u8; ACTOR_ID_SIZE] {
        &self.bytes
    }

    /// compare returns an cmp::Ordering comparing two ActorID lexicographically.
    pub fn compare(&self, other: &ActorID) -> cmp::Ordering {
        self.bytes.iter()
            .zip(other.bytes())
            .map(|(x, y)| x.cmp(y))
            .find(|&ord| ord != cmp::Ordering::Equal)
            .unwrap_or(
                self.bytes
                    .len()
                    .cmp(&other.bytes().len())
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_hex() {
        let err = ActorID::from_hex("").unwrap_err();
        assert_eq!(FromHexError::InvalidStringLength, err);

        assert!(!ActorID::from_hex("0123456789abcdef01234567").is_err());
    }

    #[test]
    fn to_string() {
        let hex_str = "0123456789abcdef01234567";
        let actor_id = ActorID::from_hex(hex_str).unwrap();
        assert_eq!(hex_str, actor_id.to_string());
    }

    #[test]
    fn compare() {
        let before_actor_id = ActorID::from_hex("0000000000abcdef01234567").unwrap();
        let after_actor_id  = ActorID::from_hex("0123456789abcdef01234567").unwrap();

        assert_eq!(cmp::Ordering::Less, before_actor_id.compare(&after_actor_id));
        assert_eq!(cmp::Ordering::Greater, after_actor_id.compare(&before_actor_id));
        assert_eq!(cmp::Ordering::Equal, before_actor_id.compare(&before_actor_id));
    }
}