//! Protobuf conversion helpers for client transport messages.

use crate::transport::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    BroadcastRequest, BroadcastResponse, DeactivateClientRequest, DeactivateClientResponse,
    DetachDocumentRequest, DetachDocumentResponse, PeekChannelRequest, PeekChannelResponse,
    PushPullChangesRequest, PushPullChangesResponse, RefreshChannelRequest, RefreshChannelResponse,
    RemoveDocumentRequest, RemoveDocumentResponse, WatchChannelEvent, WatchChannelEventType,
    WatchChannelRequest, WatchChannelResponse, WatchDocEvent, WatchDocEventType,
    WatchDocumentRequest, WatchDocumentResponse,
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
        actor_id: (!response.actor_id.is_empty()).then(|| ActorId::new(response.actor_id.clone())),
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
        disable_gc: request.disable_gc,
        ..Default::default()
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
        disable_gc: request.disable_gc,
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

pub fn to_refresh_channel_request(request: &RefreshChannelRequest) -> api::RefreshChannelRequest {
    api::RefreshChannelRequest {
        client_id: request
            .client_id
            .as_ref()
            .map(ActorId::to_string)
            .unwrap_or_default(),
        channel_key: request.channel_key.clone(),
        session_id: request.session_id.clone().unwrap_or_default(),
        client_key: request.client_key.clone(),
        metadata: request.metadata.clone().into_iter().collect(),
    }
}

pub fn from_refresh_channel_response(
    response: &api::RefreshChannelResponse,
) -> RefreshChannelResponse {
    RefreshChannelResponse {
        session_count: count_from_i64(response.session_count),
        client_id: (!response.client_id.is_empty())
            .then(|| ActorId::new(response.client_id.clone())),
        session_id: (!response.session_id.is_empty()).then(|| response.session_id.clone()),
    }
}

pub fn to_peek_channel_request(request: &PeekChannelRequest) -> api::PeekChannelRequest {
    api::PeekChannelRequest {
        channel_key: request.channel_key.clone(),
    }
}

pub fn from_peek_channel_response(response: &api::PeekChannelResponse) -> PeekChannelResponse {
    PeekChannelResponse {
        session_count: count_from_i64(response.session_count),
    }
}

pub fn to_broadcast_request(request: &BroadcastRequest) -> api::BroadcastRequest {
    api::BroadcastRequest {
        client_id: request.client_id.to_string(),
        channel_key: request.channel_key.clone(),
        topic: request.topic.clone(),
        payload: request.payload.clone(),
    }
}

pub fn from_broadcast_response(_response: &api::BroadcastResponse) -> BroadcastResponse {
    BroadcastResponse
}

/// Builds the unified `Watch` request for one channel, as JS
/// `createChannelWatchStream` does: no actor, which channels do not use.
pub fn to_watch_channel_request(request: &WatchChannelRequest) -> api::WatchRequest {
    api::WatchRequest {
        client_id: request.client_id.to_string(),
        resources: vec![api::ResourceDescriptor {
            resource: Some(api::resource_descriptor::Resource::Channel(
                api::ChannelDescriptor {
                    channel_key: request.channel_key.clone(),
                },
            )),
        }],
        actor_id: String::new(),
        heartbeat_interval_ms: i64::try_from(request.heartbeat_interval_ms).unwrap_or(i64::MAX),
    }
}

/// Converts a `Watch` response for a channel stream (JS
/// `handleWatchChannelResponse`). Bodies for other resources return `None`
/// and are skipped.
pub fn from_watch_channel_response(response: &api::WatchResponse) -> Option<WatchChannelResponse> {
    use api::watch_response::Body;

    match response.body.as_ref()? {
        Body::Initialization(init) => {
            let channel_init =
                init.resource_inits
                    .iter()
                    .find_map(|resource_init| match &resource_init.init {
                        Some(api::resource_init::Init::ChannelInit(init)) => Some(init),
                        _ => None,
                    })?;
            Some(WatchChannelResponse::Initialization {
                session_count: count_from_i64(channel_init.session_count),
                seq: count_from_i64(channel_init.seq),
                heartbeat_interval_ms: u64::try_from(init.heartbeat_interval_ms).unwrap_or(0),
            })
        }
        Body::Event(event) => {
            let Some(api::watch_event::Event::ChannelEvent(channel_event)) = &event.event else {
                return None;
            };
            let event = channel_event.event.as_ref()?;
            let event_type = if event.r#type == api::channel_event::Type::Broadcast as i32 {
                WatchChannelEventType::Broadcast
            } else {
                WatchChannelEventType::Presence
            };
            Some(WatchChannelResponse::Event(WatchChannelEvent {
                event_type,
                publisher: ActorId::new(event.publisher.clone()),
                session_count: count_from_i64(event.session_count),
                seq: count_from_i64(event.seq),
                topic: event.topic.clone(),
                payload: event.payload.clone(),
            }))
        }
        Body::Heartbeat(_) => Some(WatchChannelResponse::Heartbeat),
    }
}

