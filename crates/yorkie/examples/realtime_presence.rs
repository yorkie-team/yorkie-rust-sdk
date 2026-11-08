//! Realtime attach with initial presence, and Watched/PresenceChanged events.
//!
//! Run a server (`yorkie server`), then:
//! `YORKIE_RPC_ADDR=http://localhost:8080 cargo run -p yorkie --example realtime_presence`
//! The example exits after about one second.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use yorkie::{
    AttachOptions, Client, ClientOptions, DeactivateOptions, DocEvent, Document, Presence,
    SharedDocument,
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

fn presence(name: &str) -> Presence {
    let mut presence = Presence::new();
    presence.insert("name".to_owned(), name.into());
    presence
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = rpc_addr();
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let key = format!("realtime-presence-{millis}");

    let alice = connect(&addr).await?;
    let bob = connect(&addr).await?;

    // Alice subscribes before Bob arrives. Callbacks run under the document
    // lock, so they must not lock the same document.
    let doc_a: SharedDocument = Arc::new(Mutex::new(Document::new(&key)));
    doc_a.lock().unwrap().subscribe(|event| match event {
        DocEvent::Watched(actor, presence) => println!("watched: {actor:?} {presence:?}"),
        DocEvent::PresenceChanged(actor, presence) => {
            println!("presence changed: {actor:?} {presence:?}")
        }
        _ => {}
    });
    let doc_a = alice
        .attach(
            doc_a,
            AttachOptions {
                initial_presence: presence("alice"),
                ..AttachOptions::default()
            },
        )
        .await?;

    let doc_b: SharedDocument = Arc::new(Mutex::new(Document::new(&key)));
    let doc_b = bob
        .attach(
            doc_b,
            AttachOptions {
                initial_presence: presence("bob"),
                ..AttachOptions::default()
            },
        )
        .await?;

    tokio::time::sleep(Duration::from_millis(300)).await;
    // Bob moves his cursor; Alice sees a PresenceChanged event.
    doc_b.lock().unwrap().update_with_presence(|root, p| {
        root.set("cursor", 1)?;
        p.set(presence("bob (typing)"));
        Ok(())
    })?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!(
        "alice sees {} peers",
        doc_a.lock().unwrap().presences().len()
    );

    alice.deactivate(DeactivateOptions::default()).await?;
    bob.deactivate(DeactivateOptions::default()).await?;
    Ok(())
}
