//! Integration tests for the Connect transport against a live server.
//!
//! JS reference: `packages/sdk/test/integration/client_test.ts`.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::{rpc_addr, unique_key};
use yorkie::{
    AttachOptions, Client, ClientError, ClientOptions, ConnectTransport, DeactivateOptions,
    DetachOptions, Document, SyncMode,
};

fn manual_attach() -> AttachOptions {
    AttachOptions {
        sync_mode: Some(SyncMode::Manual),
        ..AttachOptions::default()
    }
}

fn options(addr: &str) -> ClientOptions {
    ClientOptions {
        rpc_addr: addr.to_owned(),
        ..ClientOptions::default()
    }
}

#[tokio::test]
async fn activate_attach_sync_detach_roundtrip() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("roundtrip");
    let transport = ConnectTransport::new(&options(&addr)).unwrap();

    let mut a = Client::new(options(&addr));
    let mut doc_a = Document::new(key.clone());
    a.activate(&transport).await.unwrap();
    a.attach(&transport, &mut doc_a, manual_attach())
        .await
        .unwrap();
    doc_a
        .update(|root| root.set("k1", "v1").map(|_| ()))
        .unwrap();
    a.sync(&transport, &mut doc_a).await.unwrap();
    a.detach(&transport, &mut doc_a, DetachOptions)
        .await
        .unwrap();

    let mut b = Client::new(options(&addr));
    let mut doc_b = Document::new(key);
    b.activate(&transport).await.unwrap();
    b.attach(&transport, &mut doc_b, manual_attach())
        .await
        .unwrap();
    assert_eq!(r#"{"k1":"v1"}"#, doc_b.to_sorted_json());

    a.deactivate(&transport, DeactivateOptions::default())
        .await
        .unwrap();
    b.deactivate(&transport, DeactivateOptions::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn attach_twice_fails() {
    let Some(addr) = rpc_addr() else { return };
    let transport = ConnectTransport::new(&options(&addr)).unwrap();
    let mut client = Client::new(options(&addr));
    let mut doc = Document::new(unique_key("attach-twice"));

    client.activate(&transport).await.unwrap();
    client
        .attach(&transport, &mut doc, manual_attach())
        .await
        .unwrap();
    let err = client
        .attach(&transport, &mut doc, manual_attach())
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::NotDetached(_)), "{err:?}");

    client
        .deactivate(&transport, DeactivateOptions::default())
        .await
        .unwrap();
}