/// Builds the unified `Watch` request for one document, as JS
/// `createDocumentWatchStream` does.
pub fn to_watch_document_request(request: &WatchDocumentRequest) -> api::WatchRequest {
    api::WatchRequest {
        client_id: request.client_id.to_string(),
        resources: vec![api::ResourceDescriptor {
            resource: Some(api::resource_descriptor::Resource::Document(
                api::DocumentDescriptor {
                    document_id: request.document_id.clone(),
                },
            )),
        }],
        actor_id: request.actor_id.to_string(),
        heartbeat_interval_ms: i64::try_from(request.heartbeat_interval_ms).unwrap_or(i64::MAX),
    }
}

/// Converts a `Watch` response for a document stream. Bodies this client does
/// not know (an empty body, a channel event, an unknown event type) return
/// `None` and are skipped, so a newer server does not break the stream.
pub fn from_watch_response(response: &api::WatchResponse) -> Option<WatchDocumentResponse> {
    use api::watch_response::Body;

    match response.body.as_ref()? {
        Body::Initialization(init) => Some(WatchDocumentResponse::Initialization {
            client_ids: init
                .resource_inits
                .iter()
                .filter_map(|resource_init| match &resource_init.init {
                    Some(api::resource_init::Init::DocumentInit(init)) => Some(&init.client_ids),
                    _ => None,
                })
                .flatten()
                .map(|client_id| ActorId::new(client_id.clone()))
                .collect(),
            heartbeat_interval_ms: u64::try_from(init.heartbeat_interval_ms).unwrap_or(0),
        }),
        Body::Event(event) => {
            let Some(api::watch_event::Event::DocEvent(doc_event)) = &event.event else {
                return None;
            };
            let event = doc_event.event.as_ref()?;
            let event_type = match api::DocEventType::try_from(event.r#type).ok()? {
                api::DocEventType::DocumentChanged => WatchDocEventType::DocumentChanged,
                api::DocEventType::DocumentWatched => WatchDocEventType::DocumentWatched,
                api::DocEventType::DocumentUnwatched => WatchDocEventType::DocumentUnwatched,
                api::DocEventType::DocumentBroadcast => WatchDocEventType::DocumentBroadcast,
            };
            Some(WatchDocumentResponse::Event(WatchDocEvent {
                event_type,
                publisher: ActorId::new(event.publisher.clone()),
            }))
        }
        Body::Heartbeat(_) => Some(WatchDocumentResponse::Heartbeat),
    }
}

