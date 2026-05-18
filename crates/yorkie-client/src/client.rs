use crate::attachment::{
    default_document_poll_interval, resolve_document_poll_interval, Attachment,
};
use crate::error::{ClientError, ClientResult};
use crate::options::{AttachOptions, ClientOptions, DeactivateOptions, DetachOptions, SyncMode};
use crate::transport::{
    ActivateClientRequest, AttachDocumentRequest, ClientTransport, DeactivateClientRequest,
    DetachDocumentRequest,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Client state holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    key: String,
    id: Option<ActorId>,
    status: ClientStatus,
    conditions: BTreeMap<ClientCondition, bool>,
    attachments: BTreeMap<String, Attachment>,
    options: ClientOptions,
}

impl Client {
    pub fn new(options: ClientOptions) -> Self {
        let key = options.key.clone().unwrap_or_else(generate_client_key);
        let conditions = BTreeMap::from([
            (ClientCondition::SyncLoop, false),
            (ClientCondition::WatchLoop, false),
        ]);

        Self {
            key,
            id: None,
            status: ClientStatus::Deactivated,
            conditions,
            attachments: BTreeMap::new(),
            options,
        }
    }

    pub fn id(&self) -> Option<&ActorId> {
        self.id.as_ref()
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn status(&self) -> ClientStatus {
        self.status
    }

    pub fn is_active(&self) -> bool {
        self.status == ClientStatus::Activated
    }

    pub fn condition(&self, condition: ClientCondition) -> bool {
        self.conditions.get(&condition).copied().unwrap_or(false)
    }

    pub fn options(&self) -> &ClientOptions {
        &self.options
    }

    pub fn has(&self, key: &str) -> bool {
        self.attachments.contains_key(key)
    }

    pub fn activate<T>(&mut self, transport: &mut T) -> ClientResult<()>
    where
        T: ClientTransport,
    {
        if self.is_active() {
            return Ok(());
        }

        let response = transport.activate_client(ActivateClientRequest {
            client_key: self.key.clone(),
            metadata: self.options.metadata.clone(),
            shard_key: self.shard_key(&self.key),
        })?;

        self.id = Some(response.client_id);
        self.status = ClientStatus::Activated;
        self.conditions.insert(ClientCondition::SyncLoop, true);
        Ok(())
    }

    pub fn deactivate<T>(
        &mut self,
        transport: &mut T,
        options: DeactivateOptions,
    ) -> ClientResult<()>
    where
        T: ClientTransport,
    {
        if self.status == ClientStatus::Deactivated {
            return Ok(());
        }

        let client_id = self.require_active()?.clone();
        transport.deactivate_client(DeactivateClientRequest {
            client_id,
            synchronous: options.synchronous,
            shard_key: self.shard_key(&self.key),
        })?;

        self.status = ClientStatus::Deactivated;
        self.conditions.insert(ClientCondition::SyncLoop, false);
        self.conditions.insert(ClientCondition::WatchLoop, false);
        self.attachments.clear();
        Ok(())
    }

    pub fn attach<T>(
        &mut self,
        transport: &mut T,
        doc: &mut Document,
        options: AttachOptions,
    ) -> ClientResult<()>
    where
        T: ClientTransport,
    {
        let actor_id = self.require_active()?.clone();
        if doc.status() != DocStatus::Detached {
            return Err(ClientError::NotDetached(doc.key().to_owned()));
        }

        let sync_mode = options.sync_mode.unwrap_or(SyncMode::Realtime);
        let (poll_interval, poll_interval_pinned) =
            resolve_document_poll_interval(sync_mode, options.document_poll_interval)?;

        doc.set_actor(actor_id.clone());
        let response = transport.attach_document(AttachDocumentRequest {
            client_id: actor_id,
            change_pack: doc.create_change_pack(),
            schema_key: options.schema.clone(),
            shard_key: self.shard_key(doc.key()),
        })?;
        if response.max_size_per_document > 0 {
            doc.set_max_size_per_document(response.max_size_per_document);
        }
        if !response.schema_rules.is_empty() {
            doc.set_schema_rules(response.schema_rules.clone());
        }
        doc.apply_change_pack(&response.change_pack)?;

        if doc.status() == DocStatus::Removed {
            return Ok(());
        }

        doc.apply_status(DocStatus::Attached);
        self.attachments.insert(
            doc.key().to_owned(),
            Attachment::new(
                response.document_id,
                sync_mode,
                poll_interval,
                poll_interval_pinned,
            ),
        );
        self.refresh_watch_loop_condition();

        if let Some(initial_root) = options.initial_root {
            let entries = initial_root
                .iter()
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect::<Vec<_>>();
            doc.update(|root| {
                for (key, value) in entries {
                    if root.get(&key).is_none() {
                        root.set(key, value)?;
                    }
                }
                Ok(())
            })?;
        }

        Ok(())
    }

    pub fn detach<T>(
        &mut self,
        transport: &mut T,
        doc: &mut Document,
        _options: DetachOptions,
    ) -> ClientResult<()>
    where
        T: ClientTransport,
    {
        let client_id = self.require_active()?.clone();
        let Some(attachment) = self.attachments.get(doc.key()) else {
            return Err(ClientError::NotAttached(doc.key().to_owned()));
        };
        let document_id = attachment.resource_id.clone();

        let response = transport.detach_document(DetachDocumentRequest {
            client_id,
            document_id,
            change_pack: doc.create_change_pack(),
            remove_if_not_attached: false,
            shard_key: self.shard_key(doc.key()),
        })?;
        doc.apply_change_pack(&response.change_pack)?;

        if doc.status() != DocStatus::Removed {
            doc.apply_status(DocStatus::Detached);
        }
        self.attachments.remove(doc.key());
        self.refresh_watch_loop_condition();
        Ok(())
    }

    pub fn change_sync_mode(&mut self, doc: &Document, sync_mode: SyncMode) -> ClientResult<()> {
        self.require_active()?;
        let Some(attachment) = self.attachments.get_mut(doc.key()) else {
            return Err(ClientError::NotAttached(doc.key().to_owned()));
        };

        if attachment.sync_mode == sync_mode {
            return Ok(());
        }

        attachment.sync_mode = sync_mode;
        if !attachment.poll_interval_pinned {
            attachment.poll_interval = default_document_poll_interval(sync_mode);
        }
        self.refresh_watch_loop_condition();
        Ok(())
    }

    fn require_active(&self) -> ClientResult<&ActorId> {
        if !self.is_active() {
            return Err(ClientError::ClientNotActivated(self.key.clone()));
        }

        self.id
            .as_ref()
            .ok_or_else(|| ClientError::ClientNotActivated(self.key.clone()))
    }

    fn shard_key(&self, resource_key: &str) -> String {
        format!("{}/{}", self.options.api_key, resource_key)
    }

    fn refresh_watch_loop_condition(&mut self) {
        let has_watch_attachment = self
            .attachments
            .values()
            .any(|attachment| should_start_watch_loop(attachment.sync_mode));
        self.conditions
            .insert(ClientCondition::WatchLoop, has_watch_attachment);
    }

    #[cfg(test)]
    fn apply_activation(&mut self, actor_id: impl Into<ActorId>) {
        self.id = Some(actor_id.into());
        self.status = ClientStatus::Activated;
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new(ClientOptions::default())
    }
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
        DetachDocumentRequest, DetachDocumentResponse,
    };
    use std::collections::BTreeMap;
    use std::time::Duration;
    use yorkie_core::{ActorId, DocStatus, Document, JsonObject, SchemaRule, TreeNodeRule};

