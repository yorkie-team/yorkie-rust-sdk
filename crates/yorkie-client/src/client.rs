//! Client lifecycle: activation, document attachment, and manual sync.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`activate`,
//! `deactivate`, `attach`, `detach`, `sync`, `remove`, `changeSyncMode`).
//!
//! Locking: the client keeps its own state behind one mutex, and each attached
//! document sits behind its own [`SharedDocument`] mutex. Neither lock is held
//! across an `.await`, and the client never holds both at once.

use crate::attachment::{
    default_document_poll_interval, resolve_document_poll_interval, Attachment, SharedDocument,
};
use crate::connect::ConnectTransport;
use crate::error::{ClientError, ClientResult};
use crate::options::{AttachOptions, ClientOptions, DeactivateOptions, DetachOptions, SyncMode};
use crate::transport::{
    ActivateClientRequest, AttachDocumentRequest, ClientTransport, DeactivateClientRequest,
    DetachDocumentRequest, PushPullChangesRequest, RemoveDocumentRequest,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Debug, Formatter};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use yorkie_core::{ActorId, DocStatus, Document};

/// Client activation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    Deactivated,
    Activated,
}

/// Client background loop condition keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClientCondition {
    SyncLoop,
    WatchLoop,
}

/// A Yorkie client (JS `Client`).
///
/// It owns its transport and the documents attached to it. Clones share the
/// same client, so it can be handed to background tasks.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    key: String,
    options: ClientOptions,
    transport: Arc<dyn ClientTransport>,
    state: Mutex<ClientState>,
}

/// Mutable client state. Locked only for synchronous steps.
struct ClientState {
    /// Per-session ID the server routes requests by (JS `id`).
    id: Option<ActorId>,
    /// Actor stamped into document changes (JS `actorID`).
    actor_id: Option<ActorId>,
    status: ClientStatus,
    conditions: BTreeMap<ClientCondition, bool>,
    attachments: BTreeMap<String, Attachment>,
    /// Keys with an attach in flight (JS `attachingDocs`).
    attaching: BTreeSet<String>,
}

/// The IDs an active client sends in requests and stamps into changes.
struct Session {
    client_id: ActorId,
    actor_id: ActorId,
}

/// A document to push and pull, captured from its attachment.
struct SyncTarget {
    key: String,
    doc: SharedDocument,
    document_id: String,
    /// Mode the request is sent in; `RealtimePushOnly` sends push-only.
    request_mode: SyncMode,
    /// Mode of the attachment when the sync started.
    attachment_mode: SyncMode,
}

impl Client {
    /// Creates a client that talks to `options.rpc_addr` over Connect.
    pub fn new(options: ClientOptions) -> ClientResult<Self> {
        let transport = ConnectTransport::new(&options)?;
        Ok(Self::with_transport(options, Arc::new(transport)))
    }

    /// Creates a client over the given transport, e.g. a test double.
    pub fn with_transport(options: ClientOptions, transport: Arc<dyn ClientTransport>) -> Self {
        let key = options.key.clone().unwrap_or_else(generate_client_key);
        let state = ClientState {
            id: None,
            actor_id: None,
            status: ClientStatus::Deactivated,
            conditions: BTreeMap::from([
                (ClientCondition::SyncLoop, false),
                (ClientCondition::WatchLoop, false),
            ]),
            attachments: BTreeMap::new(),
            attaching: BTreeSet::new(),
        };

        Self {
            inner: Arc::new(ClientInner {
                key,
                options,
                transport,
                state: Mutex::new(state),
            }),
        }
    }

    /// Returns the per-session client ID used to route requests (JS `getID`).
    pub fn id(&self) -> Option<ActorId> {
        self.state().id.clone()
    }

    /// Returns the actor stamped into document changes (JS `getActorID`).
    pub fn actor_id(&self) -> Option<ActorId> {
        self.state().actor_id.clone()
    }

    pub fn key(&self) -> &str {
        &self.inner.key
    }

    pub fn status(&self) -> ClientStatus {
        self.state().status
    }

    pub fn is_active(&self) -> bool {
        self.status() == ClientStatus::Activated
    }

    pub fn condition(&self, condition: ClientCondition) -> bool {
        self.state()
            .conditions
            .get(&condition)
            .copied()
            .unwrap_or(false)
    }

    pub fn options(&self) -> &ClientOptions {
        &self.inner.options
    }

    /// Returns whether a document with the given key is attached.
    pub fn has(&self, key: &str) -> bool {
        self.state().attachments.contains_key(key)
    }

    /// Registers this client with the server (JS `activate`).
    pub async fn activate(&self) -> ClientResult<()> {
        if self.is_active() {
            return Ok(());
        }

        let response = self
            .inner
            .transport
            .activate_client(ActivateClientRequest {
                client_key: self.inner.key.clone(),
                metadata: self.inner.options.metadata.clone(),
                shard_key: self.shard_key(&self.inner.key),
            })
            .await?;

        let mut state = self.state();
        state.actor_id = Some(
            response
                .actor_id
                .unwrap_or_else(|| response.client_id.clone()),
        );
        state.id = Some(response.client_id);
        state.status = ClientStatus::Activated;
        state.conditions.insert(ClientCondition::SyncLoop, true);
        Ok(())
    }

