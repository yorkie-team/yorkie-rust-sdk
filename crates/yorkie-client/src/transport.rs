use crate::error::ClientResult;
use std::collections::BTreeMap;
use yorkie_core::ActorId;

/// Request data for activating a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateClientRequest {
    pub client_key: String,
    pub metadata: BTreeMap<String, String>,
    pub shard_key: String,
}

/// Response data for activating a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateClientResponse {
    pub client_id: ActorId,
}

/// Request data for deactivating a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeactivateClientRequest {
    pub client_id: ActorId,
    pub synchronous: bool,
    pub shard_key: String,
}

/// Response data for deactivating a client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeactivateClientResponse;

/// Transport boundary used by the client lifecycle.
pub trait ClientTransport {
    fn activate_client(
        &mut self,
        request: ActivateClientRequest,
    ) -> ClientResult<ActivateClientResponse>;

    fn deactivate_client(
        &mut self,
        request: DeactivateClientRequest,
    ) -> ClientResult<DeactivateClientResponse>;
}
