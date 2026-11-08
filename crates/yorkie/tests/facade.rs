use async_trait::async_trait;
use futures_util::stream;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use yorkie::{
    ActivateClientRequest, ActivateClientResponse, ActorId, AttachChannelOptions,
    AttachDocumentRequest, AttachDocumentResponse, AttachOptions, ChangePack, Checkpoint, Client,
    ClientCondition, ClientError, ClientStatus, ClientTransport, CounterType, CounterValue,
    DeactivateClientRequest, DeactivateClientResponse, DeactivateOptions, DetachDocumentRequest,
    DetachDocumentResponse, DetachOptions, DocStatus, Document, JsonCounter, PeekChannelRequest,
    PeekChannelResponse, Presence, PushPullChangesRequest, PushPullChangesResponse,
    RefreshChannelRequest, RefreshChannelResponse, RemoveDocumentRequest, RemoveDocumentResponse,
    Result, SchemaRule, SharedDocument, SyncMode, TimeTicket, TimeTicketStruct, TreeNodeRule,
    VersionVector, WatchDocumentRequest, WatchStream,
};

#[derive(Debug, Default)]
struct FacadeTransport {
    activate_requests: AtomicUsize,
    attach_requests: AtomicUsize,
    detach_requests: AtomicUsize,
    remove_requests: AtomicUsize,
    push_pull_requests: AtomicUsize,
}

#[async_trait]
impl ClientTransport for FacadeTransport {
    async fn activate_client(
        &self,
        _request: ActivateClientRequest,
    ) -> yorkie::ClientResult<ActivateClientResponse> {
        self.activate_requests.fetch_add(1, Ordering::SeqCst);
        Ok(ActivateClientResponse {
            client_id: ActorId::new("000000000000000000000001"),
            actor_id: None,
        })
    }

    async fn deactivate_client(
        &self,
        _request: DeactivateClientRequest,
    ) -> yorkie::ClientResult<DeactivateClientResponse> {
        Ok(DeactivateClientResponse)
    }

    async fn attach_document(
        &self,
        request: AttachDocumentRequest,
    ) -> yorkie::ClientResult<AttachDocumentResponse> {
        self.attach_requests.fetch_add(1, Ordering::SeqCst);
        Ok(AttachDocumentResponse {
            document_id: "document-id".to_owned(),
            change_pack: request.change_pack,
            max_size_per_document: 1024,
            // Updates are checked against these rules: the test sets a
            // string title. Tree node rules apply to `yorkie.Tree` rules only.
            schema_rules: vec![SchemaRule::new(
                "$.title",
                "string",
                vec![TreeNodeRule::new("paragraph", "text*", "bold", "block")],
            )],
        })
    }

    async fn detach_document(
        &self,
        request: DetachDocumentRequest,
    ) -> yorkie::ClientResult<DetachDocumentResponse> {
        self.detach_requests.fetch_add(1, Ordering::SeqCst);
        Ok(DetachDocumentResponse {
            change_pack: request.change_pack,
        })
    }

    async fn remove_document(
        &self,
        request: RemoveDocumentRequest,
    ) -> yorkie::ClientResult<RemoveDocumentResponse> {
        self.remove_requests.fetch_add(1, Ordering::SeqCst);
        Ok(RemoveDocumentResponse {
            change_pack: request.change_pack,
        })
    }

    async fn push_pull_changes(
        &self,
        request: PushPullChangesRequest,
    ) -> yorkie::ClientResult<PushPullChangesResponse> {
        self.push_pull_requests.fetch_add(1, Ordering::SeqCst);
        Ok(PushPullChangesResponse {
            change_pack: request.change_pack,
        })
    }

    async fn watch_document(
        &self,
        _request: WatchDocumentRequest,
    ) -> yorkie::ClientResult<WatchStream> {
        Ok(Box::pin(stream::pending()))
    }

    async fn refresh_channel(
        &self,
        _request: RefreshChannelRequest,
    ) -> yorkie::ClientResult<RefreshChannelResponse> {
        Err(ClientError::Transport(
            "channels are not used here".to_owned(),
        ))
    }

