//! Two clients edit one document and converge.
//!
//! Run a server (`yorkie server`), then:
//! `YORKIE_RPC_ADDR=http://localhost:8080 cargo run -p yorkie --example quickstart`

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use yorkie::{
    AttachOptions, Client, ClientOptions, DeactivateOptions, Document, SharedDocument, SyncMode,
};

fn rpc_addr() -> String {
    std::env::var("YORKIE_RPC_ADDR").unwrap_or_else(|_| "http://localhost:8080".to_owned())
}

async fn connect(addr: &str) -> Result<Client, Box<dyn std::error::Error>> {
    let client = Client::new(ClientOptions {
        rpc_addr: addr.to_owned(),
        ..ClientOptions::default()
    })?;
    client.activate().await?;
    Ok(client)
}

fn manual() -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        ..AttachOptions::default()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = rpc_addr();
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let key = format!("quickstart-{millis}");

    let alice = connect(&addr).await?;
    let bob = connect(&addr).await?;
    let doc_a: SharedDocument = Arc::new(Mutex::new(Document::new(&key)));
    let doc_b: SharedDocument = Arc::new(Mutex::new(Document::new(&key)));
    let doc_a = alice.attach(doc_a, manual()).await?;
    let doc_b = bob.attach(doc_b, manual()).await?;

    // Alice writes. The lock is never held across an `.await`.
    doc_a.lock().unwrap().update(|root| {
        root.set("title", "Hello Yorkie")?;
        root.set("count", 1)?;
        Ok(())
    })?;
    alice.sync(None).await?;

    // Bob pulls Alice's change and adds his own.
    bob.sync(None).await?;
    doc_b
        .lock()
        .unwrap()
        .update(|root| root.set("author", "bob").map(|_| ()))?;
    bob.sync(None).await?;
    alice.sync(None).await?;

    let a = doc_a.lock().unwrap().to_sorted_json();
    let b = doc_b.lock().unwrap().to_sorted_json();
    assert_eq!(a, b, "documents must converge");
    println!("converged: {a}");

    alice.deactivate(DeactivateOptions::default()).await?;
    bob.deactivate(DeactivateOptions::default()).await?;
    Ok(())
}
