//! Client lifecycle: activation, document and channel attachment, manual
//! sync, and the background loops that keep realtime documents in sync and
//! channel sessions alive.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`activate`,
//! `deactivate`, `attach`, `detach`, `sync`, `remove`, `changeSyncMode`,
//! `attachChannel`, `detachChannel`, `peekChannel`, `enqueueTask`,
//! `syncInternal`, `detachInternal`, `deactivateInternal`,
//! `handleConnectError`, `refreshAuthToken`, `loopRetryDelay`).
//!
//! Errors: a one-shot call that fails on the server handles the error as JS
//! does in its `catch` (refreshing the auth token or deactivating the client)
//! and returns it; the background loops retry or stop by the same rules.
//!
//! Locking: the client keeps its own state behind one mutex, and each attached
//! document or channel sits behind its own [`SharedDocument`] or
//! [`SharedChannel`] mutex. No lock is held across an `.await`, and the client
//! never holds its state lock together with a document or channel lock. Client calls
//! and sync-loop rounds additionally take the async task queue, which is held
//! across `.await`s so that only one of them talks to the server at a time,
//! as in JS `enqueueTask`. Watch streams do not take it.
//!
//! Background loops (the sync loop in [`crate::sync_loop`] and one watch loop
//! per realtime attachment in [`crate::watch`]) are tokio tasks. They hold a
//! `Weak<ClientInner>`, so dropping every `Client` handle drops the client,
//! and their [`TaskHandle`]s live in the client state: dropping a handle
//! stops its loop. A `Client` must therefore be used inside a tokio runtime.

use crate::attachment::{
    default_document_poll_interval, resolve_document_poll_interval, uses_watch_stream,
    validate_channel_sync_mode, Attachment, ChannelAttachment, SharedDocument,
};
use crate::auth::{random_unit, AuthRefresh};
use crate::channel::{
    first_key_path, BroadcastOptions, Channel, ChannelEvent, ChannelStatus, SharedChannel,
};
use crate::connect::ConnectTransport;
use crate::error::{ClientError, ClientResult, ErrorHandling};
use crate::options::{
    AttachChannelOptions, AttachOptions, ClientOptions, DeactivateOptions, DetachOptions, SyncMode,
};
use crate::transport::{
    ActivateClientRequest, AttachDocumentRequest, BroadcastRequest, ClientTransport,
    DeactivateClientRequest, DetachDocumentRequest, PeekChannelRequest, PushPullChangesRequest,
    RefreshChannelRequest, RemoveDocumentRequest,
};
use crate::{sync_loop, watch};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};
use yorkie_core::{ActorId, DocEvent, DocStatus, DocSyncStatus, Document, ErrorCode};

/// Client activation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    Deactivated,
    Activated,
}

/// The wait before the first broadcast retry, doubled for each later one
/// (JS `DefaultBroadcastOptions.initialRetryInterval`).
const BROADCAST_INITIAL_RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// The longest wait between broadcast retries (JS
/// `DefaultBroadcastOptions.maxBackoff`).
const BROADCAST_MAX_BACKOFF: Duration = Duration::from_secs(20);

/// Client background loop condition keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClientCondition {
    SyncLoop,
    WatchLoop,
}

/// A Yorkie client (JS `Client`).
///
/// It owns its transport and the documents attached to it. Clones share the
/// same client. Activating it starts a sync loop, and attaching a document in
/// a realtime mode starts a watch stream; both run as tokio tasks, so the
/// client must be used inside a tokio runtime.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<ClientInner>,
}

pub(crate) struct ClientInner {
    key: String,
    options: ClientOptions,
    pub(crate) transport: Arc<dyn ClientTransport>,
    state: Mutex<ClientState>,
    /// Serializes client calls and sync-loop rounds (JS `enqueueTask`).
    task_queue: tokio::sync::Mutex<()>,
    /// Background loops currently alive, counted by the loops themselves.
    running_tasks: Arc<AtomicUsize>,
}

/// Mutable client state. Locked only for synchronous steps.
pub(crate) struct ClientState {
    /// Per-session ID the server routes requests by (JS `id`).
    id: Option<ActorId>,
    /// Actor stamped into document changes (JS `actorID`).
    actor_id: Option<ActorId>,
    status: ClientStatus,
    conditions: BTreeMap<ClientCondition, bool>,
    pub(crate) attachments: BTreeMap<String, Attachment>,
    /// Attached channels. They share the key space of `attachments`, as in
    /// the single JS `attachmentMap`.
    pub(crate) channels: BTreeMap<String, ChannelAttachment>,
    /// Keys with an attach in flight (JS `attachingDocs`).
    attaching: BTreeSet<String>,
    /// The sync loop task while the client is active.
    sync_loop: Option<TaskHandle>,
    /// Token refreshes after `ErrUnauthenticated`.
    auth_refresh: AuthRefresh,
}

/// The IDs an active client sends in requests and stamps into changes.
pub(crate) struct Session {
    pub(crate) client_id: ActorId,
    pub(crate) actor_id: ActorId,
}

/// A document to push and pull, captured from its attachment.
pub(crate) struct SyncTarget {
    pub(crate) key: String,
    pub(crate) doc: SharedDocument,
    document_id: String,
    /// Mode the request is sent in; `RealtimePushOnly` sends push-only.
    request_mode: SyncMode,
    /// Mode of the attachment when the sync started.
    attachment_mode: SyncMode,
}

/// A running background loop. Dropping the handle stops the loop at its next
/// `.await`.
#[derive(Debug)]
pub(crate) struct TaskHandle(tokio::task::AbortHandle);

