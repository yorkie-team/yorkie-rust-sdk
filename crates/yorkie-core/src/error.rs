use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

/// Declares [`ErrorCode`] with one variant per JS `Code` and its string.
macro_rules! error_codes {
    ($($(#[$doc:meta])* $variant:ident => $code:literal,)+) => {
        /// A Yorkie error code (JS `Code` in `util/error.ts`). The variants
        /// keep the JS names, and [`ErrorCode::as_str`] returns the JS string,
        /// which is also what the server sends in the `code` entry of an error's
        /// `ErrorInfo` metadata.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ErrorCode {
            $($(#[$doc])* $variant,)+
        }

        impl ErrorCode {
            /// Every code, in JS declaration order.
            pub const ALL: &'static [ErrorCode] = &[$(ErrorCode::$variant,)+];

            /// Returns the JS code string, such as `"ErrClientNotFound"`.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $code,)+
                }
            }
        }
    };
}

error_codes! {
    /// The operation completed successfully.
    Ok => "ok",
    /// The client is not active.
    ErrClientNotActivated => "ErrClientNotActivated",
    /// The client is not found.
    ErrClientNotFound => "ErrClientNotFound",
    /// The operation is not implemented.
    ErrUnimplemented => "ErrUnimplemented",
    /// The type is invalid.
    ErrInvalidType => "ErrInvalidType",
    /// Used to verify errors in tests.
    ErrDummy => "ErrDummy",
    /// The resource is not attached.
    ErrNotAttached => "ErrNotAttached",
    /// The resource is not detached.
    ErrNotDetached => "ErrNotDetached",
    /// A resource with the same key is already attached or attaching.
    ErrAlreadyAttached => "ErrAlreadyAttached",
    /// The server no longer has the channel session.
    ErrSessionNotFound => "ErrSessionNotFound",
    /// The document is removed.
    ErrDocumentRemoved => "ErrDocumentRemoved",
    /// The document size exceeds the limit.
    ErrDocumentSizeExceedsLimit => "ErrDocumentSizeExceedsLimit",
    /// A single change is too large to store.
    ErrChangeTooLarge => "ErrChangeTooLarge",
    /// The document schema validation failed.
    ErrDocumentSchemaValidationFailed => "ErrDocumentSchemaValidationFailed",
    /// The object key is invalid.
    ErrInvalidObjectKey => "ErrInvalidObjectKey",
    /// The argument is invalid.
    ErrInvalidArgument => "ErrInvalidArgument",
    /// Required initialization has not been completed.
    ErrNotInitialized => "ErrNotInitialized",
    /// The following actions are not ready to run.
    ErrNotReady => "ErrNotReady",
    /// The execution is rejected.
    ErrRefused => "ErrRefused",
    /// A required React context is missing (JS only).
    ErrContextNotProvided => "ErrContextNotProvided",
    /// The authorization webhook denied the request.
    ErrPermissionDenied => "ErrPermissionDenied",
    /// The request has no valid authentication credentials.
    ErrUnauthenticated => "ErrUnauthenticated",
    /// The number of subscribers exceeds the limit.
    ErrTooManySubscribers => "ErrTooManySubscribers",
    /// The number of attachments exceeds the limit.
    ErrTooManyAttachments => "ErrTooManyAttachments",
    /// The document was compacted and the client's epoch no longer matches.
    ErrEpochMismatch => "ErrEpochMismatch",
    /// Another session holds the offline-persistence lock of the document.
    ErrDocumentOpenElsewhere => "ErrDocumentOpenElsewhere",
    /// A change cannot be applied to a document.
    ErrChangeApplyFailed => "ErrChangeApplyFailed",
    /// The server has no base to fold a presence patch into.
    ErrPresenceBaseUnavailable => "ErrPresenceBaseUnavailable",
    /// A watch stream stayed silent longer than its heartbeats allow.
    ErrWatchStreamIdle => "ErrWatchStreamIdle",
}

impl Display for ErrorCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ErrorCode {
    type Err = YorkieError;

    /// Parses a JS code string. A string that is not a JS code is an
    /// `InvalidArgument` error.
    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|code| code.as_str() == value)
            .ok_or_else(|| YorkieError::InvalidArgument(format!("unknown error code {value:?}")))
    }
}

/// The common error type used by Yorkie core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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

    /// A JSON value was used before it belongs to a document update (JS
    /// `ErrNotInitialized`).
    NotInitialized(String),

    /// An argument is not valid for the requested call (JS
    /// `ErrInvalidArgument`).
    InvalidArgument(String),

    /// The requested feature is not implemented yet (JS `ErrUnimplemented`).
    Unimplemented(String),

    /// The execution is rejected (JS `ErrRefused`).
    Refused(String),

    /// A local update leaves the document violating its schema rules (JS
    /// code `ErrDocumentSchemaValidationFailed`). Holds one message per
    /// violation.
    SchemaValidationFailed(Vec<String>),

    /// A local update makes the document larger than its maximum size (JS
    /// code `ErrDocumentSizeExceedsLimit`). Sizes are in bytes.
    DocumentSizeExceedsLimit { size: usize, limit: usize },
}

/// Convenient result alias for Yorkie core operations.
pub type Result<T> = std::result::Result<T, YorkieError>;

