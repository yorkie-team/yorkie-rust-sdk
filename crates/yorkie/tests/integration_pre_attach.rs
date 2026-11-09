//! Integration tests for documents edited before they are attached, against a
//! live server.
//!
//! JS reference: `packages/sdk/test/integration/pre_attach_test.ts` ("Pre-attach
//! edits", `ed25355e`), cross-checked with Go
//! `test/integration/pre_attach_test.go` (`TestPreAttachEdits`, `1061bc35`).
//! Edits made before the first attach create their elements under the initial
//! actor; the attach re-issues those tickets to the client's own actor, so two
//! clients that fill the same key no longer push values with the same
//! `created_at`.
//!
//! Not ported: the Go-only half of "attach re-issues pre-attach tickets to the
//! client's actor" that attaches under a canceled context and retries under
//! another client. JS has no counterpart, and Go attaches under the
//! per-session client ID where JS and Rust attach under the stable actor.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::two_clients::{active_client, manual_attach, shared};
use integration_support::{deactivate, rpc_addr, unique_key};
use yorkie::{
    ActorId, Client, ClientOptions, DetachOptions, JsonObject, JsonValue, SharedDocument,
};

const INITIAL_ACTOR: &str = "000000000000000000000000";

/// Activates `n` clients with generated keys (JS `activeClients`).
async fn active_clients(addr: &str, n: usize) -> Vec<Client> {
    let mut clients = Vec::with_capacity(n);
    for _ in 0..n {
        clients.push(active_client(addr).await);
    }
    clients
}

/// Activates a client under an explicit client key (JS `activeClients(1, {
/// key })`).
async fn active_keyed_client(addr: &str, key: &str) -> Client {
    let client = Client::new(ClientOptions {
        rpc_addr: addr.to_owned(),
        key: Some(key.to_owned()),
        ..ClientOptions::default()
    })
    .unwrap();
    client.activate().await.unwrap();
    client
}

/// Sets a new Text under `key` of a detached document (JS `fill`).
fn fill(doc: &SharedDocument, key: &str, content: &str) {
    doc.lock()
        .unwrap()
        .update(|root| {
            root.set_text(key)?.edit(0, 0, content, None)?;
            Ok(())
        })
        .unwrap();
}

/// The actor of the Text under `key` (JS `creatorOf`).
fn creator_of(doc: &SharedDocument, key: &str) -> ActorId {
    match doc.lock().unwrap().get_root().get(key) {
        Some(JsonValue::Text(text)) => text.id().actor_id().clone(),
        other => panic!("expected a text under {key}, got {other:?}"),
    }
}

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn client_actor(client: &Client) -> ActorId {
    client.actor_id().expect("an active client has an actor")
}

/// JS "converges two clients that filled the same key before attach"
/// (pre_attach_test.ts:62); Go "two clients filling the same key before
/// attach converge".
#[tokio::test]
async fn converges_two_clients_that_filled_the_same_key_before_attach() {
    let Some(addr) = rpc_addr() else { return };
    // Their elements are created under the initial actor, so two clients that
    // fill the same key used to push values with the same created_at.
    let doc_key = unique_key("pre-attach-converges");

    let mut last = String::new();
    let mut last_text = String::new();
    for round in 0..3 {
        let clients = active_clients(&addr, 2).await;
        let docs = [shared(&doc_key), shared(&doc_key)];
        let contents = [format!("round{round}-c1"), format!("round{round}-c2")];
        for (doc, content) in docs.iter().zip(&contents) {
            doc.lock()
                .unwrap()
                .update(|root| {
                    root.set_text("k1")?.edit(0, 0, content, None)?;
                    let mut by = JsonObject::new();
                    by.set("by", JsonValue::String(content.clone()))?;
                    root.set("o", by)?;
                    Ok(())
                })
                .unwrap();
        }

        let (a, b) = tokio::join!(
            clients[0].attach(docs[0].clone(), manual_attach()),
            clients[1].attach(docs[1].clone(), manual_attach()),
        );
        a.unwrap();
        b.unwrap();
        for client in [&clients[0], &clients[1], &clients[0]] {
            client.sync(None).await.unwrap();
        }
        assert_eq!(sorted_json(&docs[0]), sorted_json(&docs[1]));

        // One whole client's write wins every key, never a mix or nothing.
        // Every pre-attach write has lamport 1, so the previous round's
        // winner may still win by the actor tie-break -- but nothing older.
        let (text, by) = {
            let doc = docs[0].lock().unwrap();
            let root = doc.get_root();
            let text = match root.get("k1") {
                Some(JsonValue::Text(text)) => text.to_string(),
                other => panic!("expected a text under k1, got {other:?}"),
            };
            let by = match root.get("o") {
                Some(JsonValue::Object(object)) => object.get("by").cloned(),
                other => panic!("expected an object under o, got {other:?}"),
            };
            (text, by)
        };
        let mut candidates = contents.to_vec();
        if !last_text.is_empty() {
            candidates.push(last_text.clone());
        }
        assert!(
            candidates.contains(&text),
            "{text} not in {candidates:?}: {}",
            sorted_json(&docs[0])
        );
        assert_eq!(Some(JsonValue::String(text.clone())), by);
        last_text = text;
        last = sorted_json(&docs[0]);

        deactivate(&[&clients[0], &clients[1]]).await;
    }

    let observer = active_client(&addr).await;
    let observed = observer
        .attach(shared(&doc_key), manual_attach())
        .await
        .unwrap();
    assert_eq!(last, sorted_json(&observed));
    deactivate(&[&observer]).await;
}