    /// Deactivates this client and detaches every attached document locally
    /// (JS `deactivate` and `deactivateInternal`).
    pub async fn deactivate(&self, options: DeactivateOptions) -> ClientResult<()> {
        let client_id = {
            let state = self.state();
            if state.status == ClientStatus::Deactivated {
                return Ok(());
            }
            state.session(&self.inner.key)?.client_id
        };

        self.inner
            .transport
            .deactivate_client(DeactivateClientRequest {
                client_id,
                synchronous: options.synchronous,
                shard_key: self.shard_key(&self.inner.key),
            })
            .await?;

        let attachments = {
            let mut state = self.state();
            state.status = ClientStatus::Deactivated;
            state.conditions.insert(ClientCondition::SyncLoop, false);
            state.conditions.insert(ClientCondition::WatchLoop, false);
            std::mem::take(&mut state.attachments)
        };
        for attachment in attachments.into_values() {
            lock_document(&attachment.doc).apply_status(DocStatus::Detached);
        }
        Ok(())
    }

    /// Attaches the document and returns it (JS `attach`).
    ///
    /// As in JS, the document is stamped with this client's actor and its
    /// initial presence is set before the attach request is sent.
    pub async fn attach(
        &self,
        doc: SharedDocument,
        options: AttachOptions,
    ) -> ClientResult<SharedDocument> {
        let AttachOptions {
            initial_root,
            initial_presence,
            sync_mode,
            document_poll_interval,
            schema,
        } = options;

        let session = self.session()?;
        let key = {
            let doc = lock_document(&doc);
            if doc.status() != DocStatus::Detached {
                return Err(ClientError::NotDetached(doc.key().to_owned()));
            }
            doc.key().to_owned()
        };
        let _attaching = AttachingGuard::claim(&self.inner, &key)?;
        let (poll_interval, poll_interval_pinned) =
            resolve_document_poll_interval(sync_mode, document_poll_interval)?;

        let change_pack = {
            let mut doc = lock_document(&doc);
            doc.set_actor(session.actor_id);
            doc.update_with_presence(|_, presence| {
                presence.set(initial_presence);
                Ok(())
            })?;
            doc.create_change_pack()
        };

        let response = self
            .inner
            .transport
            .attach_document(AttachDocumentRequest {
                client_id: session.client_id,
                change_pack,
                schema_key: schema,
                shard_key: self.shard_key(&key),
            })
            .await?;

        let removed = {
            let mut doc = lock_document(&doc);
            if response.max_size_per_document > 0 {
                doc.set_max_size_per_document(response.max_size_per_document);
            }
            if !response.schema_rules.is_empty() {
                doc.set_schema_rules(response.schema_rules.clone());
            }
            doc.apply_change_pack(&response.change_pack)?;
            let removed = doc.status() == DocStatus::Removed;
            if !removed {
                doc.apply_status(DocStatus::Attached);
            }
            removed
        };
        if removed {
            return Ok(doc);
        }

        {
            let mut state = self.state();
            state.attachments.insert(
                key,
                Attachment::new(
                    doc.clone(),
                    response.document_id,
                    sync_mode,
                    poll_interval,
                    poll_interval_pinned,
                ),
            );
            state.refresh_watch_loop_condition();
        }

        if let Some(initial_root) = initial_root {
            let entries = initial_root
                .iter()
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect::<Vec<_>>();
            lock_document(&doc).update(|root| {
                for (key, value) in entries {
                    if root.get(&key).is_none() {
                        root.set(key, value)?;
                    }
                }
                Ok(())
            })?;
        }

        Ok(doc)
    }

    /// Detaches the document (JS `detach`).
    ///
    /// As in JS, the presence is cleared first so peers drop this client's
    /// presence when the detach change reaches them.
    pub async fn detach(&self, doc: &SharedDocument, _options: DetachOptions) -> ClientResult<()> {
        let session = self.session()?;
        let key = document_key(doc);
        let document_id = self.attached_document_id(&key)?;

        let change_pack = {
            let mut doc = lock_document(doc);
            doc.update_with_presence(|_, presence| {
                presence.clear();
                Ok(())
            })?;
            doc.create_change_pack()
        };

        let response = self
            .inner
            .transport
            .detach_document(DetachDocumentRequest {
                client_id: session.client_id,
                document_id,
                change_pack,
                remove_if_not_attached: false,
                shard_key: self.shard_key(&key),
            })
            .await?;

        {
            let mut doc = lock_document(doc);
            doc.apply_change_pack(&response.change_pack)?;
            if doc.status() != DocStatus::Removed {
                doc.apply_status(DocStatus::Detached);
            }
        }
        self.remove_attachment(&key);
        Ok(())
    }

    /// Pushes local changes and pulls remote ones (JS `sync`).
    ///
    /// With a document, it syncs that document in push-pull mode. With
    /// `None`, it syncs every attached document in its attachment's mode.
    pub async fn sync(&self, doc: Option<&SharedDocument>) -> ClientResult<()> {
        let session = self.session()?;
        let targets = match doc {
            Some(doc) => {
                let key = document_key(doc);
                let state = self.state();
                let attachment = state
                    .attachments
                    .get(&key)
                    .ok_or_else(|| ClientError::NotAttached(key.clone()))?;
                vec![SyncTarget::new(key, attachment, SyncMode::Realtime)]
            }
            None => self
                .state()
                .attachments
                .iter()
                .map(|(key, attachment)| {
                    SyncTarget::new(key.clone(), attachment, attachment.sync_mode)
                })
                .collect(),
        };

        for target in targets {
            self.sync_document(&session.client_id, target).await?;
        }
        Ok(())
    }