impl TaskHandle {
    /// Whether the loop has ended, by returning or being aborted.
    pub(crate) fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Counts a background loop as running until its future is dropped, which
/// happens when it returns or is aborted.
struct RunningTask(Arc<AtomicUsize>);

impl RunningTask {
    fn start(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for RunningTask {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Client {
    /// Creates a client that talks to `options.rpc_addr` over Connect.
    pub fn new(options: ClientOptions) -> ClientResult<Self> {
        let transport = ConnectTransport::new(&options)?;
        Ok(Self::with_transport(options, Arc::new(transport)))
    }

    /// Creates a client over the given transport, e.g. a test double.
    pub fn with_transport(options: ClientOptions, transport: Arc<dyn ClientTransport>) -> Self {
        // JS `opts.key || uuid()`: an empty key is generated too.
        let key = options
            .key
            .clone()
            .filter(|key| !key.is_empty())
            .unwrap_or_else(generate_client_key);
        let state = ClientState {
            id: None,
            actor_id: None,
            status: ClientStatus::Deactivated,
            conditions: BTreeMap::from([
                (ClientCondition::SyncLoop, false),
                (ClientCondition::WatchLoop, false),
            ]),
            attachments: BTreeMap::new(),
            channels: BTreeMap::new(),
            attaching: BTreeSet::new(),
            sync_loop: None,
            auth_refresh: AuthRefresh::default(),
        };

        Self {
            inner: Arc::new(ClientInner {
                key,
                options,
                transport,
                state: Mutex::new(state),
                task_queue: tokio::sync::Mutex::new(()),
                running_tasks: Arc::new(AtomicUsize::new(0)),
            }),
        }
    }

    /// Returns the client a background loop works for, if a handle to it is
    /// still alive.
    pub(crate) fn upgrade(inner: &Weak<ClientInner>) -> Option<Self> {
        inner.upgrade().map(|inner| Self { inner })
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

    /// Returns whether a document or channel with the given key is attached
    /// (JS `has`).
    pub fn has(&self, key: &str) -> bool {
        let state = self.state();
        state.attachments.contains_key(key) || state.channels.contains_key(key)
    }

    /// Returns how many background loops (the sync loop and watch loops) are
    /// running. For tests that check the loops stop.
    #[doc(hidden)]
    pub fn running_task_count(&self) -> usize {
        self.inner.running_tasks.load(Ordering::SeqCst)
    }

    /// Registers this client with the server and starts the sync loop (JS
    /// `activate`).
    ///
    /// With an auth token injector, it first asks the injector for a token
    /// and fails with the injector's error if it fails.
    pub async fn activate(&self) -> ClientResult<()> {
        if self.is_active() {
            return Ok(());
        }
        // Outside the task queue, as in JS.
        if let Some(injector) = &self.inner.options.auth_token_injector {
            let token = injector(None).await?;
            let mut state = self.state();
            state.auth_refresh.activated();
            self.inner.transport.set_auth_token(token);
        }

        let _queue = self.lock_task_queue().await;
        if self.is_active() {
            return Ok(());
        }

        let response = match self
            .inner
            .transport
            .activate_client(ActivateClientRequest {
                client_key: self.inner.key.clone(),
                metadata: self.inner.options.metadata.clone(),
                shard_key: self.shard_key(&self.inner.key),
            })
            .await
        {
            Ok(response) => response,
            Err(err) => return self.rethrow(err).await,
        };

        let mut state = self.state();
        state.actor_id = Some(
            response
                .actor_id
                .unwrap_or_else(|| response.client_id.clone()),
        );
        state.id = Some(response.client_id);
        state.status = ClientStatus::Activated;
        state.conditions.insert(ClientCondition::SyncLoop, true);
        // Spawned after the status is set, so the loop's first round sees an
        // active client.
        state.sync_loop = Some(sync_loop::spawn(&self.inner));
        Ok(())
    }

    /// Deactivates this client, stops its loops, and detaches every attached
    /// document and channel locally (JS `deactivate`).
    ///
    /// If the server refuses with `ErrUnauthenticated` and the auth token
    /// injector fails to give a new token, the client is deactivated locally
    /// anyway and the injector's error is returned: the app deactivates to
    /// stop the retries when the token can no longer be refreshed, and the
    /// server reaps the client later.
    pub async fn deactivate(&self, options: DeactivateOptions) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let client_id = {
            let state = self.state();
            if state.status == ClientStatus::Deactivated {
                return Ok(());
            }
            state.session(&self.inner.key)?.client_id
        };

        let result = self
            .inner
            .transport
            .deactivate_client(DeactivateClientRequest {
                client_id,
                synchronous: options.synchronous,
                shard_key: self.shard_key(&self.inner.key),
            })
            .await;
        if let Err(err) = result {
            if err.is_error_code(ErrorCode::ErrUnauthenticated)
                && self.inner.options.auth_token_injector.is_some()
            {
                if let Err(refresh_err) = self.refresh_auth_token(err.error_reason(), false).await {
                    self.deactivate_internal();
                    return Err(refresh_err);
                }
                return Err(err);
            }
            return self.rethrow(err).await;
        }

        self.deactivate_internal();
        Ok(())
    }

    /// Marks this client deactivated, stops its loops, and detaches every
    /// attached document and channel locally (JS `deactivateInternal`). It
    /// takes no task queue, so the loops can call it. Called from a loop, it
    /// aborts that loop's own task at its next `.await`, so the caller must
    /// return without awaiting anything after it.
    fn deactivate_internal(&self) {
        let (attachments, channels, sync_loop) = {
            let mut state = self.state();
            state.status = ClientStatus::Deactivated;
            state.conditions.insert(ClientCondition::SyncLoop, false);
            state.conditions.insert(ClientCondition::WatchLoop, false);
            (
                std::mem::take(&mut state.attachments),
                std::mem::take(&mut state.channels),
                state.sync_loop.take(),
            )
        };
        drop(sync_loop);
        for attachment in attachments.into_values() {
            let doc = release_attachment(attachment);
            lock_document(&doc).apply_status(DocStatus::Detached);
        }
        for attachment in channels.into_values() {
            release_channel_attachment(attachment);
        }
    }

    /// Attaches the document and returns it (JS `attach`).
    ///
    /// As in JS, the document is stamped with this client's actor and its
    /// initial presence is set before the attach request is sent. In a
    /// realtime mode it then opens the watch stream and waits until the
    /// stream is initialized or its first attempt fails.
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

        let _queue = self.lock_task_queue().await;
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
            doc.set_actor(session.actor_id.clone());
            doc.update_with_presence(|_, presence| {
                presence.set(initial_presence);
                Ok(())
            })?;
            doc.create_change_pack()
        };

        let response = match self
            .inner
            .transport
            .attach_document(AttachDocumentRequest {
                client_id: session.client_id.clone(),
                change_pack,
                schema_key: schema,
                shard_key: self.shard_key(&key),
            })
            .await
        {
            Ok(response) => response,
            Err(err) => return self.rethrow(err).await,
        };

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

        // The client may have been deactivated, or activated again as another
        // session, while the request was in flight.
        let watch_ready = {
            let mut state = self.state();
            if state.is_current(&session) {
                let mut attachment = Attachment::new(
                    doc.clone(),
                    response.document_id,
                    sync_mode,
                    poll_interval,
                    poll_interval_pinned,
                );
                let watch_ready = uses_watch_stream(sync_mode).then(|| {
                    let (task, ready) = watch::spawn(&self.inner, key.clone(), doc.clone());
                    attachment.watch_task = Some(task);
                    ready
                });
                state.attachments.insert(key.clone(), attachment);
                state.refresh_watch_loop_condition();
                Some(watch_ready)
            } else {
                None
            }
        };
        let Some(watch_ready) = watch_ready else {
            lock_document(&doc).apply_status(DocStatus::Detached);
            return Err(ClientError::ClientNotActivated(self.inner.key.clone()));
        };
        if let Some(ready) = watch_ready {
            // An error only means the watch task ended first.
            let _ = ready.await;
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
        let _queue = self.lock_task_queue().await;
        let session = self.session()?;
        let key = document_key(doc);
        let document_id = self.attached_document_id(&key, doc)?;

        let change_pack = {
            let mut doc = lock_document(doc);
            doc.update_with_presence(|_, presence| {
                presence.clear();
                Ok(())
            })?;
            doc.create_change_pack()
        };

        let response = match self
            .inner
            .transport
            .detach_document(DetachDocumentRequest {
                client_id: session.client_id,
                document_id,
                change_pack,
                remove_if_not_attached: false,
                shard_key: self.shard_key(&key),
            })
            .await
        {
            Ok(response) => response,
            Err(err) => return self.rethrow(err).await,
        };

        {
            let mut doc = lock_document(doc);
            doc.apply_change_pack(&response.change_pack)?;
            if doc.status() != DocStatus::Removed {
                doc.apply_status(DocStatus::Detached);
            }
        }
        self.remove_attachment(&key, doc);
        Ok(())
    }

    /// Pushes local changes and pulls remote ones (JS `sync`).
    ///
    /// With a document, it syncs that document in push-pull mode. With
    /// `None`, it syncs every attached document in its attachment's mode.
    pub async fn sync(&self, doc: Option<&SharedDocument>) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let session = self.session()?;
        let targets = match doc {
            Some(doc) => {
                let key = document_key(doc);
                let state = self.state();
                let attachment = state
                    .attachments
                    .get(&key)
                    .filter(|attachment| attachment.holds(doc))
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
            if let Err(err) = self.sync_document(&session.client_id, target).await {
                return self.rethrow(err).await;
            }
        }
        Ok(())
    }

    /// Removes the document on the server (JS `remove`).
    pub async fn remove(&self, doc: &SharedDocument) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let session = self.session()?;
        let key = document_key(doc);
        let document_id = self.attached_document_id(&key, doc)?;

        let change_pack = {
            let mut doc = lock_document(doc);
            doc.set_actor(session.actor_id);
            let mut change_pack = doc.create_change_pack();
            change_pack.set_removed(true);
            change_pack
        };

        let response = match self
            .inner
            .transport
            .remove_document(RemoveDocumentRequest {
                client_id: session.client_id,
                document_id,
                change_pack,
                shard_key: self.shard_key(&key),
            })
            .await
        {
            Ok(response) => response,
            Err(err) => return self.rethrow(err).await,
        };

        lock_document(doc).apply_change_pack(&response.change_pack)?;
        self.remove_attachment(&key, doc);
        Ok(())
    }

    /// Changes the sync mode of an attached document (JS `changeSyncMode`).
    ///
    /// Leaving the realtime modes for `Manual` or `Polling` closes the watch
    /// stream; coming back opens it again and waits for it as `attach` does.
    /// Switching to `Realtime` pulls at the next sync-loop tick. Switching to
    /// `RealtimePushOnly` or `RealtimeSyncOff` also covers a sync already in
    /// flight: the remote state of its reply is dropped, as in JS.
    pub async fn change_sync_mode(
        &self,
        doc: &SharedDocument,
        sync_mode: SyncMode,
    ) -> ClientResult<()> {
        let key = document_key(doc);
        // As in JS, the switch runs on the task queue, so it waits for a sync
        // that is already pulling. A pause must still cover that pull: the
        // caller has asked not to receive anything (an IME composition, for
        // example), so mark it now for the reply guard in `push_pull`. A
        // document that is not attached has no attachment to mark.
        let _pending_pause = matches!(
            sync_mode,
            SyncMode::RealtimePushOnly | SyncMode::RealtimeSyncOff
        )
        .then(|| {
            self.with_attachment(&key, doc, |attachment| {
                PendingPullPause::start(&attachment.pending_pull_pauses)
            })
        })
        .flatten();
        let _queue = self.lock_task_queue().await;
        let (stopped_watch, watch_ready) = {
            let mut state = self.state();
            state.session(&self.inner.key)?;
            let Some(attachment) = state
                .attachments
                .get_mut(&key)
                .filter(|attachment| attachment.holds(doc))
            else {
                return Err(ClientError::NotAttached(key));
            };

            let prev_sync_mode = attachment.sync_mode;
            if prev_sync_mode == sync_mode {
                return Ok(());
            }

            let stopped_watch = if uses_watch_stream(sync_mode) {
                None
            } else {
                attachment.watch_task.take()
            };
            attachment.sync_mode = sync_mode;
            // JS: the server sends no change events while the stream is
            // closed or ignored, so pull once on resuming realtime sync.
            if sync_mode == SyncMode::Realtime {
                attachment.change_event_received = true;
            }
            if !attachment.poll_interval_pinned {
                attachment.poll_interval = default_document_poll_interval(sync_mode);
            }
            let watch_ready = (!uses_watch_stream(prev_sync_mode) && uses_watch_stream(sync_mode))
                .then(|| {
                    let (task, ready) = watch::spawn(&self.inner, key.clone(), doc.clone());
                    attachment.watch_task = Some(task);
                    ready
                });
            state.refresh_watch_loop_condition();
            (stopped_watch, watch_ready)
        };

        if let Some(task) = stopped_watch {
            drop(task);
            watch::publish_disconnected(&mut lock_document(doc));
        }
        if let Some(ready) = watch_ready {
            // An error only means the watch task ended first.
            let _ = ready.await;
        }
        Ok(())
    }

    /// Attaches the channel and returns it (JS `attachChannel`).
    ///
    /// As in JS, this only registers the channel and makes sure the sync loop
    /// runs: the loop's next round sends the first refresh, which creates the
    /// session on the server and marks the channel attached. The client does
    /// not have to be active; if it is not, the first refresh activates it
    /// with the client ID the server issues.
    pub async fn attach_channel(
        &self,
        channel: SharedChannel,
        options: AttachChannelOptions,
    ) -> ClientResult<SharedChannel> {
        let key = {
            let channel = lock_channel(&channel);
            if channel.status() != ChannelStatus::Detached {
                return Err(ClientError::NotDetached(channel.key().to_owned()));
            }
            channel.key().to_owned()
        };
        let sync_mode = options.sync_mode.unwrap_or(SyncMode::Realtime);
        validate_channel_sync_mode(sync_mode)?;
        let (heartbeat_interval, heartbeat_interval_pinned) =
            match options.channel_heartbeat_interval {
                Some(interval) if interval.is_zero() => {
                    return Err(ClientError::InvalidArgument(
                        "channel_heartbeat_interval must be greater than 0".to_owned(),
                    ));
                }
                Some(interval) => (interval, true),
                None => (self.inner.options.channel_heartbeat_interval, false),
            };

        let _queue = self.lock_task_queue().await;
        let client_id = {
            let mut state = self.state();
            if state.is_taken(&key) {
                return Err(ClientError::AlreadyAttached(key));
            }
            state.channels.insert(
                key.clone(),
                ChannelAttachment::new(
                    channel.clone(),
                    sync_mode,
                    heartbeat_interval,
                    heartbeat_interval_pinned,
                ),
            );
            // The handle check backs up the condition: a loop that ended
            // without clearing it must not leave the channel unrefreshed.
            let loop_running = state.conditions[&ClientCondition::SyncLoop]
                && state
                    .sync_loop
                    .as_ref()
                    .is_some_and(|task| !task.is_finished());
            if !loop_running {
                state.conditions.insert(ClientCondition::SyncLoop, true);
                state.sync_loop = Some(sync_loop::spawn(&self.inner));
            }
            state.id.clone()
        };
        let local_broadcast = {
            let sender = send_local_broadcasts(&self.inner, key.clone(), Arc::downgrade(&channel));
            let mut channel = lock_channel(&channel);
            // JS: a client that is already active gives the channel its ID
            // now; otherwise the first refresh does.
            if let Some(client_id) = client_id {
                channel.set_actor(client_id);
            }
            channel.subscribe(sender)
        };
        self.with_channel_attachment(&key, &channel, |attachment| {
            attachment.local_broadcast = Some(local_broadcast);
        });
        Ok(channel)
    }

    /// Detaches the channel (JS `detachChannel`).
    ///
    /// As in JS this is local: the client stops refreshing the session, and
    /// the server drops it when its TTL runs out, so peers see the session
    /// count fall only then.
    pub async fn detach_channel(&self, channel: &SharedChannel) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let key = channel_key(channel);
        if self.remove_channel_attachment(&key, channel) {
            Ok(())
        } else {
            Err(ClientError::NotAttached(key))
        }
    }

    /// Refreshes the session of an attached channel now (JS `sync(channel)`).
    /// A `Manual` channel is refreshed only this way.
    pub async fn sync_channel(&self, channel: &SharedChannel) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let key = channel_key(channel);
        if self
            .with_channel_attachment(&key, channel, |_| ())
            .is_none()
        {
            return Err(ClientError::NotAttached(key));
        }
        match self.refresh_channel(key, channel.clone()).await {
            Ok(()) => Ok(()),
            Err(err) => self.rethrow(err).await,
        }
    }

