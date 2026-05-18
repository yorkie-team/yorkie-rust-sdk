use std::error::Error;
use std::fmt::{self, Display, Formatter};
use yorkie_core::YorkieError;

/// Errors from the client lifecycle layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    ClientNotActivated(String),
    NotAttached(String),
    NotDetached(String),
    InvalidArgument(String),
    Transport(String),
    Core(YorkieError),
}

pub type ClientResult<T> = std::result::Result<T, ClientError>;

impl Display for ClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientNotActivated(key) => write!(f, "client {key:?} is not active"),
            Self::NotAttached(key) => write!(f, "resource {key:?} is not attached"),
            Self::NotDetached(key) => write!(f, "resource {key:?} is not detached"),
            Self::InvalidArgument(message) => write!(f, "invalid client argument: {message}"),
            Self::Transport(message) => write!(f, "client transport error: {message}"),
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
