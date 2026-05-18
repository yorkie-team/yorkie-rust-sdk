//! Protobuf conversion helpers for client transport messages.

use crate::transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
    DetachDocumentResponse, PushPullChangesRequest, PushPullChangesResponse, RemoveDocumentRequest,
    RemoveDocumentResponse,
};
use yorkie_core::ActorId;
use yorkie_protocol::converter::{self, ProtocolError};
use yorkie_protocol::yorkie::v1 as api;

pub type Result<T> = converter::Result<T>;

pub fn to_activate_client_request(request: &ActivateClientRequest) -> api::ActivateClientRequest {
    api::ActivateClientRequest {
        client_key: request.client_key.clone(),
        metadata: request.metadata.clone(),
    }
}

pub fn from_activate_client_response(
    response: &api::ActivateClientResponse,
) -> ActivateClientResponse {
    ActivateClientResponse {
        client_id: ActorId::new(response.client_id.clone()),
    }
}

pub fn to_deactivate_client_request(
    request: &DeactivateClientRequest,
) -> api::DeactivateClientRequest {
    api::DeactivateClientRequest {
        client_id: request.client_id.to_string(),
        synchronous: request.synchronous,
    }
}

pub fn from_deactivate_client_response(
    _response: &api::DeactivateClientResponse,
) -> DeactivateClientResponse {
    DeactivateClientResponse
}

pub fn to_attach_document_request(
    request: &AttachDocumentRequest,
) -> Result<api::AttachDocumentRequest> {
    Ok(api::AttachDocumentRequest {
        client_id: request.client_id.to_string(),
        change_pack: Some(converter::to_change_pack(&request.change_pack)?),
        schema_key: request.schema_key.clone().unwrap_or_default(),
    })
}

pub fn from_attach_document_response(
    response: &api::AttachDocumentResponse,
) -> Result<AttachDocumentResponse> {
    Ok(AttachDocumentResponse {
        document_id: response.document_id.clone(),
        change_pack: converter::from_change_pack(required(
            &response.change_pack,
            "attach_document_response.change_pack",
        )?)?,
        max_size_per_document: usize_from_i32(
            response.max_size_per_document,
            "attach_document_response.max_size_per_document",
        )?,
        schema_rules: converter::from_schema_rules(&response.schema_rules),
    })
}

pub fn to_detach_document_request(
    request: &DetachDocumentRequest,
) -> Result<api::DetachDocumentRequest> {
    Ok(api::DetachDocumentRequest {
        client_id: request.client_id.to_string(),
        document_id: request.document_id.clone(),
        change_pack: Some(converter::to_change_pack(&request.change_pack)?),
        remove_if_not_attached: request.remove_if_not_attached,
    })
}

pub fn from_detach_document_response(
    response: &api::DetachDocumentResponse,
) -> Result<DetachDocumentResponse> {
    Ok(DetachDocumentResponse {
        change_pack: converter::from_change_pack(required(
            &response.change_pack,
            "detach_document_response.change_pack",
        )?)?,
    })
}

pub fn to_remove_document_request(
    request: &RemoveDocumentRequest,
) -> Result<api::RemoveDocumentRequest> {
    Ok(api::RemoveDocumentRequest {
        client_id: request.client_id.to_string(),
        document_id: request.document_id.clone(),
        change_pack: Some(converter::to_change_pack(&request.change_pack)?),
    })
}

pub fn from_remove_document_response(
    response: &api::RemoveDocumentResponse,
) -> Result<RemoveDocumentResponse> {
    Ok(RemoveDocumentResponse {
        change_pack: converter::from_change_pack(required(
            &response.change_pack,
            "remove_document_response.change_pack",
        )?)?,
    })
}

pub fn to_push_pull_changes_request(
    request: &PushPullChangesRequest,
) -> Result<api::PushPullChangesRequest> {
    Ok(api::PushPullChangesRequest {
        client_id: request.client_id.to_string(),
        document_id: request.document_id.clone(),
        change_pack: Some(converter::to_change_pack(&request.change_pack)?),
        push_only: request.push_only,
    })
}

pub fn from_push_pull_changes_response(
    response: &api::PushPullChangesResponse,
) -> Result<PushPullChangesResponse> {
    Ok(PushPullChangesResponse {
        change_pack: converter::from_change_pack(required(
            &response.change_pack,
            "push_pull_changes_response.change_pack",
        )?)?,
    })
}

fn required<'a, T>(value: &'a Option<T>, field: &'static str) -> Result<&'a T> {
    value.as_ref().ok_or(ProtocolError::MissingField(field))
}

