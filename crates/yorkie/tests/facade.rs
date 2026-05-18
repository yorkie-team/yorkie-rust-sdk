use yorkie::{
    ActorId, AttachChannelOptions, AttachOptions, ChangePack, Checkpoint, Client, ClientCondition,
    ClientStatus, CounterType, CounterValue, DeactivateOptions, DetachOptions, Document,
    JsonCounter, Result, SyncMode, TimeTicket, TimeTicketStruct, VersionVector,
};

#[test]
fn facade_exports_document_api() -> Result<()> {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("title", "hello")?;
        Ok(())
    })?;

    assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
    assert_eq!(Checkpoint::initial(), doc.checkpoint());

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
    let client = Client::default();
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
    assert_eq!(Some(SyncMode::Polling), attach_options.sync_mode);
    assert_eq!(Some(SyncMode::Realtime), channel_options.sync_mode);
    assert_eq!(
        DeactivateOptions::default(),
        DeactivateOptions {
            keepalive: false,
            synchronous: false,
        }
    );
    assert_eq!(DetachOptions, DetachOptions);
}
