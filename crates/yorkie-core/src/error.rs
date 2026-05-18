use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// The common error type used by Yorkie core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YorkieError {
    /// An object member key is not allowed by Yorkie's document model.
    InvalidObjectKey(String),

    /// A time ticket lamport value cannot be parsed as an integer.
    InvalidTimeTicketLamport(String),

    /// Primitive bytes do not match the expected primitive representation.
    InvalidPrimitiveBytes {
        primitive_type: &'static str,
        expected: usize,
        actual: usize,
    },

    /// Primitive string bytes are not valid UTF-8.
    InvalidPrimitiveUtf8,

    /// A requested CRDT element does not exist.
    MissingCrdtElement(String),

    /// An operation is missing its execution time.
    MissingExecutionTime,

    /// A requested object member does not exist.
    MissingKey(String),

    /// A CRDT element exists but has a different kind than expected.
    UnexpectedCrdtElement { id: String, expected: &'static str },

    /// A requested object member exists but has a different JSON type.
    UnexpectedType { key: String, expected: &'static str },

    /// A requested text position or range is not valid for the current text.
    InvalidTextPosition(String),

    /// A requested tree position, path, or index is not valid for the current tree.
    InvalidTreePosition(String),

    /// A weighted index lookup is out of range.
    InvalidIndex(String),

    /// A counter operation is invalid for the current counter mode.
    InvalidCounterOperation(String),

    /// The document has been removed and can no longer be edited.
    DocumentRemoved(String),

    /// Snapshot bytes were provided without a decoded root object.
    UnsupportedSnapshot,

    /// A core value does not have protocol conversion support yet.
    UnsupportedProtocolConversion(&'static str),
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
            Self::InvalidPrimitiveBytes {
                primitive_type,
                expected,
                actual,
            } => write!(
                f,
                "invalid primitive bytes for {primitive_type}: expected {expected}, got {actual}"
            ),
            Self::InvalidPrimitiveUtf8 => write!(f, "invalid primitive string bytes"),
            Self::MissingCrdtElement(id) => write!(f, "missing CRDT element {id:?}"),
            Self::MissingExecutionTime => write!(f, "operation executed_at is not set"),
            Self::MissingKey(key) => write!(f, "missing key {key:?}"),
            Self::UnexpectedCrdtElement { id, expected } => {
                write!(f, "unexpected CRDT element {id:?}: expected {expected}")
            }
            Self::UnexpectedType { key, expected } => {
                write!(f, "unexpected type for key {key:?}: expected {expected}")
            }
            Self::InvalidTextPosition(message) => write!(f, "invalid text position: {message}"),
            Self::InvalidTreePosition(message) => write!(f, "invalid tree position: {message}"),
            Self::InvalidIndex(message) => write!(f, "invalid index: {message}"),
            Self::InvalidCounterOperation(message) => {
                write!(f, "invalid counter operation: {message}")
            }
            Self::DocumentRemoved(key) => write!(f, "document {key:?} is removed"),
            Self::UnsupportedSnapshot => write!(f, "snapshot application requires a decoded root"),
            Self::UnsupportedProtocolConversion(value) => {
                write!(f, "unsupported protocol conversion for {value}")
            }
        }
    }
}

impl Error for YorkieError {}
