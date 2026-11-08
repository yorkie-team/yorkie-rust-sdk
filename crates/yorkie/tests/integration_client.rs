//! Integration tests for the client against a live server.
//!
//! JS reference: `packages/sdk/test/integration/client_test.ts` and
//! `doc_presence_test.ts`.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::{eventually, rpc_addr, unique_key};
use std::sync::{Arc, Mutex};
use yorkie::{
    AttachOptions, Client, ClientError, ClientOptions, DeactivateOptions, DetachOptions, DocStatus,
    Document, Presence, SharedDocument, SyncMode,
};

fn manual_attach() -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        ..AttachOptions::default()
    }
}

fn options(addr: &str) -> ClientOptions {
    ClientOptions {
        rpc_addr: addr.to_owned(),
        ..ClientOptions::default()
    }
}

async fn active_client(addr: &str) -> Client {
    let client = Client::new(options(addr)).unwrap();
    client.activate().await.unwrap();
    client
}

fn shared(key: &str) -> SharedDocument {
    Arc::new(Mutex::new(Document::new(key)))
}

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn set(doc: &SharedDocument, key: &str, value: &str) {
    doc.lock()
        .unwrap()
        .update(|root| root.set(key, value).map(|_| ()))
        .unwrap();
}

#[tokio::test]
async fn activate_attach_sync_detach_roundtrip() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("roundtrip");

    let a = active_client(&addr).await;
    let doc_a = a.attach(shared(&key), manual_attach()).await.unwrap();
    set(&doc_a, "k1", "v1");
    a.sync(Some(&doc_a)).await.unwrap();
    a.detach(&doc_a, DetachOptions).await.unwrap();

    let b = active_client(&addr).await;
    let doc_b = b.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&doc_b));

    a.deactivate(DeactivateOptions::default()).await.unwrap();
    b.deactivate(DeactivateOptions::default()).await.unwrap();
}

#[tokio::test]
async fn attach_twice_fails() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("attach-twice");
    let client = active_client(&addr).await;
    let doc = shared(&key);

    client.attach(doc.clone(), manual_attach()).await.unwrap();
    let err = client.attach(doc, manual_attach()).await.unwrap_err();
    assert!(matches!(err, ClientError::NotDetached(_)), "{err:?}");

    // A second Document object with the same key is rejected by key.
    let err = client
        .attach(shared(&key), manual_attach())
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::AlreadyAttached(_)), "{err:?}");

    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();
}

/// JS `client_test.ts` "Can handle sync".
#[tokio::test]
async fn two_clients_converge_with_manual_sync() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("two-clients-converge");
    let a = active_client(&addr).await;
    let b = active_client(&addr).await;
    let doc_a = a.attach(shared(&key), manual_attach()).await.unwrap();
    let doc_b = b.attach(shared(&key), manual_attach()).await.unwrap();

    set(&doc_a, "k1", "v1");
    a.sync(None).await.unwrap();
    b.sync(None).await.unwrap();
    assert_eq!(r#"{"k1":"v1"}"#, sorted_json(&doc_b));
    assert_eq!(sorted_json(&doc_a), sorted_json(&doc_b));

    set(&doc_b, "k2", "v2");
    b.sync(Some(&doc_b)).await.unwrap();
    a.sync(Some(&doc_a)).await.unwrap();
    assert_eq!(r#"{"k1":"v1","k2":"v2"}"#, sorted_json(&doc_a));
    assert_eq!(sorted_json(&doc_a), sorted_json(&doc_b));

    a.deactivate(DeactivateOptions::default()).await.unwrap();
    b.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// JS `doc_presence_test.ts` "Can be set initial value in attach and be
/// removed in detach".
#[tokio::test]
async fn attach_with_initial_presence() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("attach-initial-presence");
    let presence = |name: &str| {
        let mut presence = Presence::new();
        presence.insert("name".to_owned(), name.into());
        presence
    };
    let attach = |name: &str| AttachOptions {
        initial_presence: presence(name),
        ..manual_attach()
    };

    let a = active_client(&addr).await;
    let b = active_client(&addr).await;
    let doc_a = a.attach(shared(&key), attach("a")).await.unwrap();
    let doc_b = b.attach(shared(&key), attach("b")).await.unwrap();
    a.sync(None).await.unwrap();
    b.sync(None).await.unwrap();

    let actor_a = a.actor_id().unwrap();
    let actor_b = b.actor_id().unwrap();
    assert_eq!(presence("a"), doc_a.lock().unwrap().my_presence());
    // Online peers come from the watch stream, which manual mode does not
    // open. Mark them online so `presence` reads the synced value, as JS
    // `getPresenceForTest` does without the online check.
    {
        let mut doc_b = doc_b.lock().unwrap();
        doc_b.add_online_client(actor_a.clone());
        assert_eq!(Some(presence("a")), doc_b.presence(&actor_a));
    }
    {
        let mut doc_a = doc_a.lock().unwrap();
        doc_a.add_online_client(actor_b.clone());
        assert_eq!(Some(presence("b")), doc_a.presence(&actor_b));
    }

    // Detach clears the presence, so the peer drops it on its next sync.
    b.detach(&doc_b, DetachOptions).await.unwrap();
    a.sync(None).await.unwrap();
    assert!(!doc_a.lock().unwrap().has_presence(&actor_b));

    a.deactivate(DeactivateOptions::default()).await.unwrap();
    b.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// Review Focus 5. JS `deactivateInternal` detaches every attachment and
/// sets each document to `Detached`; JS `deactivate` stops the sync loop and
/// `detachInternal` cancels each watch stream.
#[tokio::test]
async fn deactivate_detaches_documents_and_stops_tasks() {
    let Some(addr) = rpc_addr() else { return };
    let client = active_client(&addr).await;
    let keys = [
        unique_key("deactivate-detaches-1"),
        unique_key("deactivate-detaches-2"),
    ];
    let docs = vec![
        client
            .attach(shared(&keys[0]), manual_attach())
            .await
            .unwrap(),
        client
            .attach(shared(&keys[1]), AttachOptions::default())
            .await
            .unwrap(),
    ];
    // The sync loop and the realtime document's watch loop.
    assert_eq!(2, client.running_task_count());

    client
        .deactivate(DeactivateOptions::default())
        .await
        .unwrap();

    assert!(!client.is_active());
    for (key, doc) in keys.iter().zip(&docs) {
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert!(!client.has(key));
    }
    eventually("background loops to stop", || {
        client.running_task_count() == 0
    })
    .await;
}