    async fn peek_channel(
        &self,
        _request: PeekChannelRequest,
    ) -> yorkie::ClientResult<PeekChannelResponse> {
        Err(ClientError::Transport(
            "channels are not used here".to_owned(),
        ))
    }
}

/// Runs one client call on a fresh runtime; the loops it starts stop when
/// the runtime is dropped.
fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn facade_exports_document_api() -> Result<()> {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("title", "hello")?;
        Ok(())
    })?;

    assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
    assert_eq!(Checkpoint::initial(), doc.checkpoint());
    assert_eq!(DocStatus::Detached, doc.status());

    let pack: ChangePack = doc.create_change_pack();
    assert_eq!("test-doc", pack.document_key());
    assert_eq!(Checkpoint::new(0, 1), pack.checkpoint());

    Ok(())
}

#[test]
fn facade_exports_counter_api() -> Result<()> {
    let mut counter = JsonCounter::integer(1);
    counter.increase(2i32)?;

    assert_eq!(CounterType::Integer, counter.value_type());
    assert_eq!(CounterValue::Integer(3), counter.value());
    Ok(())
}

#[test]
fn facade_exports_time_api() {
    let actor_id = ActorId::new("000000000000000000000001");
    let ticket = TimeTicket::new(1, 0, actor_id.clone());
    let ticket_struct = TimeTicketStruct {
        lamport: "1".to_owned(),
        delimiter: 0,
        actor_id: actor_id.clone(),
    };
    let mut vector = VersionVector::new();
    vector.set(actor_id, 1);

    assert_eq!("1:000000000000000000000001:0", ticket.to_id_string());
    assert_eq!(ticket, TimeTicket::from_struct(ticket_struct).unwrap());
    assert!(vector.after_or_equal(&ticket));
}

#[test]
fn facade_exports_client_api() {
    let transport = Arc::new(FacadeTransport::default());
    let client = Client::with_transport(Default::default(), transport.clone());
    let doc: SharedDocument = Arc::new(Mutex::new(Document::new("doc-key")));
    let attach_options = AttachOptions {
        sync_mode: SyncMode::Polling,
        initial_presence: Presence::new(),
        ..AttachOptions::default()
    };
    let channel_options = AttachChannelOptions {
        sync_mode: Some(SyncMode::Realtime),
        ..AttachChannelOptions::default()
    };

    assert!(!client.key().is_empty());
    assert_eq!(ClientStatus::Deactivated, client.status());
    assert!(!client.condition(ClientCondition::SyncLoop));
    assert_eq!(
        ClientError::ClientNotActivated(client.key().to_owned()),
        block_on(client.attach(doc.clone(), attach_options.clone())).unwrap_err()
    );
    block_on(client.activate()).unwrap();
    assert_eq!(ClientStatus::Activated, client.status());
    assert_eq!(1, transport.activate_requests.load(Ordering::SeqCst));
    assert_eq!(SyncMode::Polling, attach_options.sync_mode);
    assert_eq!(Some(SyncMode::Realtime), channel_options.sync_mode);
    block_on(client.attach(doc.clone(), attach_options)).unwrap();
    assert!(client.has("doc-key"));
    assert_eq!(1024, doc.lock().unwrap().max_size_per_document());
    assert_eq!(1, doc.lock().unwrap().schema_rules().len());
    assert_eq!(1, transport.attach_requests.load(Ordering::SeqCst));
    doc.lock()
        .unwrap()
        .update(|root| root.set("title", "hello").map(|_| ()))
        .unwrap();
    block_on(client.sync(Some(&doc))).unwrap();
    assert!(!doc.lock().unwrap().has_local_changes());
    assert_eq!(1, transport.push_pull_requests.load(Ordering::SeqCst));
    block_on(client.remove(&doc)).unwrap();
    assert!(!client.has("doc-key"));
    assert_eq!(DocStatus::Removed, doc.lock().unwrap().status());
    assert_eq!(1, transport.remove_requests.load(Ordering::SeqCst));
    assert_eq!(
        DeactivateOptions::default(),
        DeactivateOptions {
            keepalive: false,
            synchronous: false,
        }
    );
    assert_eq!(DetachOptions, DetachOptions);
    assert_eq!(0, transport.detach_requests.load(Ordering::SeqCst));
}
