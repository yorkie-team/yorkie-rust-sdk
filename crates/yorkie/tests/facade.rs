use yorkie::{
    ActivateClientRequest, ActivateClientResponse, ActorId, AttachChannelOptions,
    AttachDocumentRequest, AttachDocumentResponse, AttachOptions, ChangePack, Checkpoint, Client,
    ClientCondition, ClientError, ClientStatus, ClientTransport, CounterType, CounterValue,
    DeactivateClientRequest, DeactivateClientResponse, DeactivateOptions, DetachDocumentRequest,
    DetachDocumentResponse, DetachOptions, DocStatus, Document, JsonCounter,
    PushPullChangesRequest, PushPullChangesResponse, Result, SchemaRule, SyncMode, SyncOptions,
    TimeTicket, TimeTicketStruct, TreeNodeRule, VersionVector,
};

#[derive(Debug, Default)]
struct FacadeTransport {
    activate_requests: usize,
    attach_requests: usize,
    detach_requests: usize,
    push_pull_requests: usize,
}

impl ClientTransport for FacadeTransport {
    fn activate_client(
        &mut self,
        _request: ActivateClientRequest,
    ) -> yorkie::ClientResult<ActivateClientResponse> {
        self.activate_requests += 1;
        Ok(ActivateClientResponse {
            client_id: ActorId::new("000000000000000000000001"),
        })
    }

    fn deactivate_client(
        &mut self,
        _request: DeactivateClientRequest,
    ) -> yorkie::ClientResult<DeactivateClientResponse> {
        Ok(DeactivateClientResponse)
    }

    fn attach_document(
        &mut self,
        request: AttachDocumentRequest,
    ) -> yorkie::ClientResult<AttachDocumentResponse> {
        self.attach_requests += 1;
        Ok(AttachDocumentResponse {
            document_id: "document-id".to_owned(),
            change_pack: request.change_pack,
            max_size_per_document: 1024,
            schema_rules: vec![SchemaRule::new(
                "$.profile",
                "object",
                vec![TreeNodeRule::new("paragraph", "text*", "bold", "block")],
            )],
        })
    }

    fn detach_document(
        &mut self,
        request: DetachDocumentRequest,
    ) -> yorkie::ClientResult<DetachDocumentResponse> {
        self.detach_requests += 1;
        Ok(DetachDocumentResponse {
            change_pack: request.change_pack,
        })
    }

    fn push_pull_changes(
        &mut self,
        request: PushPullChangesRequest,
    ) -> yorkie::ClientResult<PushPullChangesResponse> {
        self.push_pull_requests += 1;
        Ok(PushPullChangesResponse {
            change_pack: request.change_pack,
        })
    }
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
    let mut client = Client::default();
    let mut transport = FacadeTransport::default();
    let mut doc = Document::new("doc-key");
    let attach_options = AttachOptions {
        sync_mode: Some(SyncMode::Polling),
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
        client
            .attach(&mut transport, &mut doc, attach_options.clone())
            .unwrap_err()
    );
    client.activate(&mut transport).unwrap();
    assert_eq!(ClientStatus::Activated, client.status());
    assert_eq!(1, transport.activate_requests);
    assert_eq!(Some(SyncMode::Polling), attach_options.sync_mode);
    assert_eq!(Some(SyncMode::Realtime), channel_options.sync_mode);
    client
        .attach(&mut transport, &mut doc, attach_options)
        .unwrap();
    assert!(client.has("doc-key"));
    assert_eq!(1024, doc.max_size_per_document());
    assert_eq!(1, doc.schema_rules().len());
    assert_eq!(1, transport.attach_requests);
    doc.update(|root| root.set("title", "hello").map(|_| ()))
        .unwrap();
    client.sync(&mut transport, &mut doc).unwrap();
    assert!(!doc.has_local_changes());
    assert_eq!(1, transport.push_pull_requests);
    assert_eq!(SyncOptions::default(), SyncOptions { sync_mode: None });
    client
        .detach(&mut transport, &mut doc, DetachOptions)
        .unwrap();
    assert!(!client.has("doc-key"));
    assert_eq!(1, transport.detach_requests);
    assert_eq!(
        DeactivateOptions::default(),
        DeactivateOptions {
            keepalive: false,
            synchronous: false,
        }
    );
    assert_eq!(DetachOptions, DetachOptions);
}