impl YorkieError {
    /// Returns the JS code of this error (JS `YorkieError.code`).
    ///
    /// The variants that JS has as codes map to them. The others map to the
    /// code JS throws for the same failure: a missing element, key, or
    /// execution time and an invalid position, index, lamport, primitive, or
    /// counter operation are `ErrInvalidArgument`, as in JS `CRDTRoot` and
    /// the operations; a value of the wrong JSON or CRDT kind is
    /// `ErrInvalidType`; a missing conversion or snapshot decoding is
    /// `ErrUnimplemented`.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidObjectKey(_) => ErrorCode::ErrInvalidObjectKey,
            Self::InvalidTimeTicketLamport(_)
            | Self::InvalidPrimitiveBytes { .. }
            | Self::InvalidPrimitiveUtf8
            | Self::MissingCrdtElement(_)
            | Self::MissingExecutionTime
            | Self::MissingKey(_)
            | Self::InvalidTextPosition(_)
            | Self::InvalidTreePosition(_)
            | Self::InvalidIndex(_)
            | Self::InvalidCounterOperation(_)
            | Self::InvalidArgument(_) => ErrorCode::ErrInvalidArgument,
            Self::UnexpectedCrdtElement { .. } | Self::UnexpectedType { .. } => {
                ErrorCode::ErrInvalidType
            }
            Self::DocumentRemoved(_) => ErrorCode::ErrDocumentRemoved,
            Self::UnsupportedSnapshot
            | Self::UnsupportedProtocolConversion(_)
            | Self::Unimplemented(_) => ErrorCode::ErrUnimplemented,
            Self::NotInitialized(_) => ErrorCode::ErrNotInitialized,
            Self::Refused(_) => ErrorCode::ErrRefused,
            Self::SchemaValidationFailed(_) => ErrorCode::ErrDocumentSchemaValidationFailed,
            Self::DocumentSizeExceedsLimit { .. } => ErrorCode::ErrDocumentSizeExceedsLimit,
        }
    }
}

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
            Self::NotInitialized(message) => write!(f, "not initialized: {message}"),
            Self::InvalidArgument(message) => write!(f, "invalid argument: {message}"),
            Self::Unimplemented(message) => write!(f, "unimplemented: {message}"),
            Self::Refused(message) => write!(f, "refused: {message}"),
            Self::SchemaValidationFailed(errors) => {
                write!(f, "schema validation failed: {}", errors.join(", "))
            }
            Self::DocumentSizeExceedsLimit { size, limit } => {
                write!(f, "document size exceeded: {size} > {limit}")
            }
        }
    }
}

impl Error for YorkieError {}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, YorkieError};

    /// JS `Code`: every code round-trips through its string, and the
    /// strings are the JS ones.
    #[test]
    fn error_codes_round_trip_through_js_strings() {
        assert_eq!(29, ErrorCode::ALL.len());
        for code in ErrorCode::ALL {
            assert_eq!(Ok(*code), code.as_str().parse::<ErrorCode>());
            assert_eq!(code.as_str(), code.to_string());
        }
        assert_eq!("ok", ErrorCode::Ok.as_str());
        assert_eq!("ErrUnauthenticated", ErrorCode::ErrUnauthenticated.as_str());
        assert_eq!(
            "ErrDocumentSizeExceedsLimit",
            ErrorCode::ErrDocumentSizeExceedsLimit.as_str()
        );
        assert!(matches!(
            "ErrNoSuchCode".parse::<ErrorCode>(),
            Err(YorkieError::InvalidArgument(_))
        ));
        assert!("errunauthenticated".parse::<ErrorCode>().is_err());
    }

    #[test]
    fn core_errors_map_to_js_codes() {
        let cases = [
            (
                YorkieError::InvalidObjectKey("a.b".to_owned()),
                ErrorCode::ErrInvalidObjectKey,
            ),
            (
                YorkieError::DocumentRemoved("doc".to_owned()),
                ErrorCode::ErrDocumentRemoved,
            ),
            (
                YorkieError::NotInitialized("counter".to_owned()),
                ErrorCode::ErrNotInitialized,
            ),
            (
                YorkieError::InvalidArgument("x".to_owned()),
                ErrorCode::ErrInvalidArgument,
            ),
            (
                YorkieError::MissingCrdtElement("1:a:0".to_owned()),
                ErrorCode::ErrInvalidArgument,
            ),
            (
                YorkieError::UnexpectedType {
                    key: "k".to_owned(),
                    expected: "text",
                },
                ErrorCode::ErrInvalidType,
            ),
            (
                YorkieError::Unimplemented("x".to_owned()),
                ErrorCode::ErrUnimplemented,
            ),
            (
                YorkieError::UnsupportedProtocolConversion("x"),
                ErrorCode::ErrUnimplemented,
            ),
            (
                YorkieError::SchemaValidationFailed(vec!["bad".to_owned()]),
                ErrorCode::ErrDocumentSchemaValidationFailed,
            ),
            (
                YorkieError::DocumentSizeExceedsLimit { size: 2, limit: 1 },
                ErrorCode::ErrDocumentSizeExceedsLimit,
            ),
        ];
        for (error, code) in cases {
            assert_eq!(code, error.code(), "{error:?}");
        }
    }
}
