use crate::error::ClientResult;
use std::collections::BTreeMap;
use yorkie_core::{ActorId, ChangePack, SchemaRule};

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

/// Request data for attaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachDocumentRequest {
    pub client_id: ActorId,
    pub change_pack: ChangePack,
    pub schema_key: Option<String>,
    pub shard_key: String,
}

/// Response data for attaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachDocumentResponse {
    pub document_id: String,
    pub change_pack: ChangePack,
    pub max_size_per_document: usize,
    pub schema_rules: Vec<SchemaRule>,
}

/// Request data for detaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachDocumentRequest {
    pub client_id: ActorId,
    pub document_id: String,
    pub change_pack: ChangePack,
    pub remove_if_not_attached: bool,
    pub shard_key: String,
}

/// Response data for detaching a document.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachDocumentResponse {
    pub change_pack: ChangePack,
}

/// Request data for removing a document.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveDocumentRequest {
    pub client_id: ActorId,
    pub document_id: String,
    pub change_pack: ChangePack,
    pub shard_key: String,
}

/// Response data for removing a document.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveDocumentResponse {
    pub change_pack: ChangePack,
}

/// Request data for pushing and pulling document changes.
#[derive(Debug, Clone, PartialEq)]
pub struct PushPullChangesRequest {
    pub client_id: ActorId,
    pub document_id: String,
    pub change_pack: ChangePack,
    pub push_only: bool,
    pub shard_key: String,
}

/// Response data for pushing and pulling document changes.
#[derive(Debug, Clone, PartialEq)]
pub struct PushPullChangesResponse {
    pub change_pack: ChangePack,
}

/// Transport boundary used by the client lifecycle and document attachment.
#[allow(async_fn_in_trait)]
pub trait ClientTransport {
    async fn activate_client(
        &mut self,
        request: ActivateClientRequest,
    ) -> ClientResult<ActivateClientResponse>;

    async fn deactivate_client(
        &mut self,
        request: DeactivateClientRequest,
    ) -> ClientResult<DeactivateClientResponse>;

    async fn attach_document(
        &mut self,
        request: AttachDocumentRequest,
    ) -> ClientResult<AttachDocumentResponse>;

    async fn detach_document(
        &mut self,
        request: DetachDocumentRequest,
    ) -> ClientResult<DetachDocumentResponse>;

    async fn remove_document(
        &mut self,
        request: RemoveDocumentRequest,
    ) -> ClientResult<RemoveDocumentResponse>;

    async fn push_pull_changes(
        &mut self,
        request: PushPullChangesRequest,
    ) -> ClientResult<PushPullChangesResponse>;
}
