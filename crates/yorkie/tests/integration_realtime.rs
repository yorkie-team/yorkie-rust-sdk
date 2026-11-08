//! Integration tests for the watch stream and the realtime sync loop.
//!
//! JS reference: `packages/sdk/test/integration/client_test.ts` ("Can change
//! sync mode(realtime <-> manual)", and the watch stream side of "Can recover
//! from temporary disconnect (realtime sync)"). The rest of `client_test.ts`
//! is in `integration_client.rs`, and `doc_presence_test.ts` in
//! `integration_doc_presence.rs`.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use integration_support::{eventually, rpc_addr, unique_key};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yorkie::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    AttachOptions, BroadcastRequest, BroadcastResponse, ChannelWatchStream, Client, ClientError,
    ClientOptions, ClientResult, ClientTransport, ConnectTransport, DeactivateClientRequest,
    DeactivateClientResponse, DeactivateOptions, DetachDocumentRequest, DetachDocumentResponse,
    DocEvent, Document, PeekChannelRequest, PeekChannelResponse, PushPullChangesRequest,
    PushPullChangesResponse, RefreshChannelRequest, RefreshChannelResponse, RemoveDocumentRequest,
    RemoveDocumentResponse, SharedDocument, StreamConnectionStatus, SyncMode, WatchChannelRequest,
    WatchDocumentRequest, WatchStream,
};

type Events = Arc<Mutex<Vec<DocEvent>>>;

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

fn collect(doc: &SharedDocument) -> Events {
    let events: Events = Arc::default();
    let sink = events.clone();
    doc.lock()
        .unwrap()
        .subscribe(move |event| sink.lock().unwrap().push(event.clone()));
    events
}

fn has_event(events: &Events, expected: &DocEvent) -> bool {
    events.lock().unwrap().contains(expected)
}

/// JS `client_test.ts` realtime cases: changes reach the other client through
/// the watch stream and the sync loop, with no `sync` call.
#[tokio::test]
async fn realtime_clients_converge_without_manual_sync() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("realtime-converge");
    let a = active_client(&addr).await;
    let b = active_client(&addr).await;
    let doc_a = a
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();
    let doc_b = b
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    set(&doc_a, "k1", "v1");
    eventually("b to receive k1", || {
        sorted_json(&doc_b) == r#"{"k1":"v1"}"#
    })
    .await;

    set(&doc_b, "k2", "v2");
    eventually("a to receive k2", || {
        sorted_json(&doc_a) == r#"{"k1":"v1","k2":"v2"}"#
    })
    .await;
    assert_eq!(sorted_json(&doc_a), sorted_json(&doc_b));

    a.deactivate(DeactivateOptions::default()).await.unwrap();
    b.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// A transport whose first watch stream fails after its first message.
struct FlakyWatchTransport {
    inner: ConnectTransport,
    watch_calls: AtomicUsize,
}

#[async_trait]
impl ClientTransport for FlakyWatchTransport {
    async fn activate_client(
        &self,
        request: ActivateClientRequest,
    ) -> ClientResult<ActivateClientResponse> {
        self.inner.activate_client(request).await
    }

    async fn deactivate_client(
        &self,
        request: DeactivateClientRequest,
    ) -> ClientResult<DeactivateClientResponse> {
        self.inner.deactivate_client(request).await
    }

    async fn attach_document(
        &self,
        request: AttachDocumentRequest,
    ) -> ClientResult<AttachDocumentResponse> {
        self.inner.attach_document(request).await
    }

    async fn detach_document(
        &self,
        request: DetachDocumentRequest,
    ) -> ClientResult<DetachDocumentResponse> {
        self.inner.detach_document(request).await
    }

    async fn remove_document(
        &self,
        request: RemoveDocumentRequest,
    ) -> ClientResult<RemoveDocumentResponse> {
        self.inner.remove_document(request).await
    }

    async fn push_pull_changes(
        &self,
        request: PushPullChangesRequest,
    ) -> ClientResult<PushPullChangesResponse> {
        self.inner.push_pull_changes(request).await
    }

    async fn watch_document(&self, request: WatchDocumentRequest) -> ClientResult<WatchStream> {
        let first = self.watch_calls.fetch_add(1, Ordering::SeqCst) == 0;
        let stream = self.inner.watch_document(request).await?;
        if !first {
            return Ok(stream);
        }
        let failure = stream::iter([Err(ClientError::Transport(
            "injected stream failure".to_owned(),
        ))]);
        Ok(Box::pin(stream.take(1).chain(failure)))
    }

    async fn refresh_channel(
        &self,
        request: RefreshChannelRequest,
    ) -> ClientResult<RefreshChannelResponse> {
        self.inner.refresh_channel(request).await
    }

