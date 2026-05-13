use std::borrow::Borrow;
use std::fmt::{self, Display, Formatter};

pub const INITIAL_ACTOR_ID: &str = "000000000000000000000000";
pub const MAX_ACTOR_ID: &str = "FFFFFFFFFFFFFFFFFFFFFFFF";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorId(String);

impl ActorId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn initial() -> Self {
        Self::new(INITIAL_ACTOR_ID)
    }

    pub fn max() -> Self {
        Self::new(MAX_ACTOR_ID)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for ActorId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for ActorId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Display for ActorId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ActorId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ActorId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}
