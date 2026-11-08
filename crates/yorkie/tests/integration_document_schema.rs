//! Integration tests for schema validation of local updates against a live
//! server.
//!
//! JS reference: `packages/sdk/test/integration/document_schema_test.ts`.
//! JS attaches with a schema created through the admin API, which is out of
//! scope here: these set the same rules with `Document::set_schema_rules`
//! after attaching. The cases that only exercise the admin API
//! (`UpdateDocument`, attaching with an unknown schema) are not ported.
//! Run with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::tree::{element, text, TwoClients};
use yorkie::{JsonTree, SchemaRule, SharedDocument, TreeNodeRule, YorkieError};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

#[tokio::test]
async fn rejects_local_update_that_violates_schema() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "schema-reject").await;
    t.d1.lock()
        .unwrap()
        .set_schema_rules(vec![SchemaRule::new("$.title", "string", Vec::new())]);

    let err =
        t.d1.lock()
            .unwrap()
            .update(|root| root.set("title", 123).map(|_| ()))
            .unwrap_err();
    assert_eq!(
        "schema validation failed: expected string at path $.title",
        err.to_string()
    );
    assert_eq!("{}", sorted_json(&t.d1));

    t.d1.lock()
        .unwrap()
        .update(|root| root.set("title", "hello").map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"title":"hello"}"#, sorted_json(&t.d1));

    // Only the accepted update reaches the other client.
    t.sync_all().await;
    assert_eq!(r#"{"title":"hello"}"#, sorted_json(&t.d2));
    t.deactivate().await;
}

#[tokio::test]
async fn validates_tree_structure_with_tree_node_rules() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "schema-tree").await;
    t.d1.lock().unwrap().set_schema_rules(vec![SchemaRule::new(
        "$.content",
        "yorkie.Tree",
        vec![
            TreeNodeRule::new("doc", "block+", "", ""),
            TreeNodeRule::new("paragraph", "text*", "bold italic", "block"),
            TreeNodeRule::new("heading", "text*", "bold", "block"),
            TreeNodeRule::new("text", "", "", ""),
        ],
    )]);
    let edit = |contents| {
        t.d1.lock().unwrap().update(|root| {
            root.get_tree_mut("content")?
                .edit(0, 0, vec![contents], 0)
                .map(|_| ())
        })
    };

    // Valid: doc > paragraph > text.
    t.d1.lock()
        .unwrap()
        .update(|root| {
            let tree = JsonTree::new(element(
                "doc",
                vec![element("paragraph", vec![text("Hello")])],
            ));
            root.set("content", tree).map(|_| ())
        })
        .unwrap();
    assert!(sorted_json(&t.d1).contains("content"));

    // Valid: a heading before the paragraph.
    edit(element("heading", vec![text("Title")])).unwrap();

    // Invalid: an unknown node type under doc.
    let err = edit(element("div", vec![text("invalid")])).unwrap_err();
    assert_eq!(
        YorkieError::SchemaValidationFailed(vec!["Unknown node type: \"div\"".to_owned()]),
        err
    );

    // Invalid: text directly under doc, which takes only blocks.
    let err = edit(text("text under doc")).unwrap_err();
    assert!(
        matches!(err, YorkieError::SchemaValidationFailed(_)),
        "{err:?}"
    );

    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    t.deactivate().await;
}