    #[derive(Debug, Clone)]
    struct FakeTransport {
        client_id: ActorId,
        activate_requests: Vec<ActivateClientRequest>,
        deactivate_requests: Vec<DeactivateClientRequest>,
        attach_requests: Vec<AttachDocumentRequest>,
        detach_requests: Vec<DetachDocumentRequest>,
        attach_max_size_per_document: usize,
        attach_schema_rules: Vec<SchemaRule>,
    }

    impl Default for FakeTransport {
        fn default() -> Self {
            Self {
                client_id: ActorId::new("000000000000000000000001"),
                activate_requests: Vec::new(),
                deactivate_requests: Vec::new(),
                attach_requests: Vec::new(),
                detach_requests: Vec::new(),
                attach_max_size_per_document: 0,
                attach_schema_rules: Vec::new(),
            }
        }
    }

    impl ClientTransport for FakeTransport {
        fn activate_client(
            &mut self,
            request: ActivateClientRequest,
        ) -> ClientResult<ActivateClientResponse> {
            self.activate_requests.push(request);
            Ok(ActivateClientResponse {
                client_id: self.client_id.clone(),
            })
        }

        fn deactivate_client(
            &mut self,
            request: DeactivateClientRequest,
        ) -> ClientResult<DeactivateClientResponse> {
            self.deactivate_requests.push(request);
            Ok(DeactivateClientResponse)
        }

        fn attach_document(
            &mut self,
            request: AttachDocumentRequest,
        ) -> ClientResult<AttachDocumentResponse> {
            let change_pack = request.change_pack.clone();
            self.attach_requests.push(request);
            Ok(AttachDocumentResponse {
                document_id: "document-id".to_owned(),
                change_pack,
                max_size_per_document: self.attach_max_size_per_document,
                schema_rules: self.attach_schema_rules.clone(),
            })
        }

