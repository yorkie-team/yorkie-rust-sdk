use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// The common error type used by Yorkie core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YorkieError {
    /// The document or channel key does not match Yorkie's resource key rules.
    InvalidKey(String),

    /// A requested object member does not exist.
    MissingKey(String),

    /// A requested object member exists but has a different JSON type.
    UnexpectedType { key: String, expected: &'static str },
}

/// Convenient result alias for Yorkie core operations.
pub type Result<T> = std::result::Result<T, YorkieError>;

impl Display for YorkieError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(key) => {
                write!(f, "invalid key {key:?}: expected 4-120 slug characters")
            }
            Self::MissingKey(key) => write!(f, "missing key {key:?}"),
            Self::UnexpectedType { key, expected } => {
                write!(f, "unexpected type for key {key:?}: expected {expected}")
            }
        }
    }
}

impl Error for YorkieError {}
