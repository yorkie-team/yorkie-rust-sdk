use yorkie::{
    ActorId, ChangePack, Checkpoint, Document, Result, TimeTicket, TimeTicketStruct, VersionVector,
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