        fn detach_document(
            &mut self,
            request: DetachDocumentRequest,
        ) -> ClientResult<DetachDocumentResponse> {
            let change_pack = request.change_pack.clone();
            self.detach_requests.push(request);
            Ok(DetachDocumentResponse { change_pack })
        }
    }

    #[test]
    fn creates_client_with_default_options() {
        let client = Client::default();

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
        assert_eq!(None, attach_options.sync_mode);
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
            sync_mode: Some(SyncMode::Polling),
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

        assert_eq!(Some(SyncMode::Polling), attach_options.sync_mode);
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

        let client = Client::new(options.clone());

        assert_eq!("client-key", client.key());
        assert_eq!(&options, client.options());
        assert_eq!(
            Some("local"),
            client.options().metadata.get("region").map(String::as_str)
        );
    }

    #[test]
    fn activates_client_through_transport() -> ClientResult<()> {
        let mut client = Client::new(ClientOptions {
            key: Some("client-key".to_owned()),
            api_key: "api-key".to_owned(),
            metadata: BTreeMap::from([("region".to_owned(), "local".to_owned())]),
            ..ClientOptions::default()
        });
        let mut transport = FakeTransport::default();

        client.activate(&mut transport)?;
        client.activate(&mut transport)?;

        assert_eq!(ClientStatus::Activated, client.status());
        assert!(client.is_active());
        assert_eq!(
            Some("000000000000000000000001"),
            client.id().map(|id| id.as_str())
        );
        assert!(client.condition(ClientCondition::SyncLoop));
        assert_eq!(1, transport.activate_requests.len());
        assert_eq!("client-key", transport.activate_requests[0].client_key);
        assert_eq!(
            "api-key/client-key",
            transport.activate_requests[0].shard_key
        );
        assert_eq!(
            Some("local"),
            transport.activate_requests[0]
                .metadata
                .get("region")
                .map(String::as_str)
        );
        Ok(())
    }

    #[test]
    fn deactivates_client_through_transport() -> ClientResult<()> {
        let mut client = Client::new(ClientOptions {
            key: Some("client-key".to_owned()),
            api_key: "api-key".to_owned(),
            ..ClientOptions::default()
        });
        let mut transport = FakeTransport::default();
        let mut doc = Document::new("doc-key");

        client.activate(&mut transport)?;
        client.attach(&mut transport, &mut doc, AttachOptions::default())?;
        client.deactivate(
            &mut transport,
            DeactivateOptions {
                keepalive: true,
                synchronous: true,
            },
        )?;
        client.deactivate(&mut transport, DeactivateOptions::default())?;

        assert_eq!(ClientStatus::Deactivated, client.status());
        assert!(!client.is_active());
        assert_eq!(
            Some("000000000000000000000001"),
            client.id().map(|id| id.as_str())
        );
        assert!(!client.condition(ClientCondition::SyncLoop));
        assert!(!client.condition(ClientCondition::WatchLoop));
        assert!(!client.has("doc-key"));
        assert_eq!(1, transport.deactivate_requests.len());
        assert_eq!(
            "000000000000000000000001",
            transport.deactivate_requests[0].client_id.as_str()
        );
        assert!(transport.deactivate_requests[0].synchronous);
        assert_eq!(
            "api-key/client-key",
            transport.deactivate_requests[0].shard_key
        );
        Ok(())
    }

    #[test]
    fn rejects_attach_when_client_is_not_active() {
        let mut client = Client::new(ClientOptions {
            key: Some("client-key".to_owned()),
            ..ClientOptions::default()
        });
        let mut transport = FakeTransport::default();
        let mut doc = Document::new("doc-key");

        let err = client
            .attach(&mut transport, &mut doc, AttachOptions::default())
            .unwrap_err();

        assert_eq!(
            ClientError::ClientNotActivated("client-key".to_owned()),
            err
        );
        assert_eq!(DocStatus::Detached, doc.status());
        assert!(!client.has("doc-key"));
        assert!(transport.attach_requests.is_empty());
    }