    /// Changes the sync mode of an attached channel (JS
    /// `changeChannelSyncMode`). Channels take `Manual`, `Realtime`, and
    /// `Polling`; an unpinned heartbeat interval becomes zero in `Manual` and
    /// the client's interval otherwise.
    pub async fn change_channel_sync_mode(
        &self,
        channel: &SharedChannel,
        sync_mode: SyncMode,
    ) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let key = channel_key(channel);
        let (stopped_watch, watch_ready) = {
            let mut state = self.state();
            let active = state.session(&self.inner.key).is_ok();
            let Some(attachment) = state
                .channels
                .get_mut(&key)
                .filter(|attachment| attachment.holds(channel))
            else {
                return Err(ClientError::NotAttached(key));
            };
            if attachment.sync_mode == sync_mode {
                return Ok(());
            }
            validate_channel_sync_mode(sync_mode)?;

            let stopped_watch = attachment.watch_task.take();
            attachment.sync_mode = sync_mode;
            if !attachment.heartbeat_interval_pinned {
                attachment.heartbeat_interval = if sync_mode == SyncMode::Manual {
                    Duration::ZERO
                } else {
                    self.inner.options.channel_heartbeat_interval
                };
            }
            // Only `Realtime` watches. As in JS, a client that is not active
            // yet opens the stream after its first refresh instead (JS
            // `runWatchLoop` swallows the `ErrClientNotActivated`).
            let watch_ready = (sync_mode == SyncMode::Realtime && active).then(|| {
                let (task, ready) = watch::spawn_channel(&self.inner, key.clone(), channel.clone());
                attachment.watch_task = Some(task);
                ready
            });
            state.refresh_watch_loop_condition();
            (stopped_watch, watch_ready)
        };
        drop(stopped_watch);
        if let Some(ready) = watch_ready {
            // An error only means the watch task ended first.
            let _ = ready.await;
        }
        Ok(())
    }

    /// Returns a channel's session count without joining it (JS
    /// `peekChannel`). It needs no activation, adds no session, and gets no
    /// events; callers poll it themselves. As in JS, an error is returned as
    /// is: it does not go through `handleConnectError`, so an
    /// `ErrUnauthenticated` does not refresh the auth token.
    pub async fn peek_channel(&self, channel_key: &str) -> ClientResult<u64> {
        let _queue = self.lock_task_queue().await;
        let response = self
            .inner
            .transport
            .peek_channel(PeekChannelRequest {
                channel_key: channel_key.to_owned(),
                shard_key: self.shard_key(first_key_path(channel_key)),
            })
            .await?;
        Ok(response.session_count)
    }

    /// Broadcasts `payload` under `topic` to the other clients watching the
    /// channel attached under `key` (JS `client.broadcast`), sending its JSON
    /// encoding (JS `JSON.stringify`).
    ///
    /// A failure that JS `handleConnectError` retries is retried after 1s,
    /// 2s, 4s, and so on up to 20s (or until the next auth token refresh may
    /// run, at most 5s, after `ErrUnauthenticated`, which also publishes
    /// `ChannelEvent::AuthError`), at most `options.max_retries` times,
    /// and the call returns when the broadcast is sent or fails for good.
    /// Each try waits for the task queue; the waits between tries do not
    /// hold it. Unlike JS, which checks the attachment once, every try
    /// checks it, so detaching the channel ends the retries. `options.error`
    /// is not called here: the error is returned.
    pub async fn broadcast(
        &self,
        key: &str,
        topic: &str,
        payload: serde_json::Value,
        options: BroadcastOptions,
    ) -> ClientResult<()> {
        broadcast_with_retry(
            &Arc::downgrade(&self.inner),
            key,
            None,
            topic,
            &payload,
            options.max_retries,
        )
        .await
    }

    /// Sends one broadcast (one try of JS `broadcast`'s `doLoop`). With
    /// `channel`, the broadcast is that channel's `LocalBroadcast`, and only
    /// goes out while that channel, not just any channel of `key`, is
    /// attached.
    async fn broadcast_once(
        &self,
        key: &str,
        channel: Option<&Weak<Mutex<Channel>>>,
        topic: &str,
        payload: Vec<u8>,
    ) -> ClientResult<()> {
        let _queue = self.lock_task_queue().await;
        let client_id = {
            let state = self.state();
            let attached = match channel {
                None => state.channels.contains_key(key),
                Some(channel) => channel.upgrade().is_some_and(|channel| {
                    state
                        .channels
                        .get(key)
                        .is_some_and(|attachment| attachment.holds(&channel))
                }),
            };
            if !attached {
                return Err(ClientError::NotAttached(key.to_owned()));
            }
            state
                .id
                .clone()
                .ok_or_else(|| ClientError::ClientNotActivated(self.inner.key.clone()))?
        };
        self.inner
            .transport
            .broadcast(BroadcastRequest {
                client_id,
                channel_key: key.to_owned(),
                topic: topic.to_owned(),
                payload,
                shard_key: self.shard_key(first_key_path(key)),
            })
            .await?;
        Ok(())
    }

    /// Creates or refreshes the session of an attached channel and takes the
    /// session count the server reports (JS `syncInternal` for a channel).
    ///
    /// A channel without a session sends a first refresh with the client key
    /// and metadata. Its reply carries the session ID, which marks the
    /// channel attached, and a client ID, which the client adopts if it has
    /// none yet, becoming active. A refresh of a session the server no longer
    /// has (`ErrSessionNotFound`) clears the session, so the next round
    /// creates a new one; any other failure is published as
    /// `ChannelEvent::SyncError` and returned.
    pub(crate) async fn refresh_channel(
        &self,
        key: String,
        channel: SharedChannel,
    ) -> ClientResult<()> {
        let session_id = lock_channel(&channel).session_id().map(str::to_owned);
        let first_call = session_id.is_none();
        let client_id = self.state().id.clone();
        let (client_key, metadata) = if first_call {
            (self.inner.key.clone(), self.inner.options.metadata.clone())
        } else {
            (String::new(), BTreeMap::new())
        };
        let result = self
            .inner
            .transport
            .refresh_channel(RefreshChannelRequest {
                client_id,
                channel_key: key.clone(),
                session_id,
                client_key,
                metadata,
                shard_key: self.shard_key(first_key_path(&key)),
            })
            .await;

        let response = match result {
            Ok(response) => response,
            Err(err) => {
                if self
                    .with_channel_attachment(&key, &channel, |_| ())
                    .is_none()
                {
                    return Err(err);
                }
                let mut channel = lock_channel(&channel);
                if err.is_error_code(ErrorCode::ErrSessionNotFound) {
                    channel.set_session_id(None);
                    return Ok(());
                }
                channel.publish(ChannelEvent::SyncError {
                    error: err.clone(),
                    method: "RefreshChannel".to_owned(),
                });
                return Err(err);
            }
        };

        // The channel may have been detached, or the client deactivated,
        // while the request was in flight.
        let actor_id = {
            let mut state = self.state();
            if !state
                .channels
                .get(&key)
                .is_some_and(|attachment| attachment.holds(&channel))
            {
                return Ok(());
            }
            if first_call && state.id.is_none() {
                if let Some(client_id) = &response.client_id {
                    state.id = Some(client_id.clone());
                    // JS: channel activation carries no stable actor.
                    state.actor_id = Some(client_id.clone());
                    state.status = ClientStatus::Activated;
                }
            }
            let active = state.session(&self.inner.key).is_ok();
            let actor_id = state.id.clone();
            if let Some(attachment) = state.channels.get_mut(&key) {
                attachment.last_heartbeat_at = Some(Instant::now());
                // JS: a realtime channel opens its stream once the first
                // refresh has given the client an ID; it is not awaited.
                // A stream that stopped is opened again, as JS
                // `runWatchLoop` does when it has none.
                if first_call
                    && active
                    && attachment.sync_mode == SyncMode::Realtime
                    && attachment
                        .watch_task
                        .as_ref()
                        .is_none_or(TaskHandle::is_finished)
                {
                    let (task, _ready) =
                        watch::spawn_channel(&self.inner, key.clone(), channel.clone());
                    attachment.watch_task = Some(task);
                    state.conditions.insert(ClientCondition::WatchLoop, true);
                }
            }
            actor_id
        };

        let mut channel = lock_channel(&channel);
        if first_call {
            if let Some(actor_id) = actor_id {
                channel.set_actor(actor_id);
            }
            // JS: without a session ID the channel stays detached, and the
            // next round tries the first refresh again.
            if let Some(session_id) = response.session_id {
                channel.set_session_id(Some(session_id));
                channel.apply_status(ChannelStatus::Attached);
            }
        }
        let prev_count = channel.session_count();
        if channel.update_session_count(response.session_count, 0)
            && channel.session_count() != prev_count
        {
            let count = channel.session_count();
            channel.publish(ChannelEvent::PresenceChanged { count });
        }
        Ok(())
    }

    /// Pushes and pulls one document and publishes the sync status (JS
    /// `syncInternal`).
    pub(crate) async fn sync_document(
        &self,
        client_id: &ActorId,
        target: SyncTarget,
    ) -> ClientResult<()> {
        let result = self.push_pull(client_id, &target).await;
        let status = match result {
            Ok(_) => DocSyncStatus::Synced,
            Err(_) => DocSyncStatus::SyncFailed,
        };
        lock_document(&target.doc).publish(DocEvent::SyncStatusChanged(status));

        let removed = result?;
        if removed {
            self.remove_attachment(&target.key, &target.doc);
        } else {
            self.with_attachment(&target.key, &target.doc, |attachment| {
                attachment.last_synced_at = Some(Instant::now());
                // JS `clearWriteRejected`: a push went through, so the sync
                // loop takes this document again. An explicit `sync` is the
                // way back for a document parked by a size rejection.
                attachment.write_rejected = false;
            });
        }
        Ok(())
    }

    /// Sends one push-pull request and applies the response. Returns whether
    /// the response removed the document.
    async fn push_pull(&self, client_id: &ActorId, target: &SyncTarget) -> ClientResult<bool> {
        let change_pack = lock_document(&target.doc).create_change_pack();
        let response = self
            .inner
            .transport
            .push_pull_changes(PushPullChangesRequest {
                client_id: client_id.clone(),
                document_id: target.document_id.clone(),
                change_pack,
                push_only: target.request_mode == SyncMode::RealtimePushOnly,
                shard_key: self.shard_key(&target.key),
            })
            .await?;

        // As in JS `syncInternal`: while the document is in PushOnly or
        // SyncOff, or the request was sent push-only, the reply is taken as
        // a push acknowledgement only. Its remote state, a snapshot included,
        // is dropped and pulled again once realtime sync resumes. The content
        // does not enter the decision: even a reply without changes carries
        // the server's minimum version vector, and collecting garbage with it
        // purges tombstones that the changes this document has not pulled yet
        // anchor on.
        let mode_drops_remote_state = target.request_mode == SyncMode::RealtimePushOnly
            || matches!(
                target.attachment_mode,
                SyncMode::RealtimePushOnly | SyncMode::RealtimeSyncOff
            );
        // A pause asked for but still waiting on the task queue counts as
        // well (JS `pendingPullPauses`): this may be the very sync it waits
        // behind. Not for a `Manual` document, where nothing would pull the
        // dropped state again. In `Realtime` the sync loop cleared
        // `change_event_received` before this sync, so set it again: a switch
        // that never lands would otherwise leave the dropped state unpulled.
        let pause_pending = !mode_drops_remote_state
            && self
                .with_attachment(&target.key, &target.doc, |attachment| {
                    let pending = attachment.pending_pull_pauses.load(Ordering::SeqCst) > 0
                        && attachment.sync_mode != SyncMode::Manual;
                    if pending {
                        attachment.change_event_received = true;
                    }
                    pending
                })
                .unwrap_or(false);
        let drops_remote_state = mode_drops_remote_state || pause_pending;

        let mut doc = lock_document(&target.doc);
        if drops_remote_state {
            doc.acknowledge_pushed_changes(&response.change_pack);
        } else {
            doc.apply_change_pack(&response.change_pack)?;
        }
        Ok(doc.status() == DocStatus::Removed)
    }

    /// Handles a failed server call and returns whether to retry it (JS
    /// `handleConnectError`); see [`ClientError::handling`] for the rules.
    ///
    /// `ErrUnauthenticated` asks the auth token injector for a new token,
    /// passing the server's reason. `in_retry_loop` is true for the loops
    /// that retry on their own (the sync loop, the watch loops, and broadcast
    /// retries): their refusals skip the injector while its backoff runs, and
    /// an injector failure is not returned. A one-shot call always calls the
    /// injector and gets its error. `ErrClientNotActivated` and
    /// `ErrClientNotFound` deactivate the client.
    pub(crate) async fn handle_connect_error(
        &self,
        err: &ClientError,
        in_retry_loop: bool,
    ) -> ClientResult<bool> {
        match err.handling() {
            ErrorHandling::Retry => Ok(true),
            ErrorHandling::RefreshToken => {
                self.refresh_auth_token(err.error_reason(), in_retry_loop)
                    .await?;
                Ok(true)
            }
            ErrorHandling::Deactivate => {
                self.deactivate_internal();
                Ok(false)
            }
            ErrorHandling::Stop => Ok(false),
        }
    }

    /// Handles the error of a one-shot call as JS does in its `catch`, and
    /// returns it, or the auth token injector's error if that failed.
    async fn rethrow<T>(&self, err: ClientError) -> ClientResult<T> {
        self.handle_connect_error(&err, false).await?;
        Err(err)
    }

    /// Asks the auth token injector, if there is one, for a new token after
    /// a refusal and sets it on the transport (JS `refreshAuthToken`). See
    /// [`crate::auth`] for which calls are made and which tokens are kept.
    async fn refresh_auth_token(
        &self,
        reason: Option<&str>,
        in_retry_loop: bool,
    ) -> ClientResult<()> {
        let Some(injector) = &self.inner.options.auth_token_injector else {
            return Ok(());
        };
        let Some(call) = self
            .state()
            .auth_refresh
            .begin(in_retry_loop, tokio::time::Instant::now())
        else {
            return Ok(());
        };
        match injector(reason.map(str::to_owned)).await {
            Ok(token) => {
                // Under the state lock, so a newer token is never overwritten.
                let mut state = self.state();
                if state.auth_refresh.succeeded(call) {
                    self.inner.transport.set_auth_token(token);
                }
                Ok(())
            }
            Err(err) => {
                self.state()
                    .auth_refresh
                    .failed(call, tokio::time::Instant::now(), random_unit());
                if in_retry_loop {
                    Ok(())
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Returns how long a loop waits to retry after `err`: `delay`, or for
    /// `ErrUnauthenticated`, until the next token refresh may run if that is
    /// later, but at most 5s (JS `loopRetryDelay`). Other errors keep their
    /// delay: a method the auth webhook does not guard still works with the
    /// refused token.
    pub(crate) fn loop_retry_delay(&self, err: &ClientError, delay: Duration) -> Duration {
        if !err.is_error_code(ErrorCode::ErrUnauthenticated) {
            return delay;
        }
        self.state()
            .auth_refresh
            .loop_retry_delay(delay, tokio::time::Instant::now())
    }

    /// Waits for the task queue, which serializes client calls and sync-loop
    /// rounds (JS `enqueueTask`).
    pub(crate) async fn lock_task_queue(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.task_queue.lock().await
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, ClientState> {
        self.inner.state()
    }

    pub(crate) fn session(&self) -> ClientResult<Session> {
        self.state().session(&self.inner.key)
    }

    pub(crate) fn set_condition(&self, condition: ClientCondition, value: bool) {
        self.state().conditions.insert(condition, value);
    }

    /// Runs `f` on the attachment of `key` if it still holds `doc`.
    pub(crate) fn with_attachment<R>(
        &self,
        key: &str,
        doc: &SharedDocument,
        f: impl FnOnce(&mut Attachment) -> R,
    ) -> Option<R> {
        self.state()
            .attachments
            .get_mut(key)
            .filter(|attachment| attachment.holds(doc))
            .map(f)
    }

    pub(crate) fn shard_key(&self, resource_key: &str) -> String {
        format!("{}/{}", self.inner.options.api_key, resource_key)
    }

    /// Runs `f` on the channel attachment of `key` if it still holds
    /// `channel`.
    pub(crate) fn with_channel_attachment<R>(
        &self,
        key: &str,
        channel: &SharedChannel,
        f: impl FnOnce(&mut ChannelAttachment) -> R,
    ) -> Option<R> {
        self.state()
            .channels
            .get_mut(key)
            .filter(|attachment| attachment.holds(channel))
            .map(f)
    }

    /// Drops the channel attachment of `key` if it still holds `channel`
    /// (JS `detachInternal`), and returns whether it did.
    fn remove_channel_attachment(&self, key: &str, channel: &SharedChannel) -> bool {
        let attachment = {
            let mut state = self.state();
            if !state
                .channels
                .get(key)
                .is_some_and(|attachment| attachment.holds(channel))
            {
                return false;
            }
            let attachment = state.channels.remove(key);
            state.refresh_watch_loop_condition();
            attachment
        };
        if let Some(attachment) = attachment {
            release_channel_attachment(attachment);
        }
        true
    }

    fn attached_document_id(&self, key: &str, doc: &SharedDocument) -> ClientResult<String> {
        self.with_attachment(key, doc, |attachment| attachment.resource_id.clone())
            .ok_or_else(|| ClientError::NotAttached(key.to_owned()))
    }

    /// Drops the attachment of `key` if it still holds `doc`, which stops its
    /// watch stream (JS `detachInternal`). The only place attachments are
    /// removed one at a time.
    fn remove_attachment(&self, key: &str, doc: &SharedDocument) {
        let attachment = {
            let mut state = self.state();
            if !state
                .attachments
                .get(key)
                .is_some_and(|attachment| attachment.holds(doc))
            {
                return;
            }
            let attachment = state.attachments.remove(key);
            state.refresh_watch_loop_condition();
            attachment
        };
        if let Some(attachment) = attachment {
            release_attachment(attachment);
        }
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

    /// Spawns a background loop that counts as running until it ends or its
    /// handle is dropped.
    pub(crate) fn spawn_task(&self, task: impl Future<Output = ()> + Send + 'static) -> TaskHandle {
        let running = RunningTask::start(&self.running_tasks);
        let handle = tokio::spawn(async move {
            let _running = running;
            task.await;
        });
        TaskHandle(handle.abort_handle())
    }
}

impl ClientState {
    pub(crate) fn session(&self, client_key: &str) -> ClientResult<Session> {
        match (&self.status, &self.id, &self.actor_id) {
            (ClientStatus::Activated, Some(client_id), Some(actor_id)) => Ok(Session {
                client_id: client_id.clone(),
                actor_id: actor_id.clone(),
            }),
            _ => Err(ClientError::ClientNotActivated(client_key.to_owned())),
        }
    }

    /// Whether a document or channel with the key is attached or attaching.
    fn is_taken(&self, key: &str) -> bool {
        self.attachments.contains_key(key)
            || self.channels.contains_key(key)
            || self.attaching.contains(key)
    }

    /// Whether the client is still active in the given session.
    fn is_current(&self, session: &Session) -> bool {
        self.status == ClientStatus::Activated && self.id.as_ref() == Some(&session.client_id)
    }

    fn refresh_watch_loop_condition(&mut self) {
        let has_watch_attachment = self
            .attachments
            .values()
            .any(|attachment| uses_watch_stream(attachment.sync_mode))
            || self
                .channels
                .values()
                .any(|attachment| attachment.sync_mode == SyncMode::Realtime);
        self.conditions
            .insert(ClientCondition::WatchLoop, has_watch_attachment);
    }
}

impl SyncTarget {
    pub(crate) fn new(key: String, attachment: &Attachment, request_mode: SyncMode) -> Self {
        Self {
            key,
            doc: attachment.doc.clone(),
            document_id: attachment.resource_id.clone(),
            request_mode,
            attachment_mode: attachment.sync_mode,
        }
    }
}

/// Counts one pending pause on an attachment for the lifetime of one
/// `change_sync_mode` call (JS `pendingPullPauses` increment and the
/// decrement in its `finally`).
struct PendingPullPause(Arc<AtomicUsize>);

impl PendingPullPause {
    fn start(pauses: &Arc<AtomicUsize>) -> Self {
        pauses.fetch_add(1, Ordering::SeqCst);
        Self(pauses.clone())
    }
}

impl Drop for PendingPullPause {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
        if state.is_taken(key) {
            return Err(ClientError::AlreadyAttached(key.to_owned()));
        }
        state.attaching.insert(key.to_owned());
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

/// Drops a removed attachment, stopping its watch stream, and clears the
/// document's online clients (JS `detachInternal`). A document that was
/// watched also gets the events JS publishes when its stream closes. Must be
/// called without the client state lock.
fn release_attachment(attachment: Attachment) -> SharedDocument {
    let Attachment {
        doc, watch_task, ..
    } = attachment;
    let watched = watch_task.is_some();
    drop(watch_task);
    {
        let mut doc = lock_document(&doc);
        if watched {
            watch::publish_disconnected(&mut doc);
        } else {
            doc.reset_online_clients();
        }
    }
    doc
}

/// Drops a removed channel attachment, stopping its watch stream, and marks
/// the channel detached (JS `detachInternal` and `applyStatus(Detached)`).
/// The session ID is cleared too, so attaching the channel again starts a
/// new session. Must be called without the client state lock.
fn release_channel_attachment(attachment: ChannelAttachment) {
    let ChannelAttachment {
        channel,
        watch_task,
        local_broadcast,
        ..
    } = attachment;
    drop(watch_task);
    let mut channel = lock_channel(&channel);
    if let Some(id) = local_broadcast {
        channel.unsubscribe(id);
    }
    channel.apply_status(ChannelStatus::Detached);
    channel.set_session_id(None);
}

/// Returns the channel subscriber that sends each `LocalBroadcast` in the
/// background (the JS `local-broadcast` handler of `attachChannel`), passing
/// a final failure to the broadcast's error handler. It holds the client
/// weakly, so a channel the app keeps does not keep the client alive, and
/// the channel weakly, since the channel holds it.
fn send_local_broadcasts(
    inner: &Arc<ClientInner>,
    key: String,
    channel: Weak<Mutex<Channel>>,
) -> impl FnMut(&ChannelEvent) + Send + 'static {
    let inner = Arc::downgrade(inner);
    let runtime = tokio::runtime::Handle::current();
    move |event| {
        let ChannelEvent::LocalBroadcast {
            topic,
            payload,
            options,
            ..
        } = event
        else {
            return;
        };
        let (inner, key, channel, topic, payload, options) = (
            inner.clone(),
            key.clone(),
            channel.clone(),
            topic.clone(),
            payload.clone(),
            options.clone(),
        );
        runtime.spawn(async move {
            let result = broadcast_with_retry(
                &inner,
                &key,
                Some(&channel),
                &topic,
                &payload,
                options.max_retries,
            )
            .await;
            if let (Err(err), Some(handler)) = (result, &options.error) {
                handler.call(&err);
            }
        });
    }
}

/// Sends a broadcast, retrying after retryable failures (JS `broadcast`).
/// See `Client::broadcast`, and `Client::broadcast_once` for `channel`.
async fn broadcast_with_retry(
    inner: &Weak<ClientInner>,
    key: &str,
    channel: Option<&Weak<Mutex<Channel>>>,
    topic: &str,
    payload: &serde_json::Value,
    max_retries: Option<u32>,
) -> ClientResult<()> {
    let payload =
        serde_json::to_vec(payload).map_err(|err| ClientError::InvalidArgument(err.to_string()))?;
    let mut retries = 0;
    loop {
        let Some(client) = Client::upgrade(inner) else {
            return Err(ClientError::NotAttached(key.to_owned()));
        };
        let err = match client
            .broadcast_once(key, channel, topic, payload.clone())
            .await
        {
            Ok(()) => return Ok(()),
            Err(err) => err,
        };
        // Only the retries are a loop: the first try is the app's call.
        if !client.handle_connect_error(&err, retries > 0).await? {
            return Err(err);
        }
        if err.is_error_code(ErrorCode::ErrUnauthenticated) {
            let channel = client
                .state()
                .channels
                .get(key)
                .map(|attachment| attachment.channel.clone());
            if let Some(channel) = channel {
                lock_channel(&channel).publish(ChannelEvent::AuthError {
                    reason: err.error_reason().unwrap_or("unauthenticated").to_owned(),
                    method: "Broadcast".to_owned(),
                });
            }
        }
        if max_retries.is_some_and(|max| retries >= max) {
            return Err(err);
        }
        let delay = client.loop_retry_delay(&err, broadcast_retry_delay(retries));
        drop(client);
        tokio::time::sleep(delay).await;
        retries += 1;
    }
}

/// The wait before broadcast retry `retries + 1` (JS `exponentialBackoff`).
fn broadcast_retry_delay(retries: u32) -> Duration {
    BROADCAST_INITIAL_RETRY_INTERVAL
        .saturating_mul(2u32.saturating_pow(retries))
        .min(BROADCAST_MAX_BACKOFF)
}

pub(crate) fn lock_document(doc: &SharedDocument) -> MutexGuard<'_, Document> {
    doc.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn lock_channel(channel: &SharedChannel) -> MutexGuard<'_, Channel> {
    channel.lock().unwrap_or_else(PoisonError::into_inner)
}

fn document_key(doc: &SharedDocument) -> String {
    lock_document(doc).key().to_owned()
}

fn channel_key(channel: &SharedChannel) -> String {
    lock_channel(channel).key().to_owned()
}

/// Generates the key of a client created without one: a random (version 4)
/// UUID drawn from the OS CSPRNG, as JS `uuid()` (`util/uuid.ts`) does with
/// Web Crypto. The key must be unguessable, not merely unique: the server
/// derives the actor from it, so a predictable key would let another client
/// of the project activate as the same actor.
///
/// Panics if the OS has no random source. JS refuses to generate a key there
/// too (it throws `ErrInvalidArgument`); such a runtime must pass its own
/// `ClientOptions::key`.
fn generate_client_key() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)
        .expect("the OS random source must be available to generate a client key");
    // Version 4 (random) and RFC 4122 variant bits.
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        hex(&b[0..4]),
        hex(&b[4..6]),
        hex(&b[6..8]),
        hex(&b[8..10]),
        hex(&b[10..16])
    )
}

#[cfg(test)]
mod tests {
    use super::{broadcast_retry_delay, Client, ClientCondition, ClientStatus};
    use crate::attachment::SharedDocument;
    use crate::channel::{
        BroadcastErrorHandler, BroadcastOptions, Channel, ChannelEvent, ChannelStatus,
        SharedChannel,
    };
    use crate::error::{ClientError, ClientResult};
    use crate::options::{
        AttachChannelOptions, AttachOptions, AuthTokenInjector, ClientOptions, DeactivateOptions,
        DetachOptions, SyncMode, DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS,
        DEFAULT_POLLING_INTERVAL_MS, DEFAULT_RECONNECT_STREAM_DELAY_MS,
        DEFAULT_RETRY_SYNC_LOOP_DELAY_MS, DEFAULT_RPC_ADDR, DEFAULT_SYNC_LOOP_DURATION_MS,
    };
    use crate::transport::{
        ActivateClientRequest, ActivateClientResponse, AttachDocumentRequest,
        AttachDocumentResponse, BroadcastRequest, BroadcastResponse, ChannelWatchStream,
        ClientTransport, DeactivateClientRequest, DeactivateClientResponse, DetachDocumentRequest,
        DetachDocumentResponse, PeekChannelRequest, PeekChannelResponse, PushPullChangesRequest,
        PushPullChangesResponse, RefreshChannelRequest, RefreshChannelResponse,
        RemoveDocumentRequest, RemoveDocumentResponse, WatchChannelEvent, WatchChannelEventType,
        WatchChannelRequest, WatchChannelResponse, WatchDocumentRequest, WatchDocumentResponse,
        WatchStream,
    };
    use async_trait::async_trait;
    use futures_util::stream::{self, StreamExt};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use yorkie_core::{
        ActorId, DocEvent, DocStatus, DocSyncStatus, Document, ErrorCode, JsonObject, Presence,
        SchemaRule, StreamConnectionStatus, TreeNodeRule, YorkieError,
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
        watch_requests: Mutex<Vec<WatchDocumentRequest>>,
        /// Messages for the next watch streams, one list per stream. A stream
        /// stays open after its messages; once these run out, streams report
        /// no other watchers.
        watch_scripts: Mutex<Vec<Vec<ClientResult<WatchDocumentResponse>>>>,
        attach_max_size_per_document: usize,
        attach_schema_rules: Vec<SchemaRule>,
        refresh_requests: Mutex<Vec<RefreshChannelRequest>>,
        /// Errors for the next refreshes, in order; later refreshes succeed.
        refresh_errors: Mutex<Vec<ClientError>>,
        /// When set, the next refresh waits for it before answering.
        refresh_gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
        peek_requests: Mutex<Vec<PeekChannelRequest>>,
        channel_watch_requests: Mutex<Vec<WatchChannelRequest>>,
        /// Messages for the next channel watch streams, one list per stream;
        /// once these run out, streams report one session.
        channel_watch_scripts: Mutex<Vec<Vec<ClientResult<WatchChannelResponse>>>>,
        broadcast_requests: Mutex<Vec<BroadcastRequest>>,
        /// Errors for the next broadcasts, in order; later ones succeed.
        broadcast_errors: Mutex<Vec<ClientError>>,
        /// Errors for the next deactivations, in order; later ones succeed.
        deactivate_errors: Mutex<Vec<ClientError>>,
        /// Errors for the next push-pulls of a document key, in order; the
        /// first one for the key is used.
        push_pull_errors: Mutex<Vec<(String, ClientError)>>,
        /// The tokens set on the transport, in order.
        auth_tokens: Mutex<Vec<String>>,
        /// The token in use at each push-pull.
        push_pull_tokens: Mutex<Vec<Option<String>>>,
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
                watch_requests: Mutex::new(Vec::new()),
                watch_scripts: Mutex::new(Vec::new()),
                attach_max_size_per_document: 0,
                attach_schema_rules: Vec::new(),
                refresh_requests: Mutex::new(Vec::new()),
                refresh_errors: Mutex::new(Vec::new()),
                refresh_gate: Mutex::new(None),
                peek_requests: Mutex::new(Vec::new()),
                channel_watch_requests: Mutex::new(Vec::new()),
                channel_watch_scripts: Mutex::new(Vec::new()),
                broadcast_requests: Mutex::new(Vec::new()),
                broadcast_errors: Mutex::new(Vec::new()),
                deactivate_errors: Mutex::new(Vec::new()),
                push_pull_errors: Mutex::new(Vec::new()),
                auth_tokens: Mutex::new(Vec::new()),
                push_pull_tokens: Mutex::new(Vec::new()),
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
            let mut errors = self.deactivate_errors.lock().unwrap();
            if !errors.is_empty() {
                return Err(errors.remove(0));
            }
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
            let key = change_pack.document_key().to_owned();
            self.push_pull_requests.lock().unwrap().push(request);
            let token = self.auth_tokens.lock().unwrap().last().cloned();
            self.push_pull_tokens.lock().unwrap().push(token);
            let mut errors = self.push_pull_errors.lock().unwrap();
            if let Some(index) = errors.iter().position(|(error_key, _)| *error_key == key) {
                return Err(errors.remove(index).1);
            }
            Ok(PushPullChangesResponse { change_pack })
        }

        async fn watch_document(&self, request: WatchDocumentRequest) -> ClientResult<WatchStream> {
            self.watch_requests.lock().unwrap().push(request);
            let mut scripts = self.watch_scripts.lock().unwrap();
            let messages = if scripts.is_empty() {
                vec![Ok(WatchDocumentResponse::Initialization {
                    client_ids: Vec::new(),
                    heartbeat_interval_ms: 0,
                })]
            } else {
                scripts.remove(0)
            };
            Ok(Box::pin(stream::iter(messages).chain(stream::pending())))
        }

        /// Issues `session-<n>` on the n-th first refresh, and this
        /// transport's client ID to a client that has none.
        async fn refresh_channel(
            &self,
            request: RefreshChannelRequest,
        ) -> ClientResult<RefreshChannelResponse> {
            let first_call = request.session_id.is_none();
            let needs_client_id = request.client_id.is_none();
            self.refresh_requests.lock().unwrap().push(request);
            let gate = self.refresh_gate.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let requests = self.refresh_requests.lock().unwrap();
            let mut errors = self.refresh_errors.lock().unwrap();
            if !errors.is_empty() {
                return Err(errors.remove(0));
            }
            let sessions = requests
                .iter()
                .filter(|request| request.session_id.is_none())
                .count();
            Ok(RefreshChannelResponse {
                session_count: 1,
                client_id: (first_call && needs_client_id).then(|| self.client_id.clone()),
                session_id: first_call.then(|| format!("session-{sessions}")),
            })
        }

        async fn peek_channel(
            &self,
            request: PeekChannelRequest,
        ) -> ClientResult<PeekChannelResponse> {
            self.peek_requests.lock().unwrap().push(request);
            Ok(PeekChannelResponse { session_count: 2 })
        }

        async fn watch_channel(
            &self,
            request: WatchChannelRequest,
        ) -> ClientResult<ChannelWatchStream> {
            self.channel_watch_requests.lock().unwrap().push(request);
            let mut scripts = self.channel_watch_scripts.lock().unwrap();
            let messages = if scripts.is_empty() {
                vec![Ok(WatchChannelResponse::Initialization {
                    session_count: 1,
                    seq: 1,
                    heartbeat_interval_ms: 0,
                })]
            } else {
                scripts.remove(0)
            };
            Ok(Box::pin(stream::iter(messages).chain(stream::pending())))
        }

        async fn broadcast(&self, request: BroadcastRequest) -> ClientResult<BroadcastResponse> {
            self.broadcast_requests.lock().unwrap().push(request);
            let mut errors = self.broadcast_errors.lock().unwrap();
            if !errors.is_empty() {
                return Err(errors.remove(0));
            }
            Ok(BroadcastResponse)
        }

        fn set_auth_token(&self, token: String) {
            self.auth_tokens.lock().unwrap().push(token);
        }
    }

    /// Runs one client call on a fresh single-threaded runtime. Background
    /// loops it spawns run only while the call waits, and stop when the
    /// runtime is dropped at the end, so request counts stay exact.
    fn block_on<F>(future: F) -> F::Output
    where
        F: Future,
    {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
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

    /// Whether `key` is a lowercase RFC 4122 version 4 UUID, the format of
    /// JS `uuid()`.
    fn is_v4_uuid(key: &str) -> bool {
        let groups: Vec<&str> = key.split('-').collect();
        groups.iter().map(|group| group.len()).eq([8, 4, 4, 4, 12])
            && groups.iter().all(|group| {
                group
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            })
            && groups[2].starts_with('4')
            && groups[3].starts_with(['8', '9', 'a', 'b'])
    }

    /// JS `opts.key || uuid()`: a client without a key gets a random version
    /// 4 UUID, and an empty key counts as none.
    #[test]
    fn generates_random_uuid_client_keys() {
        let transport = Arc::new(MockTransport::default());
        let first = mock_client(ClientOptions::default(), &transport);
        let second = mock_client(ClientOptions::default(), &transport);
        assert!(is_v4_uuid(first.key()), "{}", first.key());
        assert!(is_v4_uuid(second.key()), "{}", second.key());
        assert_ne!(first.key(), second.key());

        let empty = mock_client(
            ClientOptions {
                key: Some(String::new()),
                ..ClientOptions::default()
            },
            &transport,
        );
        assert!(is_v4_uuid(empty.key()), "{}", empty.key());
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
            watch_heartbeat_interval: Duration::from_secs(5),
            user_agent: Some("test-agent".to_owned()),
            auth_token_injector: None,
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
    fn push_only_sync_takes_only_the_push_ack_for_push_only_attachment() -> ClientResult<()> {
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

        // The mock echoes the pushed change back. As in JS, the push is
        // acknowledged, so the change is not pushed again, but the echoed
        // change is not applied a second time.
        let doc = doc.lock().unwrap();
        let requests = transport.push_pull_requests.lock().unwrap();
        assert_eq!(1, requests.len());
        assert!(requests[0].push_only);
        assert!(!doc.has_local_changes());
        assert_eq!(
            requests[0].change_pack.checkpoint().client_seq(),
            doc.checkpoint().client_seq()
        );
        assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
        Ok(())
    }

    /// JS `syncInternal`: the reply to a push-only request carries the
    /// server's minimum version vector, which must not reach garbage
    /// collection. The mock echoes this document's own vector, which covers
    /// its removal.
    #[test]
    fn push_only_sync_does_not_collect_garbage() -> ClientResult<()> {
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
        doc.lock().unwrap().update(|root| {
            root.set("title", "hello")?;
            root.remove("title");
            Ok(())
        })?;
        assert_eq!(1, doc.lock().unwrap().garbage_len());

        block_on(client.sync(None))?;

        assert!(transport.push_pull_requests.lock().unwrap()[0].push_only);
        assert_eq!(1, doc.lock().unwrap().garbage_len());
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

    fn fast_loop_options() -> ClientOptions {
        ClientOptions {
            sync_loop_duration: Duration::from_millis(5),
            retry_sync_loop_delay: Duration::from_millis(5),
            reconnect_stream_delay: Duration::from_millis(5),
            ..keyed_options()
        }
    }

    /// Polls `condition` every few milliseconds for up to five seconds.
    async fn eventually(mut condition: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "condition not met within 5s"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn set_title(doc: &SharedDocument) {
        doc.lock()
            .unwrap()
            .update(|root| root.set("title", "hello").map(|_| ()))
            .unwrap();
    }

    fn pushed_keys(transport: &MockTransport) -> Vec<String> {
        transport
            .push_pull_requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.change_pack.document_key().to_owned())
            .collect()
    }

    fn collect_events(doc: &SharedDocument) -> Arc<Mutex<Vec<DocEvent>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        doc.lock()
            .unwrap()
            .subscribe(move |event| sink.lock().unwrap().push(event.clone()));
        events
    }

    /// JS `runSyncLoop` with `needRealtimeSync`: realtime documents with
    /// local changes are pushed in the background, manual ones are not.
    #[tokio::test]
    async fn sync_loop_pushes_realtime_documents_only() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        let realtime = shared("realtime");
        let manual = shared("manual");

        client.activate().await?;
        client
            .attach(realtime.clone(), AttachOptions::default())
            .await?;
        client
            .attach(
                manual.clone(),
                AttachOptions {
                    sync_mode: SyncMode::Manual,
                    ..AttachOptions::default()
                },
            )
            .await?;
        let events = collect_events(&realtime);
        set_title(&realtime);
        set_title(&manual);

        eventually(|| !realtime.lock().unwrap().has_local_changes()).await;
        assert!(events
            .lock()
            .unwrap()
            .contains(&DocEvent::SyncStatusChanged(DocSyncStatus::Synced)));
        // Give the loop a few more rounds to (wrongly) pick up the manual one.
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(manual.lock().unwrap().has_local_changes());
        assert!(!pushed_keys(&transport).contains(&"manual".to_owned()));
        {
            let requests = transport.watch_requests.lock().unwrap();
            assert_eq!(1, requests.len());
            assert_eq!("document-id", requests[0].document_id);
            assert_eq!("000000000000000000000001", requests[0].actor_id.as_str());
            assert_eq!(20000, requests[0].heartbeat_interval_ms);
            assert_eq!("api-key/realtime", requests[0].shard_key);
        }

        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `runWatchStream`: an error ends the stream with `Disconnected`, and
    /// the loop opens a new one after `reconnectStreamDelay`.
    #[tokio::test]
    async fn watch_loop_reconnects_after_a_retryable_error() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        *transport.watch_scripts.lock().unwrap() = vec![vec![
            Ok(WatchDocumentResponse::Initialization {
                client_ids: Vec::new(),
                heartbeat_interval_ms: 0,
            }),
            Err(ClientError::Transport("connection reset".to_owned())),
        ]];
        let client = mock_client(fast_loop_options(), &transport);
        let doc = shared("doc-key");
        let events = collect_events(&doc);

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        eventually(|| transport.watch_requests.lock().unwrap().len() >= 2).await;
        eventually(|| {
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| matches!(event, DocEvent::Initialized(_)))
                .count()
                >= 3
        })
        .await;

        let connection = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                DocEvent::ConnectionChanged(status) => Some(*status),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                StreamConnectionStatus::Connected,
                StreamConnectionStatus::Disconnected,
                StreamConnectionStatus::Connected,
            ],
            connection
        );
        assert!(client.condition(ClientCondition::WatchLoop));
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `handleConnectError` returning false: the watch loop stops.
    #[tokio::test]
    async fn watch_loop_stops_after_a_non_retryable_error() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        *transport.watch_scripts.lock().unwrap() = vec![vec![Err(ClientError::Rpc {
            code: "permission_denied".to_owned(),
            message: "denied".to_owned(),
            yorkie_code: None,
            reason: None,
        })]];
        let client = mock_client(fast_loop_options(), &transport);
        let doc = shared("doc-key");

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        eventually(|| client.running_task_count() == 1).await;

        assert!(!client.condition(ClientCondition::WatchLoop));
        assert!(client.has("doc-key"));
        assert_eq!(1, transport.watch_requests.lock().unwrap().len());
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `changeSyncMode`: Manual closes the stream, Realtime reopens it.
    #[tokio::test]
    async fn change_sync_mode_stops_and_restarts_the_watch_stream() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        let doc = shared("doc-key");

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        assert_eq!(2, client.running_task_count());

        client.change_sync_mode(&doc, SyncMode::Manual).await?;
        eventually(|| client.running_task_count() == 1).await;
        set_title(&doc);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(doc.lock().unwrap().has_local_changes());

        // Requests from the realtime period, before the switch.
        let pushed_before = transport.push_pull_requests.lock().unwrap().len();
        client
            .change_sync_mode(&doc, SyncMode::RealtimePushOnly)
            .await?;
        assert_eq!(2, client.running_task_count());
        assert_eq!(2, transport.watch_requests.lock().unwrap().len());
        eventually(|| transport.push_pull_requests.lock().unwrap().len() > pushed_before).await;
        assert!(
            transport.push_pull_requests.lock().unwrap()[pushed_before..]
                .iter()
                .all(|request| request.push_only)
        );

        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// Loops hold the client weakly: deactivating stops them, and dropping
    /// the last handle drops the client and stops them too.
    #[tokio::test]
    async fn background_loops_stop_on_deactivate_and_on_drop() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        client
            .attach(shared("doc-key"), AttachOptions::default())
            .await?;
        assert_eq!(2, client.running_task_count());
        client.deactivate(DeactivateOptions::default()).await?;
        eventually(|| client.running_task_count() == 0).await;

        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        client
            .attach(shared("doc-key"), AttachOptions::default())
            .await?;
        let running = client.inner.running_tasks.clone();
        let inner = Arc::downgrade(&client.inner);
        assert_eq!(2, running.load(std::sync::atomic::Ordering::SeqCst));
        drop(client);
        eventually(|| inner.upgrade().is_none()).await;
        eventually(|| running.load(std::sync::atomic::Ordering::SeqCst) == 0).await;
        Ok(())
    }

    /// Review rule: an attach that finishes after the client left the session
    /// it started in must not register the document.
    #[test]
    fn attach_finishing_after_deactivation_is_rejected() {
        struct DeactivatingTransport {
            mock: MockTransport,
            client: Mutex<Option<Client>>,
        }

        #[async_trait]
        impl ClientTransport for DeactivatingTransport {
            async fn activate_client(
                &self,
                request: ActivateClientRequest,
            ) -> ClientResult<ActivateClientResponse> {
                self.mock.activate_client(request).await
            }

            async fn deactivate_client(
                &self,
                request: DeactivateClientRequest,
            ) -> ClientResult<DeactivateClientResponse> {
                self.mock.deactivate_client(request).await
            }

            /// Deactivates the client while the attach is "in flight".
            async fn attach_document(
                &self,
                request: AttachDocumentRequest,
            ) -> ClientResult<AttachDocumentResponse> {
                if let Some(client) = self.client.lock().unwrap().take() {
                    client.state().status = ClientStatus::Deactivated;
                }
                self.mock.attach_document(request).await
            }

            async fn detach_document(
                &self,
                request: DetachDocumentRequest,
            ) -> ClientResult<DetachDocumentResponse> {
                self.mock.detach_document(request).await
            }

            async fn remove_document(
                &self,
                request: RemoveDocumentRequest,
            ) -> ClientResult<RemoveDocumentResponse> {
                self.mock.remove_document(request).await
            }

            async fn push_pull_changes(
                &self,
                request: PushPullChangesRequest,
            ) -> ClientResult<PushPullChangesResponse> {
                self.mock.push_pull_changes(request).await
            }

            async fn watch_document(
                &self,
                request: WatchDocumentRequest,
            ) -> ClientResult<WatchStream> {
                self.mock.watch_document(request).await
            }

            async fn refresh_channel(
                &self,
                request: RefreshChannelRequest,
            ) -> ClientResult<RefreshChannelResponse> {
                self.mock.refresh_channel(request).await
            }

            async fn peek_channel(
                &self,
                request: PeekChannelRequest,
            ) -> ClientResult<PeekChannelResponse> {
                self.mock.peek_channel(request).await
            }

            async fn watch_channel(
                &self,
                request: WatchChannelRequest,
            ) -> ClientResult<ChannelWatchStream> {
                self.mock.watch_channel(request).await
            }

            async fn broadcast(
                &self,
                request: BroadcastRequest,
            ) -> ClientResult<BroadcastResponse> {
                self.mock.broadcast(request).await
            }
        }

        let transport = Arc::new(DeactivatingTransport {
            mock: MockTransport::default(),
            client: Mutex::new(None),
        });
        let client = Client::with_transport(keyed_options(), transport.clone());
        *transport.client.lock().unwrap() = Some(client.clone());
        let doc = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        let err = block_on(client.attach(doc.clone(), AttachOptions::default())).unwrap_err();

        assert_eq!(
            ClientError::ClientNotActivated("client-key".to_owned()),
            err
        );
        assert!(!client.has("doc-key"));
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert_eq!(0, client.running_task_count());
    }

    /// Review rule: work that resumes after an `.await` only removes the
    /// attachment it started with, never a newer one with the same key.
    #[test]
    fn remove_attachment_keeps_a_newer_document_with_the_same_key() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        let doc = shared("doc-key");
        let stale = shared("doc-key");

        client.apply_activation("000000000000000000000001");
        block_on(client.attach(doc.clone(), AttachOptions::default()))?;
        client.remove_attachment("doc-key", &stale);
        assert!(client.has("doc-key"));

        assert_eq!(
            ClientError::NotAttached("doc-key".to_owned()),
            block_on(client.sync(Some(&stale))).unwrap_err()
        );
        assert_eq!(
            ClientError::NotAttached("doc-key".to_owned()),
            block_on(client.detach(&stale, DetachOptions)).unwrap_err()
        );
        client.remove_attachment("doc-key", &doc);
        assert!(!client.has("doc-key"));
        Ok(())
    }

    fn shared_channel(key: &str) -> SharedChannel {
        Arc::new(Mutex::new(Channel::new(key).unwrap()))
    }

    fn collect_channel_events(channel: &SharedChannel) -> Arc<Mutex<Vec<ChannelEvent>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        channel.lock().unwrap().subscribe({
            let events = events.clone();
            move |event| events.lock().unwrap().push(event.clone())
        });
        events
    }

    /// Channel options with a 10ms heartbeat, so tests see several refreshes.
    fn fast_channel(sync_mode: SyncMode) -> AttachChannelOptions {
        AttachChannelOptions {
            sync_mode: Some(sync_mode),
            channel_heartbeat_interval: Some(Duration::from_millis(10)),
        }
    }

    /// JS "can attach a channel without activating the client": the first
    /// refresh carries the client key and no IDs, and its reply activates the
    /// client and attaches the channel. Later heartbeats carry both IDs.
    #[tokio::test]
    async fn first_channel_refresh_activates_the_client() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let options = ClientOptions {
            metadata: BTreeMap::from([("name".to_owned(), "a".to_owned())]),
            ..fast_loop_options()
        };
        let client = mock_client(options, &transport);
        let channel = shared_channel("room.1");
        let events = collect_channel_events(&channel);

        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        assert!(client.has("room.1"));
        eventually(|| transport.refresh_requests.lock().unwrap().len() >= 2).await;

        assert!(client.is_active());
        let client_id = ActorId::new("000000000000000000000001");
        assert_eq!(Some(client_id.clone()), client.id());
        assert_eq!(Some(client_id.clone()), client.actor_id());
        {
            let channel = channel.lock().unwrap();
            assert_eq!(ChannelStatus::Attached, channel.status());
            assert_eq!(Some("session-1"), channel.session_id());
            assert_eq!(Some(&client_id), channel.actor_id());
            assert_eq!(1, channel.session_count());
        }
        assert_eq!(
            vec![ChannelEvent::PresenceChanged { count: 1 }],
            *events.lock().unwrap()
        );

        let requests = transport.refresh_requests.lock().unwrap().clone();
        assert_eq!(None, requests[0].client_id);
        assert_eq!(None, requests[0].session_id);
        assert_eq!("client-key", requests[0].client_key);
        assert_eq!(
            Some("a"),
            requests[0].metadata.get("name").map(String::as_str)
        );
        assert_eq!("api-key/room", requests[0].shard_key);
        assert_eq!(Some(client_id), requests[1].client_id);
        assert_eq!(Some("session-1"), requests[1].session_id.as_deref());
        assert_eq!("", requests[1].client_key);
        assert!(requests[1].metadata.is_empty());
        assert!(transport.activate_requests.lock().unwrap().is_empty());
        Ok(())
    }

    /// JS "recovers transparently when the server forgets the session": an
    /// `ErrSessionNotFound` refresh clears the session, and the next refresh
    /// starts a new one under the same client.
    #[tokio::test]
    async fn lost_channel_session_is_created_again() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        let events = collect_channel_events(&channel);
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;

        transport
            .refresh_errors
            .lock()
            .unwrap()
            .push(ClientError::Rpc {
                code: "not_found".to_owned(),
                message: "session not found".to_owned(),
                yorkie_code: Some("ErrSessionNotFound".to_owned()),
                reason: None,
            });
        eventually(|| channel.lock().unwrap().session_id() == Some("session-2")).await;

        let requests = transport.refresh_requests.lock().unwrap().clone();
        let restart = requests
            .iter()
            .rposition(|request| request.session_id.is_none())
            .unwrap();
        assert!(restart > 0);
        assert_eq!(client.id(), requests[restart].client_id);
        assert_eq!("client-key", requests[restart].client_key);
        assert!(client.condition(ClientCondition::SyncLoop));
        assert!(!events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ChannelEvent::SyncError { .. })));
        Ok(())
    }

    /// JS `SyncError`: a refresh failure is published on the channel, and
    /// one that is not retryable stops the sync loop.
    #[tokio::test]
    async fn failed_channel_refresh_publishes_a_sync_error() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let error = ClientError::Rpc {
            code: "permission_denied".to_owned(),
            message: "denied".to_owned(),
            yorkie_code: None,
            reason: None,
        };
        transport.refresh_errors.lock().unwrap().push(error.clone());
        let client = mock_client(fast_loop_options(), &transport);
        let channel = shared_channel("room");
        let events = collect_channel_events(&channel);

        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| !client.condition(ClientCondition::SyncLoop)).await;

        assert_eq!(
            vec![ChannelEvent::SyncError {
                error,
                method: "RefreshChannel".to_owned(),
            }],
            *events.lock().unwrap()
        );
        assert!(!channel.lock().unwrap().is_attached());
        assert!(!client.is_active());
        Ok(())
    }

    /// JS `needSync`: a `Manual` channel is refreshed only by
    /// `sync(channel)`.
    #[tokio::test]
    async fn manual_channel_refreshes_only_when_synced() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Manual))
            .await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(transport.refresh_requests.lock().unwrap().is_empty());

        client.sync_channel(&channel).await?;
        assert_eq!(1, transport.refresh_requests.lock().unwrap().len());
        assert!(channel.lock().unwrap().is_attached());

        client.detach_channel(&channel).await?;
        assert_eq!(
            ClientError::NotAttached("room".to_owned()),
            client.sync_channel(&channel).await.unwrap_err()
        );
        Ok(())
    }

    /// JS `attachChannel` and `assertValidChannelSyncMode`.
    #[tokio::test]
    async fn attach_channel_rejects_invalid_options_and_taken_keys() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;

        for sync_mode in [SyncMode::RealtimePushOnly, SyncMode::RealtimeSyncOff] {
            let options = AttachChannelOptions {
                sync_mode: Some(sync_mode),
                ..AttachChannelOptions::default()
            };
            assert!(matches!(
                client.attach_channel(shared_channel("room"), options).await,
                Err(ClientError::InvalidArgument(message))
                    if message.starts_with("invalid channel sync mode")
            ));
        }
        let zero = AttachChannelOptions {
            channel_heartbeat_interval: Some(Duration::ZERO),
            ..AttachChannelOptions::default()
        };
        assert_eq!(
            ClientError::InvalidArgument(
                "channel_heartbeat_interval must be greater than 0".to_owned()
            ),
            client
                .attach_channel(shared_channel("room"), zero)
                .await
                .unwrap_err()
        );
        assert!(!client.has("room"));

        client
            .attach(shared("taken"), AttachOptions::default())
            .await?;
        assert_eq!(
            ClientError::AlreadyAttached("taken".to_owned()),
            client
                .attach_channel(shared_channel("taken"), AttachChannelOptions::default())
                .await
                .unwrap_err()
        );
        let channel = client
            .attach_channel(shared_channel("room"), fast_channel(SyncMode::Polling))
            .await?;
        assert_eq!(
            ClientError::AlreadyAttached("room".to_owned()),
            client
                .attach(shared("room"), AttachOptions::default())
                .await
                .unwrap_err()
        );
        eventually(|| channel.lock().unwrap().is_attached()).await;
        assert_eq!(
            ClientError::NotDetached("room".to_owned()),
            client
                .attach_channel(channel.clone(), AttachChannelOptions::default())
                .await
                .unwrap_err()
        );
        Ok(())
    }

    /// JS `changeChannelSyncMode`: an unpinned heartbeat interval follows the
    /// mode, a pinned one stays, and document-only modes are rejected.
    #[tokio::test]
    async fn change_channel_sync_mode_resets_an_unpinned_interval() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let unpinned = shared_channel("unpinned");
        let pinned = shared_channel("pinned");
        let polling = AttachChannelOptions {
            sync_mode: Some(SyncMode::Polling),
            ..AttachChannelOptions::default()
        };
        client.attach_channel(unpinned.clone(), polling).await?;
        client
            .attach_channel(pinned.clone(), fast_channel(SyncMode::Polling))
            .await?;
        let interval = |key: &str| client.state().channels[key].heartbeat_interval;

        client
            .change_channel_sync_mode(&unpinned, SyncMode::Manual)
            .await?;
        client
            .change_channel_sync_mode(&pinned, SyncMode::Manual)
            .await?;
        assert_eq!(Duration::ZERO, interval("unpinned"));
        assert_eq!(Duration::from_millis(10), interval("pinned"));

        client
            .change_channel_sync_mode(&unpinned, SyncMode::Polling)
            .await?;
        assert_eq!(
            Duration::from_millis(DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS),
            interval("unpinned")
        );
        assert!(matches!(
            client
                .change_channel_sync_mode(&unpinned, SyncMode::RealtimePushOnly)
                .await,
            Err(ClientError::InvalidArgument(_))
        ));
        Ok(())
    }

    /// JS `deactivateInternal`: channels are detached too.
    #[tokio::test]
    async fn deactivate_detaches_channels() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;

        let detached = shared_channel("detached");
        client
            .attach_channel(detached.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| detached.lock().unwrap().is_attached()).await;
        let refreshes_of = |key: &str| {
            transport
                .refresh_requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.channel_key == key)
                .count()
        };

        // A detached channel is no longer refreshed, while the other is.
        client.detach_channel(&detached).await?;
        let (detached_before, room_before) = (refreshes_of("detached"), refreshes_of("room"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(detached_before, refreshes_of("detached"));
        assert!(refreshes_of("room") > room_before);

        client.deactivate(DeactivateOptions::default()).await?;
        let total = transport.refresh_requests.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(total, transport.refresh_requests.lock().unwrap().len());

        assert!(!client.has("room"));
        let channel = channel.lock().unwrap();
        assert_eq!(ChannelStatus::Detached, channel.status());
        assert_eq!(None, channel.session_id());
        Ok(())
    }

    /// JS `peekChannel`: no activation, and the shard key is the first key
    /// path.
    #[tokio::test]
    async fn peek_channel_needs_no_activation() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(keyed_options(), &transport);
        assert_eq!(2, client.peek_channel("room.1").await?);
        let requests = transport.peek_requests.lock().unwrap();
        assert_eq!("room.1", requests[0].channel_key);
        assert_eq!("api-key/room", requests[0].shard_key);
        assert!(!client.is_active());
        Ok(())
    }

    /// JS `exponentialBackoff`: 1s doubling per retry, at most 20s.
    #[test]
    fn broadcast_retries_back_off_exponentially() {
        let delays: Vec<u64> = (0..7)
            .map(|retries| broadcast_retry_delay(retries).as_secs())
            .collect();
        assert_eq!(vec![1, 2, 4, 8, 16, 20, 20], delays);
        assert_eq!(Duration::from_secs(20), broadcast_retry_delay(u32::MAX));
    }

    /// JS `createChannelWatchStream` and `handleWatchChannelResponse`: a
    /// realtime channel watches once its first refresh activated the client;
    /// stale counts and broadcasts whose payload is not JSON are dropped.
    #[tokio::test]
    async fn realtime_channel_watches_after_its_first_refresh() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let publisher = ActorId::new("000000000000000000000002");
        let event = |event_type, session_count, seq, payload: &[u8]| {
            Ok(WatchChannelResponse::Event(WatchChannelEvent {
                event_type,
                publisher: publisher.clone(),
                session_count,
                seq,
                topic: "chat".to_owned(),
                payload: payload.to_vec(),
            }))
        };
        *transport.channel_watch_scripts.lock().unwrap() = vec![vec![
            Ok(WatchChannelResponse::Initialization {
                session_count: 2,
                seq: 1,
                heartbeat_interval_ms: 0,
            }),
            event(WatchChannelEventType::Presence, 3, 3, b""),
            event(WatchChannelEventType::Presence, 1, 2, b""),
            event(WatchChannelEventType::Broadcast, 0, 0, b"not json"),
            event(WatchChannelEventType::Broadcast, 0, 0, br#"{"a":1}"#),
        ]];
        let client = mock_client(fast_loop_options(), &transport);
        let channel = shared_channel("room.1");
        let events = collect_channel_events(&channel);

        client
            .attach_channel(channel.clone(), AttachChannelOptions::default())
            .await?;
        eventually(|| events.lock().unwrap().len() == 4).await;

        assert_eq!(
            vec![
                ChannelEvent::PresenceChanged { count: 1 },
                ChannelEvent::Initialized { count: 2 },
                ChannelEvent::PresenceChanged { count: 3 },
                ChannelEvent::Broadcast {
                    client_id: publisher.clone(),
                    topic: "chat".to_owned(),
                    payload: json!({"a": 1}),
                },
            ],
            *events.lock().unwrap()
        );
        let requests = transport.channel_watch_requests.lock().unwrap().clone();
        assert_eq!(1, requests.len());
        assert_eq!(client.id(), Some(requests[0].client_id.clone()));
        assert_eq!("room.1", requests[0].channel_key);
        assert_eq!("api-key/room", requests[0].shard_key);
        assert!(client.condition(ClientCondition::WatchLoop));
        assert_eq!(2, client.running_task_count());
        Ok(())
    }

    /// JS "Polling mode does not open a watch stream" and
    /// `changeChannelSyncMode`: entering `Realtime` opens the stream and waits
    /// for it; leaving it closes the stream.
    #[tokio::test]
    async fn only_realtime_channels_watch() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        let events = collect_channel_events(&channel);
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(transport.channel_watch_requests.lock().unwrap().is_empty());
        assert_eq!(1, client.running_task_count());

        client
            .change_channel_sync_mode(&channel, SyncMode::Realtime)
            .await?;
        assert_eq!(1, transport.channel_watch_requests.lock().unwrap().len());
        assert!(events
            .lock()
            .unwrap()
            .contains(&ChannelEvent::Initialized { count: 1 }));
        assert_eq!(2, client.running_task_count());

        client
            .change_channel_sync_mode(&channel, SyncMode::Polling)
            .await?;
        eventually(|| client.running_task_count() == 1).await;
        assert!(!client.condition(ClientCondition::WatchLoop));
        Ok(())
    }

    /// JS `client.broadcast` and the `local-broadcast` handler of
    /// `attachChannel`: the payload goes out as JSON with the first key path
    /// as shard key, and detaching removes the handler.
    #[tokio::test]
    async fn broadcasts_are_sent_as_json() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        assert_eq!(
            ClientError::NotAttached("room.1".to_owned()),
            client
                .broadcast("room.1", "chat", json!(1), BroadcastOptions::default())
                .await
                .unwrap_err()
        );
        let channel = shared_channel("room.1");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;

        client
            .broadcast(
                "room.1",
                "chat",
                json!({"a": 1}),
                BroadcastOptions::default(),
            )
            .await?;
        let errors = Arc::new(Mutex::new(Vec::new()));
        let options = BroadcastOptions {
            max_retries: Some(0),
            error: Some(BroadcastErrorHandler::new({
                let errors = errors.clone();
                move |err| errors.lock().unwrap().push(err.clone())
            })),
        };
        channel
            .lock()
            .unwrap()
            .broadcast("cursor", json!([1, 2]), options)?;
        eventually(|| transport.broadcast_requests.lock().unwrap().len() == 2).await;

        let requests = transport.broadcast_requests.lock().unwrap().clone();
        assert_eq!(client.id(), Some(requests[0].client_id.clone()));
        assert_eq!("room.1", requests[0].channel_key);
        assert_eq!("chat", requests[0].topic);
        assert_eq!(br#"{"a":1}"#.to_vec(), requests[0].payload);
        assert_eq!("api-key/room", requests[0].shard_key);
        assert_eq!("cursor", requests[1].topic);
        assert_eq!(b"[1,2]".to_vec(), requests[1].payload);
        assert!(errors.lock().unwrap().is_empty());

        // Detaching removes the sending subscriber: after attaching again,
        // one broadcast is sent once, not twice (JS `unsubscribeLocalBroadcast`).
        client.detach_channel(&channel).await?;
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;
        channel
            .lock()
            .unwrap()
            .broadcast("again", json!(null), BroadcastOptions::default())?;
        eventually(|| transport.broadcast_requests.lock().unwrap().len() == 3).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(3, transport.broadcast_requests.lock().unwrap().len());
        Ok(())
    }

    /// Review rule: detaching while a broadcast waits to retry ends the
    /// retries (Rust checks the attachment on every try).
    #[tokio::test(start_paused = true)]
    async fn detaching_ends_broadcast_retries() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;
        transport
            .broadcast_errors
            .lock()
            .unwrap()
            .push(ClientError::Transport("connection reset".to_owned()));

        let broadcast = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .broadcast("room", "chat", json!(1), BroadcastOptions::default())
                    .await
            }
        });
        eventually(|| transport.broadcast_requests.lock().unwrap().len() == 1).await;
        client.detach_channel(&channel).await?;

        assert_eq!(
            Err(ClientError::NotAttached("room".to_owned())),
            broadcast.await.unwrap()
        );
        assert_eq!(1, transport.broadcast_requests.lock().unwrap().len());
        Ok(())
    }

    /// Review rule: a channel's pending `LocalBroadcast` retries end when
    /// that channel detaches, even if another channel is attached under the
    /// same key before the retry is due. They must not go out on the new
    /// channel's session.
    #[tokio::test(start_paused = true)]
    async fn local_broadcast_retries_do_not_move_to_a_new_channel_of_the_key() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let first = shared_channel("room");
        client
            .attach_channel(first.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| first.lock().unwrap().is_attached()).await;
        transport
            .broadcast_errors
            .lock()
            .unwrap()
            .push(ClientError::Transport("connection reset".to_owned()));

        let errors = Arc::new(Mutex::new(Vec::new()));
        let options = BroadcastOptions {
            max_retries: None,
            error: Some(BroadcastErrorHandler::new({
                let errors = errors.clone();
                move |err| errors.lock().unwrap().push(err.clone())
            })),
        };
        first
            .lock()
            .unwrap()
            .broadcast("cursor", json!(1), options)?;
        eventually(|| transport.broadcast_requests.lock().unwrap().len() == 1).await;
        client.detach_channel(&first).await?;
        let second = shared_channel("room");
        client
            .attach_channel(second.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| second.lock().unwrap().is_attached()).await;

        // The first retry is due 1s after the failure.
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(1, transport.broadcast_requests.lock().unwrap().len());
        assert_eq!(
            vec![ClientError::NotAttached("room".to_owned())],
            errors.lock().unwrap().clone()
        );
        Ok(())
    }

    /// `Client::broadcast` returns a failure that is not retryable at once,
    /// without waiting to retry.
    #[tokio::test(start_paused = true)]
    async fn client_broadcast_returns_a_non_retryable_error() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let client = mock_client(fast_loop_options(), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;
        let denied = ClientError::Rpc {
            code: "permission_denied".to_owned(),
            message: "denied".to_owned(),
            yorkie_code: None,
            reason: None,
        };
        transport
            .broadcast_errors
            .lock()
            .unwrap()
            .push(denied.clone());

        let started = tokio::time::Instant::now();
        assert_eq!(
            Err(denied),
            client
                .broadcast("room", "chat", json!(1), BroadcastOptions::default())
                .await
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(1, transport.broadcast_requests.lock().unwrap().len());
        Ok(())
    }

    /// Review rule (JS clears `SyncLoop` synchronously): a round that ends
    /// the loop marks it stopped before releasing the task queue, so a call
    /// queued behind the round sees it stopped. This pins the invariant; the
    /// old window was too short for it to catch reliably, which
    /// `attach_channel_restarts_an_ended_sync_loop` covers instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_loop_is_marked_stopped_before_the_queue_is_released() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let gate = Arc::new(tokio::sync::Notify::new());
        *transport.refresh_gate.lock().unwrap() = Some(gate.clone());
        transport
            .refresh_errors
            .lock()
            .unwrap()
            .push(ClientError::Rpc {
                code: "permission_denied".to_owned(),
                message: "denied".to_owned(),
                yorkie_code: None,
                reason: None,
            });
        let client = mock_client(fast_loop_options(), &transport);
        client
            .attach_channel(shared_channel("first"), fast_channel(SyncMode::Polling))
            .await?;
        // The round is now inside the refresh, holding the task queue.
        eventually(|| transport.refresh_requests.lock().unwrap().len() == 1).await;

        let queued = tokio::spawn({
            let client = client.clone();
            async move {
                let _queue = client.lock_task_queue().await;
                client.condition(ClientCondition::SyncLoop)
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        gate.notify_one();

        assert!(!queued.await.unwrap());
        Ok(())
    }

    /// Review rule: `attach_channel` starts a new sync loop when the old one
    /// has ended, even if the condition was left set, so the channel is
    /// refreshed.
    #[tokio::test]
    async fn attach_channel_restarts_an_ended_sync_loop() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport
            .refresh_errors
            .lock()
            .unwrap()
            .push(ClientError::Rpc {
                code: "permission_denied".to_owned(),
                message: "denied".to_owned(),
                yorkie_code: None,
                reason: None,
            });
        let client = mock_client(fast_loop_options(), &transport);
        client
            .attach_channel(shared_channel("first"), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| !client.condition(ClientCondition::SyncLoop)).await;
        eventually(|| client.running_task_count() == 0).await;

        // The window the condition guards: set, but the loop is gone.
        client.set_condition(ClientCondition::SyncLoop, true);
        let second = shared_channel("second");
        client
            .attach_channel(second.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| second.lock().unwrap().is_attached()).await;
        assert!(client.is_active());
        Ok(())
    }

    /// A server error with the given Connect code, Yorkie code, and reason.
    fn rpc_error(code: &str, yorkie_code: ErrorCode, reason: Option<&str>) -> ClientError {
        ClientError::Rpc {
            code: code.to_owned(),
            message: code.to_owned(),
            yorkie_code: Some(yorkie_code.as_str().to_owned()),
            reason: reason.map(str::to_owned),
        }
    }

    /// The refusal the server sends when the auth webhook rejects an expired
    /// token (JS `unauthenticated()` in `auth_refresh_test.ts`).
    fn token_expired() -> ClientError {
        rpc_error(
            "unauthenticated",
            ErrorCode::ErrUnauthenticated,
            Some("token expired"),
        )
    }

    /// What a failing token API makes the injector return.
    fn token_api_unavailable() -> ClientError {
        ClientError::Transport("token api unavailable".to_owned())
    }

    /// An auth token injector that gives `answers` in order and then
    /// `token-<n>` for its n-th call, with the reasons it was called with.
    fn injector(
        answers: Vec<ClientResult<String>>,
    ) -> (AuthTokenInjector, Arc<Mutex<Vec<Option<String>>>>) {
        let reasons = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(answers));
        let calls = reasons.clone();
        let injector: AuthTokenInjector = Arc::new(move |reason| {
            let mut calls = calls.lock().unwrap();
            calls.push(reason);
            let call = calls.len();
            let mut answers = answers.lock().unwrap();
            let answer = if answers.is_empty() {
                Ok(format!("token-{call}"))
            } else {
                answers.remove(0)
            };
            Box::pin(async move { answer })
        });
        (injector, reasons)
    }

    fn auth_options(injector: AuthTokenInjector) -> ClientOptions {
        ClientOptions {
            auth_token_injector: Some(injector),
            ..fast_loop_options()
        }
    }

    fn auth_errors(events: &Mutex<Vec<DocEvent>>) -> Vec<DocEvent> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, DocEvent::AuthError { .. }))
            .cloned()
            .collect()
    }

    /// JS `activate`: the injector gives the first token, with no reason,
    /// before the client registers; if it fails, so does `activate`.
    #[tokio::test]
    async fn activate_asks_the_injector_for_the_first_token() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let (injector, reasons) = injector(vec![Err(token_api_unavailable())]);
        let client = mock_client(auth_options(injector), &transport);

        assert_eq!(Err(token_api_unavailable()), client.activate().await);
        assert!(!client.is_active());
        assert!(transport.activate_requests.lock().unwrap().is_empty());

        client.activate().await?;
        assert!(client.is_active());
        assert_eq!(vec![None, None], *reasons.lock().unwrap());
        assert_eq!(vec!["token-2"], *transport.auth_tokens.lock().unwrap());
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `handleConnectError` for `ErrUnauthenticated` in the sync loop: the
    /// injector is called once with the server's reason, the new token is
    /// set, and the next round pushes with it. The document gets
    /// `auth-error` (JS webhook_test "should refresh token when receiving
    /// expired token error").
    #[tokio::test]
    async fn sync_loop_refreshes_an_expired_token_once_and_retries() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport
            .push_pull_errors
            .lock()
            .unwrap()
            .push(("doc-key".to_owned(), token_expired()));
        let (injector, reasons) = injector(Vec::new());
        let client = mock_client(auth_options(injector), &transport);
        let doc = shared("doc-key");

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        let events = collect_events(&doc);
        set_title(&doc);
        eventually(|| !doc.lock().unwrap().has_local_changes()).await;

        assert_eq!(
            vec![None, Some("token expired".to_owned())],
            *reasons.lock().unwrap()
        );
        assert_eq!(
            vec!["token-1", "token-2"],
            *transport.auth_tokens.lock().unwrap()
        );
        assert_eq!(
            vec![Some("token-1".to_owned()), Some("token-2".to_owned())],
            transport.push_pull_tokens.lock().unwrap()[..2]
        );
        assert_eq!(
            vec![DocEvent::AuthError {
                reason: Some("token expired".to_owned()),
                method: "PushPull".to_owned(),
            }],
            auth_errors(&events)
        );
        assert!(client.condition(ClientCondition::SyncLoop));
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `auth-error` for `ErrPermissionDenied`: published like a refusal,
    /// but the sync loop stops, as JS `handleConnectError` does not retry it.
    #[tokio::test]
    async fn permission_denied_publishes_an_auth_error_and_stops_the_loop() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport.push_pull_errors.lock().unwrap().push((
            "doc-key".to_owned(),
            rpc_error(
                "permission_denied",
                ErrorCode::ErrPermissionDenied,
                Some("no write"),
            ),
        ));
        let (injector, reasons) = injector(Vec::new());
        let client = mock_client(auth_options(injector), &transport);
        let doc = shared("doc-key");

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        let events = collect_events(&doc);
        set_title(&doc);
        eventually(|| !client.condition(ClientCondition::SyncLoop)).await;

        assert_eq!(
            vec![DocEvent::AuthError {
                reason: Some("no write".to_owned()),
                method: "PushPull".to_owned(),
            }],
            auth_errors(&events)
        );
        assert_eq!(vec![None], *reasons.lock().unwrap());
        assert!(client.is_active());
        Ok(())
    }

    /// JS `onError` of the document watch stream: a refusal publishes
    /// `auth-error` with method `Watch`, refreshes the token, and the stream
    /// is opened again.
    #[tokio::test]
    async fn watch_refusal_publishes_an_auth_error_and_reconnects() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        *transport.watch_scripts.lock().unwrap() = vec![vec![Err(token_expired())]];
        let (injector, reasons) = injector(Vec::new());
        let client = mock_client(auth_options(injector), &transport);
        let doc = shared("doc-key");
        let events = collect_events(&doc);

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        eventually(|| transport.watch_requests.lock().unwrap().len() >= 2).await;

        assert_eq!(
            vec![DocEvent::AuthError {
                reason: Some("token expired".to_owned()),
                method: "Watch".to_owned(),
            }],
            auth_errors(&events)
        );
        assert_eq!(
            vec![None, Some("token expired".to_owned())],
            *reasons.lock().unwrap()
        );
        assert!(client.condition(ClientCondition::WatchLoop));
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `handleConnectError` for `ErrTooManySubscribers`: the watch stream
    /// is opened again rather than given up.
    #[tokio::test]
    async fn too_many_subscribers_reconnects_the_watch_stream() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        *transport.watch_scripts.lock().unwrap() = vec![vec![Err(rpc_error(
            "resource_exhausted",
            ErrorCode::ErrTooManySubscribers,
            None,
        ))]];
        let client = mock_client(fast_loop_options(), &transport);
        let doc = shared("doc-key");

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        eventually(|| transport.watch_requests.lock().unwrap().len() >= 2).await;
        assert!(client.condition(ClientCondition::WatchLoop));
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS "lets a one-shot sync call the injector during the backoff and
    /// throw its error": `sync` returns the injector's error when it fails,
    /// and otherwise sets the new token and returns the refusal for the app
    /// to retry.
    #[tokio::test]
    async fn one_shot_sync_returns_the_injector_error_or_the_refusal() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport.push_pull_errors.lock().unwrap().extend([
            ("doc-key".to_owned(), token_expired()),
            ("doc-key".to_owned(), token_expired()),
        ]);
        let (injector, reasons) =
            injector(vec![Ok("token-1".to_owned()), Err(token_api_unavailable())]);
        let client = mock_client(auth_options(injector), &transport);
        let doc = shared("doc-key");
        client.activate().await?;
        client
            .attach(
                doc.clone(),
                AttachOptions {
                    sync_mode: SyncMode::Manual,
                    ..AttachOptions::default()
                },
            )
            .await?;

        assert_eq!(Err(token_api_unavailable()), client.sync(Some(&doc)).await);
        assert_eq!(vec!["token-1"], *transport.auth_tokens.lock().unwrap());
        assert_eq!(Err(token_expired()), client.sync(Some(&doc)).await);
        assert_eq!(
            vec!["token-1", "token-3"],
            *transport.auth_tokens.lock().unwrap()
        );
        assert_eq!(3, reasons.lock().unwrap().len());
        client.sync(Some(&doc)).await?;
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS "deactivates locally when the deactivation gets a 401 that cannot
    /// be refreshed".
    #[tokio::test]
    async fn deactivate_stops_locally_when_the_token_cannot_be_refreshed() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport
            .deactivate_errors
            .lock()
            .unwrap()
            .extend([token_expired(), token_expired()]);
        let (injector, _) = injector(vec![
            Ok("token-1".to_owned()),
            Ok("token-2".to_owned()),
            Err(ClientError::Transport("signed out".to_owned())),
        ]);
        let client = mock_client(auth_options(injector), &transport);
        let doc = shared("doc-key");
        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;

        // A token the injector refreshes: the app may retry, so stay active.
        assert_eq!(
            Err(token_expired()),
            client.deactivate(DeactivateOptions::default()).await
        );
        assert!(client.is_active());

        // Signed out: stop locally and return the injector's error.
        assert_eq!(
            Err(ClientError::Transport("signed out".to_owned())),
            client.deactivate(DeactivateOptions::default()).await
        );
        assert!(!client.is_active());
        assert!(!client.has("doc-key"));
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        eventually(|| client.running_task_count() == 0).await;
        Ok(())
    }

    /// JS `handleConnectError` for `ErrClientNotFound`: the client fixes its
    /// state by deactivating itself (`deactivateInternal`).
    #[tokio::test]
    async fn client_not_found_deactivates_the_client() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport.push_pull_errors.lock().unwrap().push((
            "doc-key".to_owned(),
            rpc_error("not_found", ErrorCode::ErrClientNotFound, None),
        ));
        let client = mock_client(fast_loop_options(), &transport);
        let doc = shared("doc-key");

        client.activate().await?;
        client.attach(doc.clone(), AttachOptions::default()).await?;
        set_title(&doc);
        eventually(|| !client.is_active()).await;

        assert!(!client.has("doc-key"));
        assert_eq!(DocStatus::Detached, doc.lock().unwrap().status());
        assert!(!client.condition(ClientCondition::SyncLoop));
        eventually(|| client.running_task_count() == 0).await;
        Ok(())
    }

    /// JS `markWriteRejected` (write_rejected_test.ts): a push refused for
    /// its size parks that document only; the loop goes on with the others,
    /// and an explicit `sync` that succeeds lets the loop take it again.
    #[tokio::test]
    async fn rejected_write_parks_only_that_document() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        transport.push_pull_errors.lock().unwrap().push((
            "big".to_owned(),
            rpc_error(
                "resource_exhausted",
                ErrorCode::ErrDocumentSizeExceedsLimit,
                None,
            ),
        ));
        let client = mock_client(fast_loop_options(), &transport);
        let big = shared("big");
        let small = shared("small");
        client.activate().await?;
        client.attach(big.clone(), AttachOptions::default()).await?;
        client
            .attach(small.clone(), AttachOptions::default())
            .await?;

        set_title(&big);
        eventually(|| pushed_keys(&transport).contains(&"big".to_owned())).await;
        set_title(&small);
        eventually(|| !small.lock().unwrap().has_local_changes()).await;
        tokio::time::sleep(Duration::from_millis(30)).await;

        let big_pushes = || {
            pushed_keys(&transport)
                .iter()
                .filter(|key| *key == "big")
                .count()
        };
        assert_eq!(1, big_pushes());
        assert!(big.lock().unwrap().has_local_changes());
        assert!(client.condition(ClientCondition::SyncLoop));

        client.sync(Some(&big)).await?;
        assert_eq!(2, big_pushes());
        set_title(&big);
        eventually(|| big_pushes() == 3).await;
        eventually(|| !big.lock().unwrap().has_local_changes()).await;
        client.deactivate(DeactivateOptions::default()).await?;
        Ok(())
    }

    /// JS `broadcast` after `ErrUnauthenticated`: the channel gets
    /// `auth-error` with method `Broadcast`, the token is refreshed, and the
    /// retry goes through.
    #[tokio::test(start_paused = true)]
    async fn broadcast_refusal_publishes_a_channel_auth_error() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let (injector, reasons) = injector(Vec::new());
        let client = mock_client(auth_options(injector), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;
        let events = collect_channel_events(&channel);
        transport
            .broadcast_errors
            .lock()
            .unwrap()
            .push(token_expired());

        client
            .broadcast("room", "chat", json!(1), BroadcastOptions::default())
            .await?;

        assert_eq!(2, transport.broadcast_requests.lock().unwrap().len());
        assert_eq!(
            vec![None, Some("token expired".to_owned())],
            *reasons.lock().unwrap()
        );
        let auth_errors = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, ChannelEvent::AuthError { .. }))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            vec![ChannelEvent::AuthError {
                reason: "token expired".to_owned(),
                method: "Broadcast".to_owned(),
            }],
            auth_errors
        );
        Ok(())
    }

    /// JS "calls the injector on a broadcast's first try during the backoff":
    /// the first try is a one-shot call, so it returns the injector's error.
    #[tokio::test]
    async fn broadcast_first_try_returns_the_injector_error() -> ClientResult<()> {
        let transport = Arc::new(MockTransport::default());
        let (injector, reasons) =
            injector(vec![Ok("token-1".to_owned()), Err(token_api_unavailable())]);
        let client = mock_client(auth_options(injector), &transport);
        client.activate().await?;
        let channel = shared_channel("room");
        client
            .attach_channel(channel.clone(), fast_channel(SyncMode::Polling))
            .await?;
        eventually(|| channel.lock().unwrap().is_attached()).await;
        transport
            .broadcast_errors
            .lock()
            .unwrap()
            .push(token_expired());

        assert_eq!(
            Err(token_api_unavailable()),
            client
                .broadcast("room", "chat", json!(1), BroadcastOptions::default())
                .await
        );
        assert_eq!(1, transport.broadcast_requests.lock().unwrap().len());
        assert_eq!(2, reasons.lock().unwrap().len());
        Ok(())
    }
}