    /// Removes the document on the server (JS `remove`).
    pub async fn remove(&self, doc: &SharedDocument) -> ClientResult<()> {
        let session = self.session()?;
        let key = document_key(doc);
        let document_id = self.attached_document_id(&key)?;

        let change_pack = {
            let mut doc = lock_document(doc);
            doc.set_actor(session.actor_id);
            let mut change_pack = doc.create_change_pack();
            change_pack.set_removed(true);
            change_pack
        };

        let response = self
            .inner
            .transport
            .remove_document(RemoveDocumentRequest {
                client_id: session.client_id,
                document_id,
                change_pack,
                shard_key: self.shard_key(&key),
            })
            .await?;

        lock_document(doc).apply_change_pack(&response.change_pack)?;
        self.remove_attachment(&key);
        Ok(())
    }

    /// Changes the sync mode of an attached document (JS `changeSyncMode`).
    pub async fn change_sync_mode(
        &self,
        doc: &SharedDocument,
        sync_mode: SyncMode,
    ) -> ClientResult<()> {
        let key = document_key(doc);
        let mut state = self.state();
        state.session(&self.inner.key)?;
        let Some(attachment) = state.attachments.get_mut(&key) else {
            return Err(ClientError::NotAttached(key));
        };

        if attachment.sync_mode == sync_mode {
            return Ok(());
        }

        attachment.sync_mode = sync_mode;
        if !attachment.poll_interval_pinned {
            attachment.poll_interval = default_document_poll_interval(sync_mode);
        }
        state.refresh_watch_loop_condition();
        Ok(())
    }

    async fn sync_document(&self, client_id: &ActorId, target: SyncTarget) -> ClientResult<()> {
        let change_pack = lock_document(&target.doc).create_change_pack();
        let response = self
            .inner
            .transport
            .push_pull_changes(PushPullChangesRequest {
                client_id: client_id.clone(),
                document_id: target.document_id,
                change_pack,
                push_only: target.request_mode == SyncMode::RealtimePushOnly,
                shard_key: self.shard_key(&target.key),
            })
            .await?;

        if response.change_pack.has_changes()
            && matches!(
                target.attachment_mode,
                SyncMode::RealtimePushOnly | SyncMode::RealtimeSyncOff
            )
        {
            return Ok(());
        }

        let removed = {
            let mut doc = lock_document(&target.doc);
            doc.apply_change_pack(&response.change_pack)?;
            doc.status() == DocStatus::Removed
        };
        if removed {
            self.remove_attachment(&target.key);
        }
        Ok(())
    }

    fn state(&self) -> MutexGuard<'_, ClientState> {
        self.inner.state()
    }

    fn session(&self) -> ClientResult<Session> {
        self.state().session(&self.inner.key)
    }

    fn attached_document_id(&self, key: &str) -> ClientResult<String> {
        self.state()
            .attachments
            .get(key)
            .map(|attachment| attachment.resource_id.clone())
            .ok_or_else(|| ClientError::NotAttached(key.to_owned()))
    }

    fn remove_attachment(&self, key: &str) {
        let mut state = self.state();
        state.attachments.remove(key);
        state.refresh_watch_loop_condition();
    }

    fn shard_key(&self, resource_key: &str) -> String {
        format!("{}/{}", self.inner.options.api_key, resource_key)
    }

    #[cfg(test)]
    fn apply_activation(&self, actor_id: impl Into<ActorId>) {
        let actor_id = actor_id.into();
        let mut state = self.state();
        state.id = Some(actor_id.clone());
        state.actor_id = Some(actor_id);
        state.status = ClientStatus::Activated;
    }
}

impl Debug for Client {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let state = self.state();
        f.debug_struct("Client")
            .field("key", &self.inner.key)
            .field("id", &state.id)
            .field("status", &state.status)
            .field("attachments", &state.attachments.keys())
            .finish_non_exhaustive()
    }
}

impl ClientInner {
    fn state(&self) -> MutexGuard<'_, ClientState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ClientState {
    fn session(&self, client_key: &str) -> ClientResult<Session> {
        match (&self.status, &self.id, &self.actor_id) {
            (ClientStatus::Activated, Some(client_id), Some(actor_id)) => Ok(Session {
                client_id: client_id.clone(),
                actor_id: actor_id.clone(),
            }),
            _ => Err(ClientError::ClientNotActivated(client_key.to_owned())),
        }
    }

    fn refresh_watch_loop_condition(&mut self) {
        let has_watch_attachment = self
            .attachments
            .values()
            .any(|attachment| should_start_watch_loop(attachment.sync_mode));
        self.conditions
            .insert(ClientCondition::WatchLoop, has_watch_attachment);
    }
}

impl SyncTarget {
    fn new(key: String, attachment: &Attachment, request_mode: SyncMode) -> Self {
        Self {
            key,
            doc: attachment.doc.clone(),
            document_id: attachment.resource_id.clone(),
            request_mode,
            attachment_mode: attachment.sync_mode,
        }
    }
}

/// Marks a key as attaching for the lifetime of one `attach` call, so a
/// concurrent attach of the same key is rejected (JS `attachingDocs`).
struct AttachingGuard<'a> {
    inner: &'a ClientInner,
    key: String,
}

