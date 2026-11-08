//! Tree helpers shared by the tree integration tests: two clients on one
//! document in manual sync mode (JS `withTwoClientsAndDocuments`), and tree
//! builders and readers.

use super::unique_key;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use yorkie::{
    AttachOptions, Client, ClientOptions, DeactivateOptions, DocEvent, Document, JsonTree,
    JsonValue, OperationInfo, SharedDocument, SyncMode, TreeNodeInput,
};

pub struct TwoClients {
    pub c1: Client,
    pub d1: SharedDocument,
    pub c2: Client,
    pub d2: SharedDocument,
}

impl TwoClients {
    /// Attaches one document to two clients in manual sync mode (JS
    /// `withTwoClientsAndDocuments`).
    pub async fn attach(addr: &str, name: &str) -> Self {
        let c1 = active_client(addr).await;
        let c2 = active_client(addr).await;
        let (d1, d2) = attach_pair(&c1, &c2, &unique_key(name)).await;
        Self { c1, d1, c2, d2 }
    }

    /// Attaches, sets the tree at `t` from d1, and syncs it to d2.
    pub async fn with_tree(addr: &str, name: &str, root: TreeNodeInput, expected: &str) -> Self {
        let t = Self::attach(addr, name).await;
        set_tree(&t.d1, root);
        t.c1.sync(None).await.unwrap();
        t.c2.sync(None).await.unwrap();
        assert_eq!(expected, xml(&t.d1));
        assert_eq!(expected, xml(&t.d2));
        t
    }

    /// Syncs c1, c2, then c1 again, so both have every change.
    pub async fn sync_all(&self) {
        self.c1.sync(None).await.unwrap();
        self.c2.sync(None).await.unwrap();
        self.c1.sync(None).await.unwrap();
    }

    pub async fn deactivate(self) {
        for client in [self.c1, self.c2] {
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

pub async fn attach_pair(c1: &Client, c2: &Client, key: &str) -> (SharedDocument, SharedDocument) {
    let d1 = c1.attach(shared(key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(key), manual_attach()).await.unwrap();
    (d1, d2)
}

pub fn shared(key: &str) -> SharedDocument {
    Arc::new(Mutex::new(Document::new(key)))
}

pub fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

pub fn element_with(node_type: &str, attrs: Value, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element_with_attributes(node_type, attributes(attrs), children)
}

pub fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

pub fn attributes(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).expect("attributes are an object")
}

pub fn set_tree(doc: &SharedDocument, root: TreeNodeInput) {
    doc.lock()
        .unwrap()
        .update(|root_object| root_object.set_tree("t", root).map(|_| ()))
        .unwrap();
}

/// Runs `edit` on the tree at `t` inside one update.
pub fn edit_tree(doc: &SharedDocument, edit: impl FnOnce(&mut JsonTree) -> yorkie::Result<()>) {
    doc.lock()
        .unwrap()
        .update(|root| edit(root.get_tree_mut("t")?))
        .unwrap();
}

pub fn xml(doc: &SharedDocument) -> String {
    let doc = doc.lock().unwrap();
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().unwrap(),
        other => panic!("expected a tree, got {other:?}"),
    }
}

/// Collects the op infos of every local and remote change under `$.t` (JS
/// test helper `subscribeDocs`).
pub fn collect_ops(doc: &SharedDocument) -> Arc<Mutex<Vec<OperationInfo>>> {
    let ops = Arc::new(Mutex::new(Vec::new()));
    let sink = ops.clone();
    doc.lock().unwrap().subscribe_path("$.t", move |event| {
        if let DocEvent::LocalChange(info) | DocEvent::RemoteChange(info) = event {
            sink.lock().unwrap().extend(info.operations.iter().cloned());
        }
    });
    ops
}
