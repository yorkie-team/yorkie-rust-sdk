//! Tests of `AttachOptions::disable_gc` in `Client::attach` and the sync
//! loop, against the JS client (`client.ts` 1588, 1628-1647, 1698-1708,
//! 4025-4037). The live-server cases are in
//! `crates/yorkie/tests/integration_disable_gc.rs`.

use super::{block_on, keyed_options, mock_client, shared, MockTransport};
use crate::error::ClientResult;
use crate::options::{AttachOptions, DetachOptions, SyncMode};
use std::sync::{Arc, Mutex};
use yorkie_core::{ChangePack, Document};
use yorkie_protocol::converter;

const REMOTE_ACTOR: &str = "0000000000000000000000b1";

/// Two changes another actor pushed, as an attach reply that carries them.
fn remote_pack(key: &str) -> ChangePack {
    let mut remote = Document::new(key);
    remote.set_actor(REMOTE_ACTOR);
    remote.update(|root| root.set("k", 1).map(|_| ())).unwrap();
    remote.update(|root| root.set("k", 2).map(|_| ())).unwrap();
    let mut pack = converter::to_change_pack(&remote.create_change_pack()).unwrap();
    pack.checkpoint.get_or_insert_default().server_seq = 2;
    converter::from_change_pack(&pack).unwrap()
}

fn manual(disable_gc: bool) -> AttachOptions {
    AttachOptions {
        sync_mode: SyncMode::Manual,
        disable_gc,
        ..AttachOptions::default()
    }
}

/// JS sets `disableGC` on the document before applying the attach response
/// (`client.ts` 1640-1645), so the remote changes the response carries
/// already take the lamport-only path, and sends it on the attach request
/// and every push-pull of that attachment.
#[test]
fn attach_applies_its_response_with_the_option_already_set() -> ClientResult<()> {
    for disable_gc in [true, false] {
        let transport = Arc::new(MockTransport {
            attach_reply: Mutex::new(Some(remote_pack("doc-key"))),
            ..MockTransport::default()
        });
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        block_on(client.activate())?;
        block_on(client.attach(doc.clone(), manual(disable_gc)))?;
        {
            let doc = doc.lock().unwrap();
            assert_eq!(r#"{"k":2}"#, doc.to_sorted_json());
            let expected = if disable_gc { 1 } else { 2 };
            assert_eq!(expected, doc.version_vector().size(), "{disable_gc}");
            assert_eq!(
                !disable_gc,
                doc.version_vector().has(REMOTE_ACTOR),
                "{disable_gc}"
            );
        }

        doc.lock()
            .unwrap()
            .update(|root| root.set("j", 3).map(|_| ()))?;
        block_on(client.sync(Some(&doc)))?;
        block_on(client.sync(Some(&doc)))?;

        assert_eq!(
            disable_gc,
            transport.attach_requests.lock().unwrap()[0].disable_gc
        );
        let push_pulls = transport.push_pull_requests.lock().unwrap();
        assert_eq!(2, push_pulls.len());
        assert!(push_pulls.iter().all(|r| r.disable_gc == disable_gc));
    }
    Ok(())
}

/// JS "Re-attach without disableGC restores normal sync behavior"
/// (`disable_gc_test.ts` 217-244), on the wire: the flag belongs to the
/// attachment, so a later attach without it sends `false` again.
#[test]
fn a_reattach_without_the_option_sends_it_off_again() -> ClientResult<()> {
    let transport = Arc::new(MockTransport::default());
    let client = mock_client(keyed_options(), &transport);
    block_on(client.activate())?;

    let first = shared("doc-key");
    block_on(client.attach(first.clone(), manual(true)))?;
    block_on(client.sync(Some(&first)))?;
    block_on(client.detach(&first, DetachOptions))?;

    let second = shared("doc-key");
    block_on(client.attach(second.clone(), manual(false)))?;
    block_on(client.sync(Some(&second)))?;

    let attaches = transport.attach_requests.lock().unwrap();
    assert_eq!(
        vec![true, false],
        attaches.iter().map(|r| r.disable_gc).collect::<Vec<_>>()
    );
    let push_pulls = transport.push_pull_requests.lock().unwrap();
    assert_eq!(
        vec![true, false],
        push_pulls.iter().map(|r| r.disable_gc).collect::<Vec<_>>()
    );
    Ok(())
}