/// JS "re-issues pre-attach tickets to the client actor"
/// (pre_attach_test.ts:118); Go "attach re-issues pre-attach tickets to the
/// client's actor".
#[tokio::test]
async fn reissues_pre_attach_tickets_to_the_client_actor() {
    let Some(addr) = rpc_addr() else { return };
    let clients = active_clients(&addr, 2).await;
    let (c1, c2) = (&clients[0], &clients[1]);
    let doc = shared(&unique_key("pre-attach-reissue"));
    fill(&doc, "k1", "abc");
    assert_eq!(INITIAL_ACTOR, creator_of(&doc, "k1").as_str());

    let doc = c1.attach(doc, manual_attach()).await.unwrap();
    assert_eq!(client_actor(c1), creator_of(&doc, "k1"));
    assert_eq!(r#"{"k1":[{"val":"abc"}]}"#, sorted_json(&doc));

    let key = doc.lock().unwrap().key().to_owned();
    let observed = c2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(sorted_json(&doc), sorted_json(&observed));
    assert_eq!(client_actor(c1), creator_of(&observed, "k1"));

    deactivate(&[c1, c2]).await;
}

/// JS "does not re-issue a second pre-attach document of a key"
/// (pre_attach_test.ts:141); Go "a second pre-attach document of the same key
/// is not re-issued".
#[tokio::test]
async fn does_not_reissue_a_second_pre_attach_document_of_a_key() {
    let Some(addr) = rpc_addr() else { return };
    // A re-issue keeps the lamports, and a fresh document starts them at 1,
    // so re-issuing this one to the same actor would mint the created_at the
    // first attach already pushed. The tickets keep the initial actor.
    let doc_key = unique_key("pre-attach-second-doc");
    let clients = active_clients(&addr, 2).await;
    let (client, observer) = (&clients[0], &clients[1]);

    let doc1 = shared(&doc_key);
    fill(&doc1, "first", "a");
    let doc1 = client.attach(doc1, manual_attach()).await.unwrap();
    assert_eq!(client_actor(client), creator_of(&doc1, "first"));
    client.detach(&doc1, DetachOptions).await.unwrap();

    let doc2 = shared(&doc_key);
    fill(&doc2, "second", "b");
    let doc2 = client.attach(doc2, manual_attach()).await.unwrap();
    assert_eq!(INITIAL_ACTOR, creator_of(&doc2, "second").as_str());

    // Both elements survive. Read them through another client: a pull drops
    // the changes the pulling actor itself pushed up to its checkpoint's
    // client_seq, and doc2 repeated the client_seq doc1 pushed under, so its
    // own pull filters doc1's change out (yorkie#2123).
    let observed = observer
        .attach(shared(&doc_key), manual_attach())
        .await
        .unwrap();
    assert_eq!(
        r#"{"first":[{"val":"a"}],"second":[{"val":"b"}]}"#,
        sorted_json(&observed)
    );

    deactivate(&[client, observer]).await;
}

/// JS "keeps an earlier session of an explicit client key"
/// (pre_attach_test.ts:178). Go has no counterpart: it re-issues under the
/// per-session client ID for any key.
#[tokio::test]
async fn keeps_an_earlier_session_of_an_explicit_client_key() {
    let Some(addr) = rpc_addr() else { return };
    // Every session of an explicit key shares its stable actor. A re-issue in
    // the second session would mint the tickets the first session's first
    // edits carry, and the server would keep one of the two elements, so
    // pre-attach tickets under an explicit key keep the initial actor.
    let doc_key = unique_key("pre-attach-explicit-key");
    let client_key = format!("{doc_key}-client");

    let first = active_keyed_client(&addr, &client_key).await;
    let doc1 = first
        .attach(shared(&doc_key), manual_attach())
        .await
        .unwrap();
    fill(&doc1, "first", "a");
    first.sync(None).await.unwrap();
    let first_actor = client_actor(&first);
    deactivate(&[&first]).await;

    let second = active_keyed_client(&addr, &client_key).await;
    assert_eq!(first_actor, client_actor(&second));
    let doc2 = shared(&doc_key);
    fill(&doc2, "second", "b");
    let doc2 = second.attach(doc2, manual_attach()).await.unwrap();
    assert_eq!(INITIAL_ACTOR, creator_of(&doc2, "second").as_str());

    let observer = active_client(&addr).await;
    let observed = observer
        .attach(shared(&doc_key), manual_attach())
        .await
        .unwrap();
    assert_eq!(
        r#"{"first":[{"val":"a"}],"second":[{"val":"b"}]}"#,
        sorted_json(&observed)
    );

    deactivate(&[&second, &observer]).await;
}
