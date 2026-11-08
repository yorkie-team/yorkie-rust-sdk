use std::error::Error;
use std::fmt::{self, Display, Formatter};
use yorkie_core::{ErrorCode, YorkieError};

/// The codes of a push the server refuses for its content rather than for
/// conditions, so it refuses the same pack again (JS
/// `unretryableWriteCodes`).
pub(crate) const UNRETRYABLE_WRITE_CODES: [ErrorCode; 2] = [
    ErrorCode::ErrDocumentSizeExceedsLimit,
    ErrorCode::ErrChangeTooLarge,
];

/// Errors from the client lifecycle layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    ClientNotActivated(String),
    NotAttached(String),
    /// A resource with the same key is already attached or attaching
    /// (JS `ErrAlreadyAttached`).
    AlreadyAttached(String),
    NotDetached(String),
    InvalidArgument(String),
    Transport(String),
    /// Error reply from the server, with its Connect error code and, when
    /// the server sent them, the `code` and `reason` entries of the first
    /// `ErrorInfo` detail's metadata (JS `errorCodeOf` and
    /// `errorMetadataOf(err).reason`). The code is a Yorkie code such as
    /// `ErrSessionNotFound`; the reason is the auth webhook's reason for an
    /// `ErrUnauthenticated` or `ErrPermissionDenied`.
    Rpc {
        code: String,
        message: String,
        yorkie_code: Option<String>,
        reason: Option<String>,
    },
    Core(YorkieError),
}

pub type ClientResult<T> = std::result::Result<T, ClientError>;

/// What the client does after a failed server call (JS
/// `handleConnectError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorHandling {
    /// Retry the call.
    Retry,
    /// Get a new token from the auth token injector, if there is one, and
    /// retry the call.
    RefreshToken,
    /// Deactivate the client locally and stop.
    Deactivate,
    /// Stop.
    Stop,
}

impl ClientError {
    /// Returns the JS code of this error.
    ///
    /// For a server error it is the Yorkie code the server sent (JS
    /// `errorCodeOf`), or `None` when it sent none or one this SDK does not
    /// know. Lifecycle errors map to the code JS throws for them, and core
    /// errors to [`YorkieError::code`]. `Transport` errors have no code: JS
    /// sees them as Connect errors without one.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::ClientNotActivated(_) => Some(ErrorCode::ErrClientNotActivated),
            Self::NotAttached(_) => Some(ErrorCode::ErrNotAttached),
            Self::AlreadyAttached(_) => Some(ErrorCode::ErrAlreadyAttached),
            Self::NotDetached(_) => Some(ErrorCode::ErrNotDetached),
            Self::InvalidArgument(_) => Some(ErrorCode::ErrInvalidArgument),
            Self::Transport(_) => None,
            Self::Rpc { yorkie_code, .. } => yorkie_code.as_deref()?.parse().ok(),
            Self::Core(err) => Some(err.code()),
        }
    }

    /// Whether the server replied with the given Yorkie error code (JS
    /// `isErrorCode(err, code)`, which only looks at server errors).
    pub(crate) fn is_error_code(&self, expected: ErrorCode) -> bool {
        matches!(self, Self::Rpc { yorkie_code: Some(code), .. } if code == expected.as_str())
    }

    /// The reason the server gave in the error metadata (JS
    /// `errorMetadataOf(err).reason`).
    pub(crate) fn error_reason(&self) -> Option<&str> {
        match self {
            Self::Rpc { reason, .. } => reason.as_deref(),
            _ => None,
        }
    }

    /// What the client does after this error (JS `handleConnectError`), in
    /// JS order:
    ///
    /// 1. A push refused for its size (`ErrDocumentSizeExceedsLimit`,
    ///    `ErrChangeTooLarge`) is not retried: the server refuses the same
    ///    pack again.
    /// 2. The Connect codes `canceled`, `unknown`, `resource_exhausted`, and
    ///    `unavailable` are retried, as are `Transport` errors, which JS sees
    ///    as `unavailable` or `unknown`.
    /// 3. `ErrUnauthenticated` refreshes the auth token and is retried.
    /// 4. `ErrTooManySubscribers` is retried.
    /// 5. `ErrClientNotActivated` and `ErrClientNotFound` deactivate the
    ///    client.
    /// 6. Everything else, including `ErrTooManyAttachments`,
    ///    `ErrEpochMismatch`, and errors the client raised itself, stops.
    pub(crate) fn handling(&self) -> ErrorHandling {
        let code = match self {
            Self::Transport(_) => return ErrorHandling::Retry,
            Self::Rpc { code, .. } => code,
            _ => return ErrorHandling::Stop,
        };
        if UNRETRYABLE_WRITE_CODES
            .iter()
            .any(|write_code| self.is_error_code(*write_code))
        {
            return ErrorHandling::Stop;
        }
        if matches!(
            code.as_str(),
            "canceled" | "unknown" | "resource_exhausted" | "unavailable"
        ) {
            return ErrorHandling::Retry;
        }
        if self.is_error_code(ErrorCode::ErrUnauthenticated) {
            return ErrorHandling::RefreshToken;
        }
        if self.is_error_code(ErrorCode::ErrTooManySubscribers) {
            return ErrorHandling::Retry;
        }
        if self.is_error_code(ErrorCode::ErrClientNotActivated)
            || self.is_error_code(ErrorCode::ErrClientNotFound)
        {
            return ErrorHandling::Deactivate;
        }
        ErrorHandling::Stop
    }

    /// Whether a background loop should retry after this error (JS
    /// `handleConnectError` returning true).
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(
            self.handling(),
            ErrorHandling::Retry | ErrorHandling::RefreshToken
        )
    }
}

