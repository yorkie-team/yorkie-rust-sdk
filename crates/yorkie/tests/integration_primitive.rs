//! Integration tests for primitive values.
//!
//! JS reference: `packages/sdk/test/integration/primitive_test.ts`. The first
//! two cases edit a local document only, so they run without a server. Run
//! the rest with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p
//! yorkie`.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::two_clients::TwoClients;
use yorkie::{Checkpoint, Document, JsonObject, JsonValue, SharedDocument, YorkieError};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

/// JS "should apply updates of string" (primitive_test.ts:24).
#[test]
fn applies_updates_of_string() {
    let mut doc1 = Document::new("test-doc");

    assert_eq!(Checkpoint::initial(), doc1.checkpoint());
    assert!(!doc1.has_local_changes());

    doc1.update(|root| {
        root.set("k1", "v1")?;
        root.set("k2", "v2")?;
        assert_eq!(Some(&JsonValue::String("v1".to_owned())), root.get("k1"));
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":"v1","k2":"v2"}"#, doc1.to_sorted_json());

    assert!(doc1.has_local_changes());
}

/// JS "can rollback, primitive deepcopy" (primitive_test.ts:42): an update
/// that fails leaves the document as it was.
#[test]
fn rolls_back_a_failed_update() {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("k1", JsonObject::new())?;
        let k1 = root.get_object_mut("k1")?;
        k1.set("k1-1", 1)?;
        k1.set("k1-2", 2)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":{"k1-1":1,"k1-2":2}}"#, doc.to_sorted_json());

    let result = doc.update(|root| {
        root.get_object_mut("k1")?.remove("k1-1");
        Err(YorkieError::InvalidArgument("dummy error".to_owned()))
    });
    assert!(result.is_err());
    assert_eq!(r#"{"k1":{"k1-1":1,"k1-2":2}}"#, doc.to_sorted_json());
}

/// JS "Can handle primitive types" (primitive_test.ts:63). JS also sets a
/// `Uint8Array` (k6), a `Date` (k7) and `undefined` (k8); the Rust
/// `JsonValue` has no bytes, date or undefined variant, so those are left
/// out.
#[tokio::test]
async fn syncs_primitive_types() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "primitive-types").await;

    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.set("k0", ())?;
            root.set("k1", true)?;
            root.set("k2", 2147483647)?;
            root.set("k3", 9223372036854775807i64)?;
            root.set("k4", 1.79)?;
            root.set("k5", "4")?;
            Ok(())
        })
        .unwrap();

    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "should sync out-of-int32 integers without truncation"
/// (primitive_test.ts:93). JS stores `2 ** 31` as a Long because it does not
/// fit an Integer; in Rust the caller picks the type with `i64`.
#[tokio::test]
async fn syncs_out_of_int32_integers_without_truncation() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "primitive-out-of-int32").await;

    t.d1.lock()
        .unwrap()
        .update(|root| root.set("ts", 2i64.pow(31)).map(|_| ()))
        .unwrap();

    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    assert_eq!(r#"{"ts":2147483648}"#, sorted_json(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    assert_eq!(
        Some(&JsonValue::Long(2147483648)),
        t.d2.lock().unwrap().get_root().get("ts")
    );

    t.deactivate().await;
}