    #[test]
    fn attaches_document_through_transport_and_records_lifecycle_state() -> ClientResult<()> {
        let mut client = Client::new(ClientOptions {
            key: Some("client-key".to_owned()),
            api_key: "api-key".to_owned(),
            ..ClientOptions::default()
        });
        let mut transport = FakeTransport::default();
        let mut doc = Document::new("doc-key");
        let mut initial_root = JsonObject::new();
        initial_root.set("title", "hello")?;

        client.apply_activation("000000000000000000000001");
        client.attach(
            &mut transport,
            &mut doc,
            AttachOptions {
                initial_root: Some(initial_root),
                sync_mode: Some(SyncMode::Polling),
                schema: Some("schema".to_owned()),
                ..AttachOptions::default()
            },
        )?;

        assert_eq!(
            Some("000000000000000000000001"),
            client.id().map(|id| id.as_str())
        );
        assert!(client.has("doc-key"));
        assert_eq!(DocStatus::Attached, doc.status());
        assert!(doc.is_attached());
        assert_eq!("000000000000000000000001", doc.actor_id().as_str());
        assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());
        assert_eq!(0, doc.max_size_per_document());
        assert!(doc.schema_rules().is_empty());
        let attachment = client.attachments.get("doc-key").unwrap();
        assert_eq!("document-id", attachment.resource_id);
        assert_eq!(SyncMode::Polling, attachment.sync_mode);
        assert_eq!(
            Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS),
            attachment.poll_interval
        );
        assert!(!attachment.poll_interval_pinned);
        assert!(!client.condition(ClientCondition::WatchLoop));
        assert_eq!(1, transport.attach_requests.len());
        assert_eq!(
            "000000000000000000000001",
            transport.attach_requests[0].client_id.as_str()
        );
        assert_eq!("api-key/doc-key", transport.attach_requests[0].shard_key);
        assert_eq!(
            Some("schema"),
            transport.attach_requests[0].schema_key.as_deref()
        );
        assert_eq!(
            "doc-key",
            transport.attach_requests[0].change_pack.document_key()
        );

        client.detach(&mut transport, &mut doc, DetachOptions)?;

        assert!(!client.has("doc-key"));
        assert_eq!(DocStatus::Detached, doc.status());
        assert_eq!("000000000000000000000000", doc.actor_id().as_str());
        assert_eq!(1, transport.detach_requests.len());
        assert_eq!(
            "000000000000000000000001",
            transport.detach_requests[0].client_id.as_str()
        );
        assert_eq!("document-id", transport.detach_requests[0].document_id);
        assert_eq!(
            "doc-key",
            transport.detach_requests[0].change_pack.document_key()
        );
        assert!(!transport.detach_requests[0].remove_if_not_attached);
        assert_eq!("api-key/doc-key", transport.detach_requests[0].shard_key);
        Ok(())
    }

    #[test]
    fn rejects_attach_when_document_is_not_detached() {
        let mut client = Client::new(ClientOptions {
            key: Some("client-key".to_owned()),
            ..ClientOptions::default()
        });
        let mut transport = FakeTransport::default();
        let mut doc = Document::new("doc-key");

        client.apply_activation("000000000000000000000001");
        doc.apply_status(DocStatus::Attached);

        let err = client
            .attach(&mut transport, &mut doc, AttachOptions::default())
            .unwrap_err();

        assert_eq!(ClientError::NotDetached("doc-key".to_owned()), err);
        assert!(transport.attach_requests.is_empty());
    }

    #[test]
    fn changes_document_sync_mode_and_validates_poll_interval() -> ClientResult<()> {
        let mut client = Client::new(ClientOptions {
            key: Some("client-key".to_owned()),
            ..ClientOptions::default()
        });
        let mut transport = FakeTransport {
            attach_max_size_per_document: 4096,
            attach_schema_rules: vec![SchemaRule::new(
                "$.profile",
                "object",
                vec![TreeNodeRule::new("paragraph", "text*", "bold", "block")],
            )],
            ..FakeTransport::default()
        };
        let mut doc = Document::new("doc-key");

        client.apply_activation("000000000000000000000001");
        client.attach(
            &mut transport,
            &mut doc,
            AttachOptions {
                sync_mode: Some(SyncMode::Realtime),
                ..AttachOptions::default()
            },
        )?;

        assert!(client.condition(ClientCondition::WatchLoop));
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
        client.change_sync_mode(&doc, SyncMode::Polling)?;

        assert!(!client.condition(ClientCondition::WatchLoop));
        let attachment = client.attachments.get("doc-key").unwrap();
        assert_eq!(SyncMode::Polling, attachment.sync_mode);
        assert_eq!(
            Duration::from_millis(DEFAULT_POLLING_INTERVAL_MS),
            attachment.poll_interval
        );

        let mut other_doc = Document::new("other-doc");
        let err = client
            .attach(
                &mut transport,
                &mut other_doc,
                AttachOptions {
                    document_poll_interval: Some(Duration::ZERO),
                    ..AttachOptions::default()
                },
            )
            .unwrap_err();
        assert_eq!(
            ClientError::InvalidArgument(
                "document_poll_interval must be greater than 0".to_owned()
            ),
            err
        );
        Ok(())
    }
}
