use std::error::Error;
use std::fmt::{self, Display, Formatter};
use yorkie_core::YorkieError;

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
    /// the server sent one, the Yorkie error code from the error details
    /// (JS `errorCodeOf`, such as `ErrSessionNotFound`).
    Rpc {
        code: String,
        message: String,
        yorkie_code: Option<String>,
    },
    Core(YorkieError),
}

pub type ClientResult<T> = std::result::Result<T, ClientError>;

impl ClientError {
    /// Whether a background loop should retry after this error (JS
    /// `handleConnectError` returning true).
    ///
    /// Network failures are retried, as JS sees them as `Unavailable` or
    /// `Unknown`. Server errors are retried for the Connect codes JS retries.
    /// Everything else stops the loop.
    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Rpc { code, .. } => matches!(
                code.as_str(),
                "canceled" | "unknown" | "resource_exhausted" | "unavailable" | "unauthenticated"
            ),
            _ => false,
        }
    }

    /// Whether the server replied with the given Yorkie error code (JS
    /// `isErrorCode(err, code)`).
    pub(crate) fn has_yorkie_code(&self, expected: &str) -> bool {
        matches!(self, Self::Rpc { yorkie_code: Some(code), .. } if code == expected)
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
    use super::ClientError;
    use yorkie_core::YorkieError;

    #[test]
    fn retries_network_and_transient_server_errors_like_js() {
        let rpc = |code: &str| ClientError::Rpc {
            code: code.to_owned(),
            message: String::new(),
            yorkie_code: None,
        };
        assert!(ClientError::Transport("connection reset".to_owned()).is_retryable());
        for code in [
            "canceled",
            "unknown",
            "resource_exhausted",
            "unavailable",
            "unauthenticated",
        ] {
            assert!(rpc(code).is_retryable(), "{code}");
        }
        assert!(!rpc("failed_precondition").is_retryable());
        assert!(!rpc("not_found").is_retryable());
        assert!(!ClientError::NotAttached("doc".to_owned()).is_retryable());
        assert!(!ClientError::Core(YorkieError::DocumentRemoved("doc".to_owned())).is_retryable());
    }
}