impl<'a> AttachingGuard<'a> {
    fn claim(inner: &'a ClientInner, key: &str) -> ClientResult<Self> {
        let mut state = inner.state();
        if state.attachments.contains_key(key) || !state.attaching.insert(key.to_owned()) {
            return Err(ClientError::AlreadyAttached(key.to_owned()));
        }
        Ok(Self {
            inner,
            key: key.to_owned(),
        })
    }
}

impl Drop for AttachingGuard<'_> {
    fn drop(&mut self) {
        self.inner.state().attaching.remove(&self.key);
    }
}

fn lock_document(doc: &SharedDocument) -> MutexGuard<'_, Document> {
    doc.lock().unwrap_or_else(PoisonError::into_inner)
}

fn document_key(doc: &SharedDocument) -> String {
    lock_document(doc).key().to_owned()
}

fn generate_client_key() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("rust-{timestamp:x}-{counter:x}")
}

fn should_start_watch_loop(sync_mode: SyncMode) -> bool {
    sync_mode != SyncMode::Manual && sync_mode != SyncMode::Polling
}

#[cfg(test)]
mod tests {
    use super::{Client, ClientCondition, ClientStatus};
    use crate::attachment::SharedDocument;
    use crate::error::{ClientError, ClientResult};
    use crate::options::{
        AttachChannelOptions, AttachOptions, ClientOptions, DeactivateOptions, DetachOptions,
        SyncMode, DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS, DEFAULT_POLLING_INTERVAL_MS,
        DEFAULT_RECONNECT_STREAM_DELAY_MS, DEFAULT_RETRY_SYNC_LOOP_DELAY_MS, DEFAULT_RPC_ADDR,
        DEFAULT_SYNC_LOOP_DURATION_MS,
    };
    use crate::transport::{
        ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest,
        AttachDocumentResponse, ClientTransport, DeactivateClientRequest, DeactivateClientResponse,
        DetachDocumentRequest, DetachDocumentResponse, PushPullChangesRequest,
        PushPullChangesResponse, RemoveDocumentRequest, RemoveDocumentResponse,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;
    use yorkie_core::{
        ActorId, DocStatus, Document, JsonObject, Presence, SchemaRule, TreeNodeRule, YorkieError,
    };
    use yorkie_protocol::converter;
    use yorkie_protocol::yorkie::v1::presence_change::ChangeType;

    #[derive(Debug)]
    struct MockTransport {
        client_id: ActorId,
        actor_id: Option<ActorId>,
        activate_requests: Mutex<Vec<ActivateClientRequest>>,
        deactivate_requests: Mutex<Vec<DeactivateClientRequest>>,
        attach_requests: Mutex<Vec<AttachDocumentRequest>>,
        detach_requests: Mutex<Vec<DetachDocumentRequest>>,
        remove_requests: Mutex<Vec<RemoveDocumentRequest>>,
        push_pull_requests: Mutex<Vec<PushPullChangesRequest>>,
        attach_max_size_per_document: usize,
        attach_schema_rules: Vec<SchemaRule>,
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self {
                client_id: ActorId::new("000000000000000000000001"),
                actor_id: None,
                activate_requests: Mutex::new(Vec::new()),
                deactivate_requests: Mutex::new(Vec::new()),
                attach_requests: Mutex::new(Vec::new()),
                detach_requests: Mutex::new(Vec::new()),
                remove_requests: Mutex::new(Vec::new()),
                push_pull_requests: Mutex::new(Vec::new()),
                attach_max_size_per_document: 0,
                attach_schema_rules: Vec::new(),
            }
        }
    }

    #[async_trait]
    impl ClientTransport for MockTransport {
        async fn activate_client(
            &self,
            request: ActivateClientRequest,
        ) -> ClientResult<ActivateClientResponse> {
            self.activate_requests.lock().unwrap().push(request);
            Ok(ActivateClientResponse {
                client_id: self.client_id.clone(),
                actor_id: self.actor_id.clone(),
            })
        }

        async fn deactivate_client(
            &self,
            request: DeactivateClientRequest,
        ) -> ClientResult<DeactivateClientResponse> {
            self.deactivate_requests.lock().unwrap().push(request);
            Ok(DeactivateClientResponse)
        }

        async fn attach_document(
            &self,
            request: AttachDocumentRequest,
        ) -> ClientResult<AttachDocumentResponse> {
            let change_pack = request.change_pack.clone();
            self.attach_requests.lock().unwrap().push(request);
            Ok(AttachDocumentResponse {
                document_id: "document-id".to_owned(),
                change_pack,
                max_size_per_document: self.attach_max_size_per_document,
                schema_rules: self.attach_schema_rules.clone(),
            })
        }

        async fn detach_document(
            &self,
            request: DetachDocumentRequest,
        ) -> ClientResult<DetachDocumentResponse> {
            let change_pack = request.change_pack.clone();
            self.detach_requests.lock().unwrap().push(request);
            Ok(DetachDocumentResponse { change_pack })
        }

        async fn remove_document(
            &self,
            request: RemoveDocumentRequest,
        ) -> ClientResult<RemoveDocumentResponse> {
            let change_pack = request.change_pack.clone();
            self.remove_requests.lock().unwrap().push(request);
            Ok(RemoveDocumentResponse { change_pack })
        }