fn usize_from_i32(value: i32, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| ProtocolError::InvalidInteger { field, value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{
        ActivateClientRequest, AttachDocumentRequest, DeactivateClientRequest,
        DetachDocumentRequest, PushPullChangesRequest, RemoveDocumentRequest,
    };
    use std::collections::BTreeMap;
    use yorkie_core::{Document, SchemaRule, TreeNodeRule};

    #[test]
    fn converts_activate_client_messages() {
        let request = ActivateClientRequest {
            client_key: "client-key".to_owned(),
            metadata: BTreeMap::from([("region".to_owned(), "local".to_owned())]),
            shard_key: "api-key/client-key".to_owned(),
        };

        let proto_request = to_activate_client_request(&request);
        assert_eq!("client-key", proto_request.client_key);
        assert_eq!(
            Some("local"),
            proto_request.metadata.get("region").map(String::as_str)
        );

        let response = from_activate_client_response(&api::ActivateClientResponse {
            client_id: "000000000000000000000001".to_owned(),
        });
        assert_eq!("000000000000000000000001", response.client_id.as_str());
    }

    #[test]
    fn converts_deactivate_client_messages() {
        let request = DeactivateClientRequest {
            client_id: ActorId::new("000000000000000000000001"),
            synchronous: true,
            shard_key: "api-key/client-key".to_owned(),
        };

        let proto_request = to_deactivate_client_request(&request);
        assert_eq!("000000000000000000000001", proto_request.client_id);
        assert!(proto_request.synchronous);
        assert_eq!(
            DeactivateClientResponse,
            from_deactivate_client_response(&api::DeactivateClientResponse {})
        );
    }

    #[test]
    fn converts_attach_document_messages() -> Result<()> {
        let mut doc = Document::new("doc-key");
        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        let request = AttachDocumentRequest {
            client_id: ActorId::new("000000000000000000000001"),
            change_pack: doc.create_change_pack(),
            schema_key: Some("schema".to_owned()),
            shard_key: "api-key/doc-key".to_owned(),
        };

        let proto_request = to_attach_document_request(&request)?;
        assert_eq!("000000000000000000000001", proto_request.client_id);
        assert_eq!("schema", proto_request.schema_key);
        let request_pack = proto_request.change_pack.as_ref().unwrap();
        assert_eq!("doc-key", request_pack.document_key);
        assert_eq!(1, request_pack.changes.len());

        let proto_response = api::AttachDocumentResponse {
            document_id: "document-id".to_owned(),
            change_pack: Some(request_pack.clone()),
            max_size_per_document: 4096,
            schema_rules: converter::to_schema_rules(&[SchemaRule::new(
                "$.profile",
                "object",
                vec![TreeNodeRule::new("paragraph", "text*", "bold", "block")],
            )]),
        };
        let response = from_attach_document_response(&proto_response)?;
        assert_eq!("document-id", response.document_id);
        assert_eq!("doc-key", response.change_pack.document_key());
        assert_eq!(4096, response.max_size_per_document);
        assert_eq!(
            Some("$.profile"),
            response.schema_rules.first().map(|rule| rule.path.as_str())
        );
        Ok(())
    }

    #[test]
    fn converts_detach_document_messages() -> Result<()> {
        let doc = Document::new("doc-key");
        let request = DetachDocumentRequest {
            client_id: ActorId::new("000000000000000000000001"),
            document_id: "document-id".to_owned(),
            change_pack: doc.create_change_pack(),
            remove_if_not_attached: true,
            shard_key: "api-key/doc-key".to_owned(),
        };

        let proto_request = to_detach_document_request(&request)?;
        assert_eq!("000000000000000000000001", proto_request.client_id);
        assert_eq!("document-id", proto_request.document_id);
        assert!(proto_request.remove_if_not_attached);

        let response = from_detach_document_response(&api::DetachDocumentResponse {
            change_pack: proto_request.change_pack,
        })?;
        assert_eq!("doc-key", response.change_pack.document_key());
        Ok(())
    }

    #[test]
    fn converts_remove_document_messages() -> Result<()> {
        let doc = Document::new("doc-key");
        let mut change_pack = doc.create_change_pack();
        change_pack.set_removed(true);
        let request = RemoveDocumentRequest {
            client_id: ActorId::new("000000000000000000000001"),
            document_id: "document-id".to_owned(),
            change_pack,
            shard_key: "api-key/doc-key".to_owned(),
        };

        let proto_request = to_remove_document_request(&request)?;
        assert_eq!("000000000000000000000001", proto_request.client_id);
        assert_eq!("document-id", proto_request.document_id);
        assert!(proto_request.change_pack.as_ref().unwrap().is_removed);

        let response = from_remove_document_response(&api::RemoveDocumentResponse {
            change_pack: proto_request.change_pack,
        })?;
        assert!(response.change_pack.is_removed());
        Ok(())
    }

    #[test]
    fn converts_push_pull_changes_messages() -> Result<()> {
        let mut doc = Document::new("doc-key");
        doc.update(|root| root.set("title", "hello").map(|_| ()))?;
        let request = PushPullChangesRequest {
            client_id: ActorId::new("000000000000000000000001"),
            document_id: "document-id".to_owned(),
            change_pack: doc.create_change_pack(),
            push_only: true,
            shard_key: "api-key/doc-key".to_owned(),
        };

        let proto_request = to_push_pull_changes_request(&request)?;
        assert_eq!("000000000000000000000001", proto_request.client_id);
        assert_eq!("document-id", proto_request.document_id);
        assert!(proto_request.push_only);
        assert_eq!(1, proto_request.change_pack.as_ref().unwrap().changes.len());

        let response = from_push_pull_changes_response(&api::PushPullChangesResponse {
            change_pack: proto_request.change_pack,
        })?;
        assert_eq!("doc-key", response.change_pack.document_key());
        assert!(response.change_pack.has_changes());
        Ok(())
    }

    #[test]
    fn rejects_responses_without_change_pack() {
        assert_eq!(
            ProtocolError::MissingField("attach_document_response.change_pack"),
            from_attach_document_response(&api::AttachDocumentResponse::default()).unwrap_err()
        );
        assert_eq!(
            ProtocolError::MissingField("push_pull_changes_response.change_pack"),
            from_push_pull_changes_response(&api::PushPullChangesResponse::default()).unwrap_err()
        );
    }

    #[test]
    fn rejects_negative_attach_response_max_size() {
        let mut response = api::AttachDocumentResponse::default();
        response.change_pack = Some(
            converter::to_change_pack(&Document::new("doc-key").create_change_pack()).unwrap(),
        );
        response.max_size_per_document = -1;

        assert_eq!(
            ProtocolError::InvalidInteger {
                field: "attach_document_response.max_size_per_document",
                value: -1,
            },
            from_attach_document_response(&response).unwrap_err()
        );
    }
}
