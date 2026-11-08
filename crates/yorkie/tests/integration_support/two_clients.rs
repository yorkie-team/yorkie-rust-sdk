//! Two clients on one document in manual sync mode, the Rust counterpart of
//! JS `withTwoClientsAndDocuments` (`test/integration/integration_helper.ts`).

use super::unique_key;
use std::sync::{Arc, Mutex};
use yorkie::{
    AttachOptions, Client, ClientOptions, DeactivateOptions, DetachOptions, Document,
    SharedDocument, SyncMode,
};

/// Two active clients, each attached to its own replica of one document.
///
/// `c1` always has the lower actor ID. Concurrent-edit conflicts are resolved
/// by the actor tie-break, and the JS tests expect `c2` to win ties, so this
/// pins the order the same way JS `withTwoClientsAndDocuments` does instead of
/// relying on activation order.
pub struct TwoClients {
    pub c1: Client,
    pub d1: SharedDocument,
    pub c2: Client,
    pub d2: SharedDocument,
}

impl TwoClients {
    /// Activates two clients and attaches a fresh document, keyed by `name`
    /// plus a timestamp, to each in manual sync mode.
    pub async fn attach(addr: &str, name: &str) -> Self {
        let mut c1 = active_client(addr).await;
        let mut c2 = active_client(addr).await;
        if c1.actor_id() > c2.actor_id() {
            std::mem::swap(&mut c1, &mut c2);
        }
        let (d1, d2) = attach_pair(&c1, &c2, &unique_key(name)).await;
        Self { c1, d1, c2, d2 }
    }

    /// Syncs c1, c2, then c1 again, so both have every change.
    pub async fn sync_all(&self) {
        self.c1.sync(None).await.unwrap();
        self.c2.sync(None).await.unwrap();
        self.c1.sync(None).await.unwrap();
    }

    /// Detaches the documents that are still attached, then deactivates both
    /// clients, as the JS helper does after its callback.
    pub async fn deactivate(self) {
        for (client, doc) in [(self.c1, self.d1), (self.c2, self.d2)] {
            let attached = doc.lock().unwrap().is_attached();
            if attached {
                client.detach(&doc, DetachOptions).await.unwrap();
            }
            client
                .deactivate(DeactivateOptions::default())
                .await
                .unwrap();
        }
    }
}

pub fn manual_attach() -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        ..AttachOptions::default()
    }
}

pub async fn active_client(addr: &str) -> Client {
    let client = Client::new(ClientOptions {
        rpc_addr: addr.to_owned(),
        ..ClientOptions::default()
    })
    .unwrap();
    client.activate().await.unwrap();
    client
}

/// Attaches a new replica of `key` to each client in manual sync mode.
pub async fn attach_pair(c1: &Client, c2: &Client, key: &str) -> (SharedDocument, SharedDocument) {
    let d1 = c1.attach(shared(key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(key), manual_attach()).await.unwrap();
    (d1, d2)
}

pub fn shared(key: &str) -> SharedDocument {
    Arc::new(Mutex::new(Document::new(key)))
}
