use std::cmp::Ordering;
use yorkie_core::{
    ActorId, TimeTicket, VersionVector, INITIAL_ACTOR_ID, INITIAL_DELIMITER, INITIAL_LAMPORT,
    MAX_ACTOR_ID, MAX_DELIMITER, MAX_LAMPORT, TIME_TICKET_SIZE,
};

#[test]
fn exposes_actor_id_constants() {
    assert_eq!("000000000000000000000000", INITIAL_ACTOR_ID);
    assert_eq!("FFFFFFFFFFFFFFFFFFFFFFFF", MAX_ACTOR_ID);
    assert_eq!(INITIAL_ACTOR_ID, ActorId::initial().as_str());
    assert_eq!(MAX_ACTOR_ID, ActorId::max().as_str());
}

#[test]
fn creates_time_ticket_identifiers() {
    let ticket = TimeTicket::new(7, 2, "000000000000000000000034");

    assert_eq!("7:000000000000000000000034:2", ticket.to_id_string());
    assert_eq!("7:34:2", ticket.to_test_string());
    assert_eq!("7", ticket.lamport_as_string());
    assert_eq!(7, ticket.lamport());
    assert_eq!(2, ticket.delimiter());
    assert_eq!("000000000000000000000034", ticket.actor_id().as_str());
}

#[test]
fn converts_time_ticket_to_and_from_struct() -> yorkie_core::Result<()> {
    let ticket = TimeTicket::new(7, 2, "000000000000000000000034");

    let ticket_struct = ticket.to_struct();
    assert_eq!("7", ticket_struct.lamport);
    assert_eq!(2, ticket_struct.delimiter);
    assert_eq!("000000000000000000000034", ticket_struct.actor_id.as_str());
    assert_eq!(ticket, TimeTicket::from_struct(ticket_struct)?);

    Ok(())
}

#[test]
fn exposes_initial_and_max_time_tickets() {
    assert_eq!(8 + 4 + 12, TIME_TICKET_SIZE);
    assert_eq!(0, INITIAL_LAMPORT);
    assert_eq!(0, INITIAL_DELIMITER);
    assert_eq!(u32::MAX, MAX_DELIMITER);
    assert_eq!(i64::MAX, MAX_LAMPORT);

    let initial = TimeTicket::initial();
    assert_eq!("0:000000000000000000000000:0", initial.to_id_string());

    let max = TimeTicket::max();
    assert_eq!(i64::MAX, max.lamport());
    assert_eq!(u32::MAX, max.delimiter());
    assert_eq!(MAX_ACTOR_ID, max.actor_id().as_str());
}

#[test]
fn orders_time_tickets_by_lamport_actor_and_delimiter() {
    let low = TimeTicket::new(1, 9, "b");
    let higher_lamport = TimeTicket::new(2, 0, "a");
    let higher_actor = TimeTicket::new(1, 0, "c");
    let higher_delimiter = TimeTicket::new(1, 10, "b");

    assert_eq!(Ordering::Less, low.cmp(&higher_lamport));
    assert_eq!(Ordering::Less, low.cmp(&higher_actor));
    assert_eq!(Ordering::Less, low.cmp(&higher_delimiter));
    assert!(higher_lamport.after(&low));
}

#[test]
fn sets_actor_without_mutating_time_ticket() {
    let ticket = TimeTicket::new(7, 2, "a");
    let changed = ticket.set_actor("b");

    assert_eq!("a", ticket.actor_id().as_str());
    assert_eq!("b", changed.actor_id().as_str());
    assert_eq!(ticket.lamport(), changed.lamport());
    assert_eq!(ticket.delimiter(), changed.delimiter());
}

#[test]
fn tracks_version_vector_versions() {
    let mut vector = VersionVector::new();
    vector.set("a", 1);
    vector.set("b", 3);

    assert_eq!(2, vector.size());
    assert!(vector.has("a"));
    assert_eq!(Some(3), vector.get("b"));
    assert_eq!(3, vector.max_lamport());
    assert!(vector.after_or_equal(&TimeTicket::new(3, 0, "b")));
    assert!(!vector.after_or_equal(&TimeTicket::new(4, 0, "b")));
    assert!(!vector.after_or_equal(&TimeTicket::new(1, 0, "c")));

    vector.unset("a");
    assert!(!vector.has("a"));
}

#[test]
fn merges_and_filters_version_vectors() {
    let mut left = VersionVector::new();
    left.set("a", 1);
    left.set("b", 4);

    let mut right = VersionVector::new();
    right.set("a", 3);
    right.set("c", 2);

    let merged = left.max(&right);
    assert_eq!(Some(3), merged.get("a"));
    assert_eq!(Some(4), merged.get("b"));
    assert_eq!(Some(2), merged.get("c"));

    let filtered = merged.filter(&right);
    assert_eq!(2, filtered.size());
    assert_eq!(Some(3), filtered.get("a"));
    assert_eq!(Some(2), filtered.get("c"));
    assert_eq!(None, filtered.get("b"));
}