    async fn peek_channel(&self, request: PeekChannelRequest) -> ClientResult<PeekChannelResponse> {
        self.inner.peek_channel(request).await
    }

    async fn watch_channel(
        &self,
        request: WatchChannelRequest,
    ) -> ClientResult<ChannelWatchStream> {
        self.inner.watch_channel(request).await
    }

    async fn broadcast(&self, request: BroadcastRequest) -> ClientResult<BroadcastResponse> {
        self.inner.broadcast(request).await
    }
}

/// Review Focus 1. JS "Can recover from temporary disconnect (realtime
/// sync)": a failed watch stream is reported as disconnected, reopened, and
/// the documents still converge.
#[tokio::test]
async fn watch_reconnects_after_stream_error() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("watch-reconnects");
    let options = ClientOptions {
        reconnect_stream_delay: Duration::from_millis(100),
        ..options(&addr)
    };
    let transport = Arc::new(FlakyWatchTransport {
        inner: ConnectTransport::new(&options).unwrap(),
        watch_calls: AtomicUsize::new(0),
    });
    let a = Client::with_transport(options, transport.clone());
    a.activate().await.unwrap();
    let b = active_client(&addr).await;

    let doc_a = shared(&key);
    let events = collect(&doc_a);
    let doc_a = a.attach(doc_a, AttachOptions::default()).await.unwrap();
    let doc_b = b
        .attach(shared(&key), AttachOptions::default())
        .await
        .unwrap();

    eventually("a second watch stream", || {
        transport.watch_calls.load(Ordering::SeqCst) >= 2
    })
    .await;
    let disconnected = DocEvent::ConnectionChanged(StreamConnectionStatus::Disconnected);
    assert!(has_event(&events, &disconnected));
    // `Connected` is published before the second Watch request is sent, so
    // the server may not watch for `a` yet. Its `Initialized` comes after it
    // does; the one before `Disconnected` is the closed stream's.
    eventually("the second stream to initialize", || {
        let events = events.lock().unwrap();
        let after_disconnect = events
            .iter()
            .rposition(|event| event == &disconnected)
            .map_or(&events[..], |index| &events[index + 1..]);
        after_disconnect
            .iter()
            .any(|event| matches!(event, DocEvent::Initialized(_)))
    })
    .await;

    set(&doc_b, "k1", "v1");
    eventually("a to receive k1", || {
        sorted_json(&doc_a) == r#"{"k1":"v1"}"#
    })
    .await;
    set(&doc_a, "k2", "v2");
    eventually("b to receive k2", || {
        sorted_json(&doc_b) == r#"{"k1":"v1","k2":"v2"}"#
    })
    .await;

    a.deactivate(DeactivateOptions::default()).await.unwrap();
    b.deactivate(DeactivateOptions::default()).await.unwrap();
}

/// JS `client_test.ts` "Can change sync mode(realtime <-> manual)".
#[tokio::test]
async fn change_sync_mode_manual_stops_auto_sync() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("change-sync-mode");
    let manual = || AttachOptions {
        sync_mode: SyncMode::Manual,
        ..AttachOptions::default()
    };
    let a = active_client(&addr).await;
    let b = active_client(&addr).await;

    // 01. Both attach manually; b sees a's change only after syncing.
    let doc_a = a.attach(shared(&key), manual()).await.unwrap();
    let doc_b = b.attach(shared(&key), manual()).await.unwrap();
    set(&doc_a, "version", "v1");
    a.sync(None).await.unwrap();
    assert_eq!("{}", sorted_json(&doc_b));
    b.sync(None).await.unwrap();
    assert_eq!(r#"{"version":"v1"}"#, sorted_json(&doc_b));

    // 02. b switches to realtime and then follows a without syncing.
    b.change_sync_mode(&doc_b, SyncMode::Realtime)
        .await
        .unwrap();
    set(&doc_a, "version", "v2");
    a.sync(None).await.unwrap();
    eventually("b to receive v2", || {
        sorted_json(&doc_b) == r#"{"version":"v2"}"#
    })
    .await;

    // 03. b switches back to manual and stops following a.
    b.change_sync_mode(&doc_b, SyncMode::Manual).await.unwrap();
    set(&doc_a, "version", "v3");
    a.sync(None).await.unwrap();
    // Several sync-loop ticks (50ms each) in which nothing may arrive.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(r#"{"version":"v2"}"#, sorted_json(&doc_b));
    b.sync(None).await.unwrap();
    assert_eq!(r#"{"version":"v3"}"#, sorted_json(&doc_b));

    a.deactivate(DeactivateOptions::default()).await.unwrap();
    b.deactivate(DeactivateOptions::default()).await.unwrap();
}
