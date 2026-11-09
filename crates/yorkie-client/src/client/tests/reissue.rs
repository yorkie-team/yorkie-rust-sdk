//! Tests of the pre-attach ticket re-issue in `Client::attach`, ported from
//! JS `test/unit/client/reissue_claim_test.ts` ("Client.attach re-issues
//! pre-attach tickets", `ed25355e`) and cross-checked with Go
//! `client/reissue_claim_internal_test.go` (`TestClaimReissue`, `1061bc35`).
//!
//! Not ported: "refuses to generate a key when it cannot be unguessable".
//! `generate_client_key` draws from the OS random source and has no weak
//! fallback to refuse.

use super::{block_on, keyed_options, mock_client, MockTransport};
use crate::attachment::SharedDocument;
use crate::error::{ClientError, ClientResult};
use crate::options::{AttachOptions, ClientOptions, DeactivateOptions, DetachOptions, SyncMode};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yorkie_core::{ActorId, ChangePack, Document, INITIAL_ACTOR_ID};
use yorkie_protocol::converter;

const ACTOR_A: &str = "0000000000000000000000a1";
const ACTOR_B: &str = "0000000000000000000000b1";

/// A transport whose attach and detach replies only acknowledge the pushed
/// changes (JS `attachResponse`).
fn acking_transport() -> Arc<MockTransport> {
    Arc::new(MockTransport {
        ack_pushes: true,
        ..MockTransport::default()
    })
}

/// JS `fakeClient`: a client forced active under the given actor. Without a
/// key option the client key is generated, as in JS.
fn fake_client(actor: &str, transport: &Arc<MockTransport>, key: Option<&str>) -> super::Client {
    let options = match key {
        Some(key) => ClientOptions {
            key: Some(key.to_owned()),
            ..keyed_options()
        },
        None => ClientOptions::default(),
    };
    let client = mock_client(options, transport);
    client.apply_activation(actor);
    client
}

/// JS `filled`: a detached document of the given key edited before any
/// attach, so every ticket it holds names the initial actor.
fn filled(key: &str, content: &str) -> SharedDocument {
    let mut doc = Document::new(key);
    doc.update(|root| {
        root.set_text("t")?.edit(0, 0, content, None)?;
        Ok(())
    })
    .unwrap();
    Arc::new(Mutex::new(doc))
}

/// The sorted JSON of a document `filled` with `content`, to compare roots
/// with (JS `doc.getRoot().t.toString()`).
fn filled_json(content: &str) -> String {
    filled("expected", content).lock().unwrap().to_sorted_json()
}

fn manual() -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        ..AttachOptions::default()
    }
}

/// The pack the `index`-th attach pushed.
fn pushed(transport: &MockTransport, index: usize) -> ChangePack {
    transport.attach_requests.lock().unwrap()[index]
        .change_pack
        .clone()
}

/// JS `countActors(ticketsOf(pack))`: how many tickets of a pack name each
/// actor, leaving out lamport-0 ones (the root's). It reads the protobuf
/// message's `Debug` output, where each `TimeTicket` prints its lamport and
/// actor bytes. JS `ticketsOf` also decodes nested Object, Array and Tree
/// values; these Text-only documents send none.
fn actors_of(pack: &ChangePack) -> BTreeMap<String, usize> {
    let debug = format!("{:?}", converter::to_change_pack(pack).unwrap());
    let mut actors = BTreeMap::new();
    for ticket in debug.split("TimeTicket { lamport: ").skip(1) {
        let (lamport, rest) = ticket.split_once(',').unwrap();
        let bytes = rest
            .split_once("actor_id: [")
            .and_then(|(_, rest)| rest.split_once(']'))
            .unwrap()
            .0;
        if lamport.parse::<i64>().unwrap() == 0 {
            continue;
        }
        let actor = bytes
            .split(", ")
            .map(|byte| format!("{:02x}", byte.parse::<u8>().unwrap()))
            .collect::<String>();
        *actors.entry(actor).or_default() += 1;
    }
    actors
}

fn count(actors: &BTreeMap<String, usize>, actor: &str) -> usize {
    actors.get(actor).copied().unwrap_or(0)
}

fn lost() -> ClientError {
    ClientError::Rpc {
        code: "unknown".to_owned(),
        message: "lost".to_owned(),
        yorkie_code: None,
        reason: None,
    }
}

fn actor_of(doc: &SharedDocument) -> String {
    doc.lock().unwrap().actor_id().as_str().to_owned()
}

/// JS "pushes the pre-attach changes under the client actor".
#[test]
fn pushes_the_pre_attach_changes_under_the_client_actor() -> ClientResult<()> {
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);
    let doc = filled("reissue-claim-push", "hello");

    block_on(client.attach(doc.clone(), manual()))?;

    let actors = actors_of(&pushed(&transport, 0));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");
    assert!(count(&actors, ACTOR_A) > 0, "{actors:?}");
    assert_eq!(filled_json("hello"), doc.lock().unwrap().to_sorted_json());
    Ok(())
}