        async fn push_pull_changes(
            &self,
            request: PushPullChangesRequest,
        ) -> ClientResult<PushPullChangesResponse> {
            let change_pack = request.change_pack.clone();
            self.push_pull_requests.lock().unwrap().push(request);
            Ok(PushPullChangesResponse { change_pack })
        }
    }

    fn block_on<F>(future: F) -> F::Output
    where
        F: Future,
    {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn mock_client(options: ClientOptions, transport: &Arc<MockTransport>) -> Client {
        Client::with_transport(options, transport.clone())
    }

    fn keyed_options() -> ClientOptions {
        ClientOptions {
            key: Some("client-key".to_owned()),
            api_key: "api-key".to_owned(),
            ..ClientOptions::default()
        }
    }

    fn shared(key: &str) -> SharedDocument {
        Arc::new(Mutex::new(Document::new(key)))
    }

    fn presence(value: serde_json::Value) -> Presence {
        value.as_object().unwrap().clone()
    }

    /// Presence change type of the last change in a pack, as sent on the wire.
    fn last_presence_change_type(pack: &yorkie_core::ChangePack) -> Option<ChangeType> {
        let pack = converter::to_change_pack(pack).unwrap();
        let change = pack.changes.last()?;
        let presence_change = change.presence_change.as_ref()?;
        ChangeType::try_from(presence_change.r#type).ok()
    }

    #[test]
    fn creates_client_with_default_options() {
        let client = Client::new(ClientOptions::default()).unwrap();

        assert!(!client.key().is_empty());
        assert_eq!(ClientStatus::Deactivated, client.status());
        assert!(!client.is_active());
        assert!(!client.condition(ClientCondition::SyncLoop));
        assert!(!client.condition(ClientCondition::WatchLoop));

        let options = client.options();
        assert_eq!(DEFAULT_RPC_ADDR, options.rpc_addr);
        assert_eq!(
            Duration::from_millis(DEFAULT_SYNC_LOOP_DURATION_MS),
            options.sync_loop_duration
        );
        assert_eq!(
            Duration::from_millis(DEFAULT_RETRY_SYNC_LOOP_DELAY_MS),
            options.retry_sync_loop_delay
        );
        assert_eq!(
            Duration::from_millis(DEFAULT_RECONNECT_STREAM_DELAY_MS),
            options.reconnect_stream_delay
        );
        assert_eq!(
            Duration::from_millis(DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS),
            options.channel_heartbeat_interval
        );
    }

    #[test]
    fn creates_attachment_option_defaults() {
        let deactivate_options = DeactivateOptions::default();
        let attach_options = AttachOptions::default();
        let attach_channel_options = AttachChannelOptions::default();
        let detach_options = DetachOptions;

        assert!(!deactivate_options.keepalive);
        assert!(!deactivate_options.synchronous);
        assert_eq!(None, attach_options.initial_root);
        assert!(attach_options.initial_presence.is_empty());
        assert_eq!(SyncMode::Realtime, attach_options.sync_mode);
        assert_eq!(None, attach_options.document_poll_interval);
        assert_eq!(None, attach_options.schema);
        assert_eq!(None, attach_channel_options.sync_mode);
        assert_eq!(None, attach_channel_options.channel_heartbeat_interval);
        assert_eq!(DetachOptions, detach_options);
        assert_eq!(
            Duration::from_millis(3000),
            Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)
        );
    }

    #[test]
    fn carries_sync_mode_values() {
        let attach_options = AttachOptions {
            sync_mode: SyncMode::Polling,
            document_poll_interval: Some(Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)),
            schema: Some("schema".to_owned()),
            ..AttachOptions::default()
        };
        let channel_options = AttachChannelOptions {
            sync_mode: Some(SyncMode::Realtime),
            channel_heartbeat_interval: Some(Duration::from_millis(
                DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS,
            )),
        };

        assert_eq!(SyncMode::Polling, attach_options.sync_mode);
        assert_eq!(
            Some(Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS)),
            attach_options.document_poll_interval
        );
        assert_eq!(Some(SyncMode::Realtime), channel_options.sync_mode);
        assert_eq!(SyncMode::Manual, SyncMode::Manual);
        assert_eq!(SyncMode::RealtimePushOnly, SyncMode::RealtimePushOnly);
        assert_eq!(SyncMode::RealtimeSyncOff, SyncMode::RealtimeSyncOff);
    }

    #[test]
    fn uses_explicit_client_options() {
        let options = ClientOptions {
            rpc_addr: "http://localhost:8080".to_owned(),
            key: Some("client-key".to_owned()),
            api_key: "api-key".to_owned(),
            metadata: BTreeMap::from([("region".to_owned(), "local".to_owned())]),
            sync_loop_duration: Duration::from_millis(25),
            retry_sync_loop_delay: Duration::from_millis(200),
            reconnect_stream_delay: Duration::from_millis(300),
            channel_heartbeat_interval: Duration::from_secs(10),
            user_agent: Some("test-agent".to_owned()),
        };

        let client = Client::new(options.clone()).unwrap();

        assert_eq!("client-key", client.key());
        assert_eq!(&options, client.options());
        assert_eq!(
            Some("local"),
            client.options().metadata.get("region").map(String::as_str)
        );
    }

    #[test]
    fn clones_share_one_client() {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let clone = client.clone();

        block_on(clone.activate()).unwrap();

        assert!(client.is_active());
        assert_eq!(1, transport.activate_requests.lock().unwrap().len());
    }

    #[test]
    fn client_futures_are_send() {
        fn assert_send<T: Send>(_: &T) {}
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        assert_send(&client.activate());
        assert_send(&client.attach(doc.clone(), AttachOptions::default()));
        assert_send(&client.sync(Some(&doc)));
        assert_send(&client.detach(&doc, DetachOptions));
        assert_send(&client.remove(&doc));
        assert_send(&client.change_sync_mode(&doc, SyncMode::Manual));
        assert_send(&client.deactivate(DeactivateOptions::default()));
    }

    #[test]
    fn activates_client_through_transport() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(
            ClientOptions {
                metadata: BTreeMap::from([("region".to_owned(), "local".to_owned())]),
                ..keyed_options()
            },
            &transport,
        );

        block_on(client.activate())?;
        block_on(client.activate())?;

        assert_eq!(ClientStatus::Activated, client.status());
        assert!(client.is_active());
        assert_eq!(
            Some("000000000000000000000001"),
            client.id().as_ref().map(ActorId::as_str)
        );
        assert_eq!(client.id(), client.actor_id());
        assert!(client.condition(ClientCondition::SyncLoop));
        let requests = transport.activate_requests.lock().unwrap();
        assert_eq!(1, requests.len());
        assert_eq!("client-key", requests[0].client_key);
        assert_eq!("api-key/client-key", requests[0].shard_key);
        assert_eq!(
            Some("local"),
            requests[0].metadata.get("region").map(String::as_str)
        );
        Ok(())
    }

    /// JS `this.actorID = res.actorId || res.clientId`: changes carry the
    /// actor, while requests keep routing by the per-session client ID.
    #[test]
    fn stamps_changes_with_actor_id_and_routes_by_client_id() -> ClientResult<()> {
        let transport = Arc::new(MockTransport {
            actor_id: Some(ActorId::new("000000000000000000000002")),
            ..MockTransport::default()
        });
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        block_on(client.activate())?;
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;
        doc.lock()
            .unwrap()
            .update(|root| root.set("title", "hello").map(|_| ()))?;
        block_on(client.sync(Some(&doc)))?;
        block_on(client.remove(&doc))?;

        assert_eq!(
            Some("000000000000000000000001"),
            client.id().as_ref().map(ActorId::as_str)
        );
        assert_eq!(
            Some("000000000000000000000002"),
            client.actor_id().as_ref().map(ActorId::as_str)
        );
        assert_eq!(
            "000000000000000000000002",
            doc.lock().unwrap().actor_id().as_str()
        );
        let attach = &transport.attach_requests.lock().unwrap()[0];
        assert_eq!("000000000000000000000001", attach.client_id.as_str());
        let pack = converter::to_change_pack(&attach.change_pack).unwrap();
        assert_eq!(
            Some(vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]),
            pack.changes[0].id.as_ref().map(|id| id.actor_id.clone())
        );
        assert_eq!(
            "000000000000000000000001",
            transport.push_pull_requests.lock().unwrap()[0]
                .client_id
                .as_str()
        );
        assert_eq!(
            "000000000000000000000001",
            transport.remove_requests.lock().unwrap()[0]
                .client_id
                .as_str()
        );
        Ok(())
    }

    #[test]
    fn deactivates_client_and_detaches_documents() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");
        let other = shared("other-key");

        block_on(client.activate())?;
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;
        block_on(client.attach(other.clone(), AttachOptions::default()))?;
        block_on(client.deactivate(DeactivateOptions {
            keepalive: true,
            synchronous: true,
        }))?;
        block_on(client.deactivate(DeactivateOptions::default()))?;

        assert_eq!(ClientStatus::Deactivated, client.status());
        assert!(!client.is_active());
        assert_eq!(
            Some("000000000000000000000001"),
            client.id().as_ref().map(ActorId::as_str)
        );
        assert!(!client.condition(ClientCondition::SyncLoop));
        assert!(!client.condition(ClientCondition::WatchLoop));
        assert!(!client.has("doc-key"));
        assert!(!client.has("other-key"));
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert_eq!(DocStatus::Detached, other.lock().unwrap().status());
        let requests = transport.deactivate_requests.lock().unwrap();
        assert_eq!(1, requests.len());
        assert_eq!("000000000000000000000001", requests[0].client_id.as_str());
        assert!(requests[0].synchronous);
        assert_eq!("api-key/client-key", requests[0].shard_key);
        Ok(())
    }

    #[test]
    fn rejects_attach_when_client_is_not_active() {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        let err = block_on(client.attach(doc.clone(), AttachOptions::default())).unwrap_err();

        assert_eq!(
            ClientError::ClientNotActivated("client-key".to_owned()),
            err
        );
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert!(!client.has("doc-key"));
        assert!(transport.attach_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn attaches_document_through_transport_and_records_lifecycle_state() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");
        let mut initial_root = JsonObject::new();
        initial_root.set("title", "hello")?;

        client.apply_activation("000000000000000000000001");
        let attached = block_on(client.attach(
            doc.clone(),
            AttachOptions {
                initial_root: Some(initial_root),
                sync_mode: SyncMode::Polling,
                schema: Some("schema".to_owned()),
                ..AttachOptions::default()
            },
        ))?;

        assert!(Arc::ptr_eq(&doc, &attached));
        assert!(client.has("doc-key"));
        {
            let doc = doc.lock().unwrap();
            assert_eq!(DocStatus::Attached, doc.status());
            assert!(doc.is_attached());
            assert_eq!("000000000000000000000001", doc.actor_id().as_str());
            assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
            assert_eq!(0, doc.max_size_per_document());
            assert!(doc.schema_rules().is_empty());
        }
        {
            let state = client.state();
            let attachment = state.attachments.get("doc-key").unwrap();
            assert!(Arc::ptr_eq(&doc, &attachment.doc));
            assert_eq!("document-id", attachment.resource_id);
            assert_eq!(SyncMode::Polling, attachment.sync_mode);
            assert_eq!(
                Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS),
                attachment.poll_interval
            );
            assert!(!attachment.poll_interval_pinned);
        }
        assert!(!client.condition(ClientCondition::WatchLoop));
        {
            let requests = transport.attach_requests.lock().unwrap();
            assert_eq!(1, requests.len());
            assert_eq!("000000000000000000000001", requests[0].client_id.as_str());
            assert_eq!("api-key/doc-key", requests[0].shard_key);
            assert_eq!(Some("schema"), requests[0].schema_key.as_deref());
            assert_eq!("doc-key", requests[0].change_pack.document_key());
        }

        block_on(client.detach(&doc, DetachOptions))?;

        assert!(!client.has("doc-key"));
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert_eq!(
            "000000000000000000000000",
            doc.lock().unwrap().actor_id().as_str()
        );
        let requests = transport.detach_requests.lock().unwrap();
        assert_eq!(1, requests.len());
        assert_eq!("000000000000000000000001", requests[0].client_id.as_str());
        assert_eq!("document-id", requests[0].document_id);
        assert_eq!("doc-key", requests[0].change_pack.document_key());
        assert!(!requests[0].remove_if_not_attached);
        assert_eq!("api-key/doc-key", requests[0].shard_key);
        Ok(())
    }

    /// JS `attach` runs `doc.update((_, p) => p.set(initialPresence))` before
    /// the attach request, so the presence rides on the attach change pack.
    #[test]
    fn attach_sends_initial_presence() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(
            doc.clone(),
            AttachOptions {
                initial_presence: presence(json!({"name": "a"})),
                ..AttachOptions::default()
            },
        ))?;

        assert_eq!(
            presence(json!({"name": "a"})),
            doc.lock().unwrap().my_presence()
        );
        let requests = transport.attach_requests.lock().unwrap();
        assert_eq!(
            Some(ChangeType::Put),
            last_presence_change_type(&requests[0].change_pack)
        );
        Ok(())
    }

    /// JS `detach` runs `doc.update((_, p) => p.clear())` before detaching.
    #[test]
    fn detach_clears_presence() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;
        block_on(client.detach(&doc, DetachOptions))?;

        let requests = transport.detach_requests.lock().unwrap();
        assert_eq!(
            Some(ChangeType::Clear),
            last_presence_change_type(&requests[0].change_pack)
        );
        Ok(())
    }

    /// JS rejects a second document with an attached key before any request
    /// (`ErrAlreadyAttached`).
    #[test]
    fn rejects_attach_of_another_document_with_an_attached_key() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");
        let duplicate = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(doc, AttachOptions::default()))?;
        let err = block_on(client.attach(duplicate.clone(), AttachOptions::default())).unwrap_err();

        assert_eq!(ClientError::AlreadyAttached("doc-key".to_owned()), err);
        assert_eq!(DocStatus::Detached, duplicate.lock().unwrap().status());
        assert_eq!(1, transport.attach_requests.lock().unwrap().len());
        assert!(client.state().attaching.is_empty());
        Ok(())
    }

    #[test]
    fn removes_document_through_transport() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;
        doc.lock()
            .unwrap()
            .update(|root| root.set("title", "hello").map(|_| ()))?;

        block_on(client.remove(&doc))?;

        assert_eq!(DocStatus::Removed, doc.lock().unwrap().status());
        assert!(!client.has("doc-key"));
        assert!(!client.condition(ClientCondition::WatchLoop));
        assert!(!doc.lock().unwrap().has_local_changes());
        {
            let requests = transport.remove_requests.lock().unwrap();
            assert_eq!(1, requests.len());
            assert_eq!("000000000000000000000001", requests[0].client_id.as_str());
            assert_eq!("document-id", requests[0].document_id);
            assert_eq!("doc-key", requests[0].change_pack.document_key());
            assert!(requests[0].change_pack.is_removed());
            assert!(requests[0].change_pack.has_changes());
            assert_eq!("api-key/doc-key", requests[0].shard_key);
        }
        assert_eq!(
            YorkieError::DocumentRemoved("doc-key".to_owned()),
            doc.lock()
                .unwrap()
                .update(|root| root.set("next", "value").map(|_| ()))
                .unwrap_err()
        );
        assert_eq!(
            ClientError::NotDetached("doc-key".to_owned()),
            block_on(client.attach(doc, AttachOptions::default())).unwrap_err()
        );
        Ok(())
    }

    #[test]
    fn rejects_remove_and_detach_when_client_is_not_active_or_document_is_not_attached() {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        let not_active = ClientError::ClientNotActivated("client-key".to_owned());
        assert_eq!(not_active, block_on(client.remove(&doc)).unwrap_err());
        assert_eq!(
            not_active,
            block_on(client.detach(&doc, DetachOptions)).unwrap_err()
        );

        client.apply_activation("000000000000000000000001");
        let not_attached = ClientError::NotAttached("doc-key".to_owned());
        assert_eq!(not_attached, block_on(client.remove(&doc)).unwrap_err());
        assert_eq!(
            not_attached,
            block_on(client.detach(&doc, DetachOptions)).unwrap_err()
        );
        assert!(transport.remove_requests.lock().unwrap().is_empty());
        assert!(transport.detach_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn syncs_document_through_push_pull_transport() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;
        doc.lock()
            .unwrap()
            .update(|root| root.set("title", "hello").map(|_| ()))?;
        assert!(doc.lock().unwrap().has_local_changes());

        block_on(client.sync(Some(&doc)))?;

        assert!(!doc.lock().unwrap().has_local_changes());
        assert_eq!(DocStatus::Attached, doc.lock().unwrap().status());
        let requests = transport.push_pull_requests.lock().unwrap();
        assert_eq!(1, requests.len());
        assert_eq!("000000000000000000000001", requests[0].client_id.as_str());
        assert_eq!("document-id", requests[0].document_id);
        assert_eq!("doc-key", requests[0].change_pack.document_key());
        assert!(requests[0].change_pack.has_changes());
        assert!(!requests[0].push_only);
        assert_eq!("api-key/doc-key", requests[0].shard_key);
        Ok(())
    }

    /// JS `sync()` with no resource syncs every attached document.
    #[test]
    fn sync_without_document_syncs_every_attachment() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let first = shared("first");
        let second = shared("second");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(first.clone(), AttachOptions::default()))?;
        block_on(client.attach(second.clone(), AttachOptions::default()))?;
        for doc in [&first, &second] {
            doc.lock()
                .unwrap()
                .update(|root| root.set("title", "hello").map(|_| ()))?;
        }

        block_on(client.sync(None))?;

        assert!(!first.lock().unwrap().has_local_changes());
        assert!(!second.lock().unwrap().has_local_changes());
        let requests = transport.push_pull_requests.lock().unwrap();
        let mut keys = requests
            .iter()
            .map(|request| request.change_pack.document_key().to_owned())
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(vec!["first", "second"], keys);
        Ok(())
    }

    #[test]
    fn rejects_sync_when_client_is_not_active_or_document_is_not_attached() {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        let err = block_on(client.sync(Some(&doc))).unwrap_err();
        assert_eq!(
            ClientError::ClientNotActivated("client-key".to_owned()),
            err
        );

        client.apply_activation("000000000000000000000001");
        let err = block_on(client.sync(Some(&doc))).unwrap_err();
        assert_eq!(ClientError::NotAttached("doc-key".to_owned()), err);
        assert!(transport.push_pull_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn push_only_sync_ignores_response_changes_for_push_only_attachment() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(
            doc.clone(),
            AttachOptions {
                sync_mode: SyncMode::RealtimePushOnly,
                ..AttachOptions::default()
            },
        ))?;
        doc.lock()
            .unwrap()
            .update(|root| root.set("title", "hello").map(|_| ()))?;

        block_on(client.sync(None))?;

        assert!(doc.lock().unwrap().has_local_changes());
        let requests = transport.push_pull_requests.lock().unwrap();
        assert_eq!(1, requests.len());
        assert!(requests[0].push_only);
        Ok(())
    }

    #[test]
    fn rejects_attach_when_document_is_not_detached() {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        doc.lock().unwrap().apply_status(DocStatus::Attached);

        let err = block_on(client.attach(doc, AttachOptions::default())).unwrap_err();

        assert_eq!(ClientError::NotDetached("doc-key".to_owned()), err);
        assert!(transport.attach_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn changes_document_sync_mode_and_validates_poll_interval() -> ClientResult<()> {
        let transport = Arc::new(MockTransport {
            attach_max_size_per_document: 4096,
            attach_schema_rules: vec![SchemaRule::new(
                "$.profile",
                "object",
                vec![TreeNodeRule::new("paragraph", "text*", "bold", "block")],
            )],
            ..MockTransport::default()
        });
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;

        assert!(client.condition(ClientCondition::WatchLoop));
        {
            let doc = doc.lock().unwrap();
            assert_eq!(4096, doc.max_size_per_document());
            assert_eq!(
                Some("$.profile"),
                doc.schema_rules().first().map(|rule| rule.path.as_str())
            );
            assert_eq!(
                Some("paragraph"),
                doc.schema_rules()
                    .first()
                    .and_then(|rule| rule.tree_nodes.first())
                    .map(|rule| rule.node_type.as_str())
            );
        }
        block_on(client.change_sync_mode(&doc, SyncMode::Polling))?;

        assert!(!client.condition(ClientCondition::WatchLoop));
        {
            let state = client.state();
            let attachment = state.attachments.get("doc-key").unwrap();
            assert_eq!(SyncMode::Polling, attachment.sync_mode);
            assert_eq!(
                Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS),
                attachment.poll_interval
            );
        }

        let other_doc = shared("other-doc");
        let err = block_on(client.attach(
            other_doc,
            AttachOptions {
                document_poll_interval: Some(Duration::ZERO),
                ..AttachOptions::default()
            },
        ))
        .unwrap_err();
        assert_eq!(
            ClientError::InvalidArgument(
                "document_poll_interval must be greater than 0".to_owned()
            ),
            err
        );
        assert!(client.state().attaching.is_empty());
        Ok(())
    }
}
