//! Tree helpers shared by the tree integration tests: tree builders and
//! readers, and a `TwoClients` constructor that starts from a synced tree.

use super::two_clients::TwoClients;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use yorkie::{DocEvent, JsonTree, JsonValue, OperationInfo, SharedDocument, TreeNodeInput};

impl TwoClients {
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