impl Display for ClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientNotActivated(key) => write!(f, "client {key:?} is not active"),
            Self::NotAttached(key) => write!(f, "resource {key:?} is not attached"),
            Self::AlreadyAttached(key) => write!(f, "resource {key:?} is already attached"),
            Self::NotDetached(key) => write!(f, "resource {key:?} is not detached"),
            Self::InvalidArgument(message) => write!(f, "invalid client argument: {message}"),
            Self::Transport(message) => write!(f, "client transport error: {message}"),
            Self::Rpc { code, message, .. } => write!(f, "rpc error {code}: {message}"),
            Self::Core(err) => Display::fmt(err, f),
        }
    }
}

impl Error for ClientError {}

impl From<YorkieError> for ClientError {
    fn from(value: YorkieError) -> Self {
        Self::Core(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientError, ErrorHandling};
    use yorkie_core::{ErrorCode, YorkieError};

    fn rpc(code: &str, yorkie_code: Option<ErrorCode>) -> ClientError {
        ClientError::Rpc {
            code: code.to_owned(),
            message: String::new(),
            yorkie_code: yorkie_code.map(|code| code.as_str().to_owned()),
            reason: None,
        }
    }

    /// JS `handleConnectError`: which errors are retried, refresh the token,
    /// deactivate the client, or stop.
    #[test]
    fn handles_errors_like_js_handle_connect_error() {
        assert!(ClientError::Transport("connection reset".to_owned()).is_retryable());
        for code in ["canceled", "unknown", "resource_exhausted", "unavailable"] {
            assert_eq!(ErrorHandling::Retry, rpc(code, None).handling(), "{code}");
        }
        assert_eq!(
            ErrorHandling::RefreshToken,
            rpc("unauthenticated", Some(ErrorCode::ErrUnauthenticated)).handling()
        );
        // JS only refreshes on the Yorkie code: a bare 401 stops.
        assert_eq!(ErrorHandling::Stop, rpc("unauthenticated", None).handling());
        assert_eq!(
            ErrorHandling::Retry,
            rpc(
                "failed_precondition",
                Some(ErrorCode::ErrTooManySubscribers)
            )
            .handling()
        );
        for code in [
            ErrorCode::ErrClientNotActivated,
            ErrorCode::ErrClientNotFound,
        ] {
            assert_eq!(
                ErrorHandling::Deactivate,
                rpc("failed_precondition", Some(code)).handling(),
                "{code}"
            );
        }
        // The server sends size rejections as `resource_exhausted`, which is
        // otherwise retried.
        for code in [
            ErrorCode::ErrDocumentSizeExceedsLimit,
            ErrorCode::ErrChangeTooLarge,
        ] {
            assert_eq!(
                ErrorHandling::Stop,
                rpc("resource_exhausted", Some(code)).handling(),
                "{code}"
            );
        }
        for code in [
            ErrorCode::ErrTooManyAttachments,
            ErrorCode::ErrEpochMismatch,
            ErrorCode::ErrPermissionDenied,
        ] {
            assert!(
                !rpc("failed_precondition", Some(code)).is_retryable(),
                "{code}"
            );
        }
        assert!(!rpc("not_found", None).is_retryable());
        assert!(!ClientError::NotAttached("doc".to_owned()).is_retryable());
        assert!(!ClientError::Core(YorkieError::DocumentRemoved("doc".to_owned())).is_retryable());
    }

    #[test]
    fn maps_errors_to_js_codes() {
        assert_eq!(
            Some(ErrorCode::ErrUnauthenticated),
            rpc("unauthenticated", Some(ErrorCode::ErrUnauthenticated)).code()
        );
        assert_eq!(None, rpc("unauthenticated", None).code());
        let unknown = ClientError::Rpc {
            code: "internal".to_owned(),
            message: String::new(),
            yorkie_code: Some("ErrSomethingNew".to_owned()),
            reason: None,
        };
        assert_eq!(None, unknown.code());
        assert_eq!(None, ClientError::Transport("reset".to_owned()).code());
        assert_eq!(
            Some(ErrorCode::ErrClientNotActivated),
            ClientError::ClientNotActivated("key".to_owned()).code()
        );
        assert_eq!(
            Some(ErrorCode::ErrNotAttached),
            ClientError::NotAttached("doc".to_owned()).code()
        );
        assert_eq!(
            Some(ErrorCode::ErrAlreadyAttached),
            ClientError::AlreadyAttached("doc".to_owned()).code()
        );
        assert_eq!(
            Some(ErrorCode::ErrNotDetached),
            ClientError::NotDetached("doc".to_owned()).code()
        );
        assert_eq!(
            Some(ErrorCode::ErrInvalidArgument),
            ClientError::InvalidArgument("x".to_owned()).code()
        );
        assert_eq!(
            Some(ErrorCode::ErrDocumentSizeExceedsLimit),
            ClientError::Core(YorkieError::DocumentSizeExceedsLimit { size: 2, limit: 1 }).code()
        );
    }
}
