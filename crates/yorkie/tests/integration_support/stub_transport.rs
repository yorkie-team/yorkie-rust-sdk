//! A transport over the real server whose push-pull calls a test can fail or
//! observe. It stands in for what JS tests do with `vi.stubGlobal('fetch',
//! ...)` and by replacing `client.rpcClient.pushPullChanges`.

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use yorkie::{
    ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest, AttachDocumentResponse,
    BroadcastRequest, BroadcastResponse, ChannelWatchStream, Client, ClientError, ClientOptions,
    ClientResult, ClientTransport, ConnectTransport, DeactivateClientRequest,
    DeactivateClientResponse, DetachDocumentRequest, DetachDocumentResponse, PeekChannelRequest,
    PeekChannelResponse, PushPullChangesRequest, PushPullChangesResponse, RefreshChannelRequest,
    RefreshChannelResponse, RemoveDocumentRequest, RemoveDocumentResponse, WatchChannelRequest,
    WatchDocumentRequest, WatchStream,
};

/// A network failure shared by every client built on it, as a stubbed
/// global `fetch` is shared by every JS client. While it is set, each
/// push-pull fails with it before reaching the server.
#[derive(Default)]
pub struct Network {
    failure: Mutex<Option<ClientError>>,
    failed: AtomicUsize,
}

impl Network {
    /// Fails every later push-pull with `err` until `restore`.
    pub fn fail_with(&self, err: ClientError) {
        *self.failure.lock().unwrap() = Some(err);
    }

    /// Lets push-pulls reach the server again (JS `vi.unstubAllGlobals`).
    pub fn restore(&self) {
        *self.failure.lock().unwrap() = None;
    }

    /// How many push-pulls have failed so far.
    pub fn failed(&self) -> usize {
        self.failed.load(Ordering::SeqCst)
    }

    fn check(&self) -> ClientResult<()> {
        match self.failure.lock().unwrap().clone() {
            Some(err) => {
                self.failed.fetch_add(1, Ordering::SeqCst);
                Err(err)
            }
            None => Ok(()),
        }
    }
}

/// The error a JS test's stubbed `fetch` makes the client see: Connect
/// reports a failed fetch as `unknown`.
pub fn failed_to_fetch() -> ClientError {
    rpc_error("unknown", "Failed to fetch")
}

/// A server error with the given Connect code and no Yorkie code.
pub fn rpc_error(code: &str, message: &str) -> ClientError {
    ClientError::Rpc {
        code: code.to_owned(),
        message: message.to_owned(),
        yorkie_code: None,
        reason: None,
    }
}

/// Runs after the server answers a push-pull and before the client sees the
/// answer; the client waits for the future it returns.
pub type PushPullHook = Arc<
    dyn Fn(&PushPullChangesRequest, &PushPullChangesResponse) -> BoxFuture<'static, ()>
        + Send
        + Sync,
>;

pub struct StubTransport {
    inner: ConnectTransport,
    network: Arc<Network>,
    on_push_pull: Mutex<Option<PushPullHook>>,
}

impl StubTransport {
    /// Calls `hook` for every later push-pull answer, until `clear_hook`.
    pub fn set_hook(&self, hook: PushPullHook) {
        *self.on_push_pull.lock().unwrap() = Some(hook);
    }

    pub fn clear_hook(&self) {
        *self.on_push_pull.lock().unwrap() = None;
    }
}

/// A client on a `StubTransport` over `network`, not yet activated.
pub fn stubbed_client(
    options: ClientOptions,
    network: Arc<Network>,
) -> (Client, Arc<StubTransport>) {
    let transport = Arc::new(StubTransport {
        inner: ConnectTransport::new(&options).unwrap(),
        network,
        on_push_pull: Mutex::new(None),
    });
    (
        Client::with_transport(options, transport.clone()),
        transport,
    )
}

#[async_trait]
impl ClientTransport for StubTransport {
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
        self.network.check()?;
        let hook = self.on_push_pull.lock().unwrap().clone();
        let response = self.inner.push_pull_changes(request.clone()).await?;
        if let Some(hook) = hook {
            hook(&request, &response).await;
        }
        Ok(response)
    }

    async fn watch_document(&self, request: WatchDocumentRequest) -> ClientResult<WatchStream> {
        self.inner.watch_document(request).await
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