/// JS "keeps the re-issued state when the attach fails": the failed attach
/// keeps the re-issued document, and a retry under another client re-issues
/// it from the first actor to its own.
#[test]
fn keeps_the_reissued_state_when_the_attach_fails() -> ClientResult<()> {
    let transport = acking_transport();
    transport.attach_errors.lock().unwrap().push(lost());
    let failing = fake_client(ACTOR_A, &transport, None);
    let doc = filled("reissue-claim-fail", "hello");

    let failed = block_on(failing.attach(doc.clone(), manual()));
    assert_eq!(Some(lost()), failed.err());
    assert_eq!(ACTOR_A, actor_of(&doc));
    let actors = actors_of(&pushed(&transport, 0));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");

    let retried = acking_transport();
    let other = fake_client(ACTOR_B, &retried, None);
    block_on(other.attach(doc.clone(), manual()))?;

    let actors = actors_of(&pushed(&retried, 0));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");
    assert_eq!(0, count(&actors, ACTOR_A), "{actors:?}");
    assert!(count(&actors, ACTOR_B) > 0, "{actors:?}");
    Ok(())
}

/// JS "does not re-issue when the attach options are rejected": the zero
/// poll interval is rejected before the re-issue and the claim, so a valid
/// retry still re-issues.
#[test]
fn does_not_reissue_when_the_attach_options_are_rejected() -> ClientResult<()> {
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);
    let doc = filled("reissue-claim-invalid", "hello");

    let rejected = block_on(client.attach(
        doc.clone(),
        AttachOptions {
            document_poll_interval: Some(Duration::ZERO),
            ..AttachOptions::default()
        },
    ));
    match rejected {
        Err(ClientError::InvalidArgument(message)) => {
            assert!(message.contains("document_poll_interval"), "{message}");
        }
        other => panic!("expected an invalid argument, got {other:?}"),
    }
    assert_eq!(INITIAL_ACTOR_ID, actor_of(&doc));
    assert!(doc.lock().unwrap().history().can_undo());

    block_on(client.attach(doc.clone(), manual()))?;
    let actors = actors_of(&pushed(&transport, 0));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");
    Ok(())
}

/// JS "declines a second never-synced document of a key on one client": the
/// claim is taken before the round trip, so a lost response counts, and the
/// declined document keeps the initial actor in the tickets it minted.
#[test]
fn declines_a_second_never_synced_document_of_a_key_on_one_client() -> ClientResult<()> {
    let key = "reissue-claim-second";
    let transport = acking_transport();
    transport.attach_errors.lock().unwrap().push(lost());
    let client = fake_client(ACTOR_A, &transport, None);

    let first = filled(key, "one");
    assert!(block_on(client.attach(first, manual())).is_err());
    let actors = actors_of(&pushed(&transport, 0));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");

    let second = filled(key, "two");
    block_on(client.attach(second.clone(), manual()))?;

    let actors = actors_of(&pushed(&transport, 1));
    assert!(count(&actors, INITIAL_ACTOR_ID) > 0, "{actors:?}");
    assert_eq!(ACTOR_A, actor_of(&second));
    Ok(())
}

/// JS "re-issues a later document of a key the first left untouched": a
/// plain attach of a document that minted nothing takes no claim.
#[test]
fn reissues_a_later_document_of_a_key_the_first_left_untouched() -> ClientResult<()> {
    let key = "reissue-claim-untouched";
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);

    let first = Arc::new(Mutex::new(Document::new(key)));
    block_on(client.attach(first.clone(), manual()))?;
    block_on(client.detach(&first, DetachOptions))?;

    let second = filled(key, "two");
    block_on(client.attach(second.clone(), manual()))?;

    let actors = actors_of(&pushed(&transport, 1));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");
    assert!(count(&actors, ACTOR_A) > 0, "{actors:?}");
    assert_eq!(filled_json("two"), second.lock().unwrap().to_sorted_json());
    Ok(())
}

/// JS "declines a later document when the first minted while attached": the
/// claim is taken on the way out (JS `detachInternal`).
#[test]
fn declines_a_later_document_when_the_first_minted_while_attached() -> ClientResult<()> {
    let key = "reissue-claim-minted-attached";
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);

    let first = Arc::new(Mutex::new(Document::new(key)));
    block_on(client.attach(first.clone(), manual()))?;
    first.lock().unwrap().update(|root| {
        root.set_text("t")?.edit(0, 0, "one", None)?;
        Ok(())
    })?;
    block_on(client.detach(&first, DetachOptions))?;

    let second = filled(key, "two");
    block_on(client.attach(second.clone(), manual()))?;

    let actors = actors_of(&pushed(&transport, 1));
    assert!(count(&actors, INITIAL_ACTOR_ID) > 0, "{actors:?}");
    assert_eq!(ACTOR_A, actor_of(&second));
    Ok(())
}

