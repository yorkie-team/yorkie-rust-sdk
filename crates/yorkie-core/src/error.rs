use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// The common error type used by Yorkie core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YorkieError {
    /// An object member key is not allowed by Yorkie's document model.
    InvalidObjectKey(String),

    /// A time ticket lamport value cannot be parsed as an integer.
    InvalidTimeTicketLamport(String),

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
            Self::InvalidObjectKey(key) => {
                write!(f, "invalid object key {key:?}: key must not contain '.'")
            }
            Self::InvalidTimeTicketLamport(lamport) => {
                write!(f, "invalid time ticket lamport {lamport:?}")
            }
            Self::MissingKey(key) => write!(f, "missing key {key:?}"),
            Self::UnexpectedType { key, expected } => {
                write!(f, "unexpected type for key {key:?}: expected {expected}")
            }
        }
    }
}

impl Error for YorkieError {}
