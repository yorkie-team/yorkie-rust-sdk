// The module mirrors the JS `change/change.ts` layout.
#[allow(clippy::module_inception)]
mod change;
mod checkpoint;
mod context;
mod id;
mod pack;

pub(crate) use change::{Change, ChangeExecutionResult, TreeNodeRepoint};
pub use checkpoint::{
    Checkpoint, INITIAL_CHECKPOINT, INITIAL_CLIENT_SEQ, INITIAL_SERVER_SEQ, MAX_CHECKPOINT,
    MAX_CLIENT_SEQ, MAX_SERVER_SEQ,
};
pub(crate) use context::ChangeContext;
pub(crate) use id::ChangeId;
pub use pack::ChangePack;