/// Like JS "declines a later document when the first minted while attached",
/// with the attachment dropped by `deactivate` (JS `deactivateInternal`
/// runs `detachInternal` for each attachment). The claim outlives the
/// session: the next one has the same stable actor.
#[test]
fn declines_a_later_document_when_the_first_minted_before_deactivate() -> ClientResult<()> {
    let key = "reissue-claim-minted-deactivate";
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);

    let first = Arc::new(Mutex::new(Document::new(key)));
    block_on(client.attach(first.clone(), manual()))?;
    first.lock().unwrap().update(|root| {
        root.set_text("t")?.edit(0, 0, "one", None)?;
        Ok(())
    })?;
    block_on(client.deactivate(DeactivateOptions::default()))?;
    client.apply_activation(ACTOR_A);

    let second = filled(key, "two");
    block_on(client.attach(second.clone(), manual()))?;

    let actors = actors_of(&pushed(&transport, 1));
    assert!(count(&actors, INITIAL_ACTOR_ID) > 0, "{actors:?}");
    assert_eq!(ACTOR_A, actor_of(&second));
    Ok(())
}

/// Covers the `doc.hasMintedOperations()` branch of JS `attach`: a document
/// that cannot re-issue, having synced under another client, still takes the
/// claim at attach when it carries minted operations, since its edits are
/// about to be pushed under this client's actor. JS has no test of it.
#[test]
fn takes_the_claim_at_attach_for_a_minted_document_it_cannot_reissue() -> ClientResult<()> {
    let key = "reissue-claim-minted-elsewhere";
    let elsewhere = acking_transport();
    let other = fake_client(ACTOR_B, &elsewhere, None);
    let first = Arc::new(Mutex::new(Document::new(key)));
    block_on(other.attach(first.clone(), manual()))?;
    first.lock().unwrap().update(|root| {
        root.set_text("t")?.edit(0, 0, "one", None)?;
        Ok(())
    })?;
    block_on(other.detach(&first, DetachOptions))?;

    // The attach fails, so only the claim it took before the round trip
    // can decline the next document.
    let transport = acking_transport();
    transport.attach_errors.lock().unwrap().push(lost());
    let client = fake_client(ACTOR_A, &transport, None);
    assert!(block_on(client.attach(first, manual())).is_err());

    let second = filled(key, "two");
    block_on(client.attach(second, manual()))?;

    let actors = actors_of(&pushed(&transport, 1));
    assert!(count(&actors, INITIAL_ACTOR_ID) > 0, "{actors:?}");
    Ok(())
}

/// JS "re-issues a second never-synced document of another key": the claim
/// is per document key.
#[test]
fn reissues_a_second_never_synced_document_of_another_key() -> ClientResult<()> {
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);

    let first = filled("reissue-claim-key-one", "one");
    block_on(client.attach(first, manual()))?;

    let second = filled("reissue-claim-key-two", "two");
    block_on(client.attach(second.clone(), manual()))?;

    assert_eq!(2, transport.attach_requests.lock().unwrap().len());
    let actors = actors_of(&pushed(&transport, 1));
    assert_eq!(0, count(&actors, INITIAL_ACTOR_ID), "{actors:?}");
    assert!(count(&actors, ACTOR_A) > 0, "{actors:?}");
    assert_eq!(filled_json("two"), second.lock().unwrap().to_sorted_json());
    Ok(())
}

/// JS "does not re-issue under an explicit client key": every session of an
/// explicit key shares its stable actor.
#[test]
fn does_not_reissue_under_an_explicit_client_key() -> ClientResult<()> {
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, Some("reissue-claim-explicit-client"));
    let doc = filled("reissue-claim-explicit", "hello");

    block_on(client.attach(doc.clone(), manual()))?;

    let actors = actors_of(&pushed(&transport, 0));
    assert!(count(&actors, INITIAL_ACTOR_ID) > 0, "{actors:?}");
    assert_eq!(ACTOR_A, actor_of(&doc));
    Ok(())
}

/// JS `claimReissue` and `markReissueClaim`, cross-checked with Go
/// `TestClaimReissue`. Asking is a pure query (Go records on every ask;
/// JS, followed here, records only through the mark), and a claim is per
/// key and per actor.
#[test]
fn claims_a_reissue_per_key_and_actor() {
    let transport = acking_transport();
    let client = fake_client(ACTOR_A, &transport, None);
    let actor_a = ActorId::from(ACTOR_A);
    let actor_b = ActorId::from(ACTOR_B);

    assert!(client.claim_reissue(&actor_a, "k1"));
    assert!(client.claim_reissue(&actor_a, "k1"));

    client.mark_reissue_claim(actor_a.clone(), "k1");
    assert!(!client.claim_reissue(&actor_a, "k1"));

    assert!(client.claim_reissue(&actor_a, "k2"));
    assert!(client.claim_reissue(&actor_b, "k1"));
    client.mark_reissue_claim(actor_b.clone(), "k1");
    assert!(!client.claim_reissue(&actor_b, "k1"));

    let explicit = fake_client(ACTOR_A, &transport, Some("explicit"));
    assert!(!explicit.claim_reissue(&actor_a, "k1"));
}