/// Session counts and sequence numbers are `int64` on the wire; a negative
/// one is never sent, and reads as 0.
fn count_from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
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
            ..Default::default()
        });
        assert_eq!("000000000000000000000001", response.client_id.as_str());
        assert_eq!(None, response.actor_id);

        let response = from_activate_client_response(&api::ActivateClientResponse {
            client_id: "000000000000000000000001".to_owned(),
            actor_id: "000000000000000000000002".to_owned(),
        });
        assert_eq!("000000000000000000000001", response.client_id.as_str());
        assert_eq!(
            Some("000000000000000000000002"),
            response.actor_id.as_ref().map(ActorId::as_str)
        );
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
            disable_gc: false,
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
            ..Default::default()
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
            disable_gc: false,
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

    /// JS `attachDocument` (`client.ts` 1588) and `syncInternal` (4025-4037)
    /// send the attachment's `disableGC` as the request's `disableGc`.
    #[test]
    fn encodes_disable_gc_on_attach_and_push_pull() -> Result<()> {
        let doc = Document::new("doc-key");
        for disable_gc in [true, false] {
            let attach = to_attach_document_request(&AttachDocumentRequest {
                client_id: ActorId::new("000000000000000000000001"),
                change_pack: doc.create_change_pack(),
                schema_key: None,
                disable_gc,
                shard_key: "api-key/doc-key".to_owned(),
            })?;
            assert_eq!(disable_gc, attach.disable_gc);

            let push_pull = to_push_pull_changes_request(&PushPullChangesRequest {
                client_id: ActorId::new("000000000000000000000001"),
                document_id: "document-id".to_owned(),
                change_pack: doc.create_change_pack(),
                push_only: false,
                disable_gc,
                shard_key: "api-key/doc-key".to_owned(),
            })?;
            assert_eq!(disable_gc, push_pull.disable_gc);
        }
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
        let response = api::AttachDocumentResponse {
            change_pack: Some(
                converter::to_change_pack(&Document::new("doc-key").create_change_pack()).unwrap(),
            ),
            max_size_per_document: -1,
            ..Default::default()
        };

        assert_eq!(
            ProtocolError::InvalidInteger {
                field: "attach_document_response.max_size_per_document",
                value: -1,
            },
            from_attach_document_response(&response).unwrap_err()
        );
    }

    /// JS `syncInternal` for a channel and `peekChannel`.
    #[test]
    fn converts_channel_session_messages() {
        let first = to_refresh_channel_request(&RefreshChannelRequest {
            client_id: None,
            channel_key: "room.1".to_owned(),
            session_id: None,
            client_key: "client-key".to_owned(),
            metadata: BTreeMap::from([("region".to_owned(), "local".to_owned())]),
            shard_key: "api-key/room".to_owned(),
        });
        assert_eq!("", first.client_id);
        assert_eq!("room.1", first.channel_key);
        assert_eq!("", first.session_id);
        assert_eq!("client-key", first.client_key);
        assert_eq!(
            Some("local"),
            first.metadata.get("region").map(String::as_str)
        );

        let heartbeat = to_refresh_channel_request(&RefreshChannelRequest {
            client_id: Some(ActorId::new("000000000000000000000001")),
            channel_key: "room.1".to_owned(),
            session_id: Some("session".to_owned()),
            client_key: String::new(),
            metadata: BTreeMap::new(),
            shard_key: "api-key/room".to_owned(),
        });
        assert_eq!("000000000000000000000001", heartbeat.client_id);
        assert_eq!("session", heartbeat.session_id);

        assert_eq!(
            RefreshChannelResponse {
                session_count: 2,
                client_id: Some(ActorId::new("000000000000000000000001")),
                session_id: Some("session".to_owned()),
            },
            from_refresh_channel_response(&api::RefreshChannelResponse {
                session_count: 2,
                client_id: "000000000000000000000001".to_owned(),
                session_id: "session".to_owned(),
            })
        );
        assert_eq!(
            RefreshChannelResponse {
                session_count: 0,
                client_id: None,
                session_id: None,
            },
            from_refresh_channel_response(&api::RefreshChannelResponse {
                session_count: -1,
                ..Default::default()
            })
        );

        assert_eq!(
            "room.1",
            to_peek_channel_request(&PeekChannelRequest {
                channel_key: "room.1".to_owned(),
                shard_key: "api-key/room".to_owned(),
            })
            .channel_key
        );
        assert_eq!(
            PeekChannelResponse { session_count: 3 },
            from_peek_channel_response(&api::PeekChannelResponse { session_count: 3 })
        );
    }

    /// JS `createChannelWatchStream`, `handleWatchChannelResponse`, and the
    /// `broadcast` request.
    #[test]
    fn converts_channel_watch_and_broadcast_messages() {
        let request = to_watch_channel_request(&WatchChannelRequest {
            client_id: ActorId::new("000000000000000000000001"),
            channel_key: "room.1".to_owned(),
            heartbeat_interval_ms: 20000,
            shard_key: "api-key/room".to_owned(),
        });
        assert_eq!("000000000000000000000001", request.client_id);
        assert_eq!("", request.actor_id);
        assert_eq!(20000, request.heartbeat_interval_ms);
        assert_eq!(
            vec![api::ResourceDescriptor {
                resource: Some(api::resource_descriptor::Resource::Channel(
                    api::ChannelDescriptor {
                        channel_key: "room.1".to_owned(),
                    }
                )),
            }],
            request.resources
        );

        let init = api::WatchResponse {
            body: Some(api::watch_response::Body::Initialization(
                api::WatchInitialization {
                    resource_inits: vec![api::ResourceInit {
                        init: Some(api::resource_init::Init::ChannelInit(api::ChannelInit {
                            channel_key: "room.1".to_owned(),
                            session_count: 2,
                            seq: 7,
                        })),
                    }],
                    heartbeat_interval_ms: 15000,
                },
            )),
        };
        assert_eq!(
            Some(WatchChannelResponse::Initialization {
                session_count: 2,
                seq: 7,
                heartbeat_interval_ms: 15000,
            }),
            from_watch_channel_response(&init)
        );

        let event = |r#type: i32| api::WatchResponse {
            body: Some(api::watch_response::Body::Event(api::WatchEvent {
                event: Some(api::watch_event::Event::ChannelEvent(
                    api::ChannelWatchEvent {
                        channel_key: "room.1".to_owned(),
                        event: Some(api::ChannelEvent {
                            r#type,
                            publisher: "000000000000000000000002".to_owned(),
                            session_count: 3,
                            seq: 8,
                            topic: "chat".to_owned(),
                            payload: b"{}".to_vec(),
                        }),
                    },
                )),
            })),
        };
        let expected = |event_type| {
            Some(WatchChannelResponse::Event(WatchChannelEvent {
                event_type,
                publisher: ActorId::new("000000000000000000000002"),
                session_count: 3,
                seq: 8,
                topic: "chat".to_owned(),
                payload: b"{}".to_vec(),
            }))
        };
        assert_eq!(
            expected(WatchChannelEventType::Broadcast),
            from_watch_channel_response(&event(api::channel_event::Type::Broadcast as i32))
        );
        // JS treats every type but BROADCAST as a count change.
        for r#type in [
            api::channel_event::Type::Presence as i32,
            api::channel_event::Type::Unspecified as i32,
        ] {
            assert_eq!(
                expected(WatchChannelEventType::Presence),
                from_watch_channel_response(&event(r#type))
            );
        }
        let document_event = api::WatchResponse {
            body: Some(api::watch_response::Body::Event(api::WatchEvent {
                event: Some(api::watch_event::Event::DocEvent(
                    api::DocWatchEvent::default(),
                )),
            })),
        };
        assert_eq!(None, from_watch_channel_response(&document_event));

        let broadcast = to_broadcast_request(&BroadcastRequest {
            client_id: ActorId::new("000000000000000000000001"),
            channel_key: "room.1".to_owned(),
            topic: "chat".to_owned(),
            payload: br#"{"a":1}"#.to_vec(),
            shard_key: "api-key/room".to_owned(),
        });
        assert_eq!("000000000000000000000001", broadcast.client_id);
        assert_eq!("room.1", broadcast.channel_key);
        assert_eq!("chat", broadcast.topic);
        assert_eq!(br#"{"a":1}"#.to_vec(), broadcast.payload);
    }

    /// JS `createDocumentWatchStream` and `handleWatchDocumentResponse`.
    #[test]
    fn converts_watch_messages() {
        let request = to_watch_document_request(&WatchDocumentRequest {
            client_id: ActorId::new("000000000000000000000001"),
            actor_id: ActorId::new("000000000000000000000002"),
            document_id: "document-id".to_owned(),
            heartbeat_interval_ms: 20000,
            shard_key: "api-key/doc-key".to_owned(),
        });
        assert_eq!("000000000000000000000001", request.client_id);
        assert_eq!("000000000000000000000002", request.actor_id);
        assert_eq!(20000, request.heartbeat_interval_ms);
        assert_eq!(
            vec![api::ResourceDescriptor {
                resource: Some(api::resource_descriptor::Resource::Document(
                    api::DocumentDescriptor {
                        document_id: "document-id".to_owned(),
                    }
                )),
            }],
            request.resources
        );

        let init = api::WatchResponse {
            body: Some(api::watch_response::Body::Initialization(
                api::WatchInitialization {
                    resource_inits: vec![api::ResourceInit {
                        init: Some(api::resource_init::Init::DocumentInit(api::DocumentInit {
                            document_id: "document-id".to_owned(),
                            client_ids: vec!["000000000000000000000003".to_owned()],
                        })),
                    }],
                    heartbeat_interval_ms: 15000,
                },
            )),
        };
        assert_eq!(
            Some(WatchDocumentResponse::Initialization {
                client_ids: vec![ActorId::new("000000000000000000000003")],
                heartbeat_interval_ms: 15000,
            }),
            from_watch_response(&init)
        );

        let event = |r#type: i32| api::WatchResponse {
            body: Some(api::watch_response::Body::Event(api::WatchEvent {
                event: Some(api::watch_event::Event::DocEvent(api::DocWatchEvent {
                    document_id: "document-id".to_owned(),
                    event: Some(api::DocEvent {
                        r#type,
                        publisher: "000000000000000000000003".to_owned(),
                        body: None,
                    }),
                })),
            })),
        };
        assert_eq!(
            Some(WatchDocumentResponse::Event(WatchDocEvent {
                event_type: WatchDocEventType::DocumentWatched,
                publisher: ActorId::new("000000000000000000000003"),
            })),
            from_watch_response(&event(api::DocEventType::DocumentWatched as i32))
        );
        assert_eq!(None, from_watch_response(&event(99)));

        let heartbeat = api::WatchResponse {
            body: Some(api::watch_response::Body::Heartbeat(api::WatchHeartbeat {})),
        };
        assert_eq!(
            Some(WatchDocumentResponse::Heartbeat),
            from_watch_response(&heartbeat)
        );
        assert_eq!(None, from_watch_response(&api::WatchResponse::default()));
    }
}
