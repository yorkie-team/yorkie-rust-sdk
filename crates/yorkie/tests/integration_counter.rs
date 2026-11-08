//! Integration tests for counters.
//!
//! JS reference: `packages/sdk/test/integration/counter_test.ts`. The cases
//! that edit a local document only run without a server. Run the rest with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.
//!
//! JS `new Counter(n)` makes an Integer counter from a number and a Long
//! counter from a bigint; the Rust counterparts are `JsonCounter::integer`
//! (`i32`) and `JsonCounter::long` (`i64`).

mod integration_support;

use integration_support::two_clients::{active_client, manual_attach, shared, TwoClients};
use integration_support::{assert_undo_redo, rpc_addr, unique_key};
use yorkie::{
    CounterValue, DeactivateOptions, Document, JsonCounter, JsonObject, JsonValue, SharedDocument,
};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

fn counter_value(object: &JsonObject, key: &str) -> CounterValue {
    match object.get(key) {
        Some(JsonValue::Counter(counter)) => counter.value(),
        other => panic!("expected a counter at {key}, got {other:?}"),
    }
}

fn k1(doc: &Document) -> &JsonObject {
    match doc.get_root().get("k1") {
        Some(JsonValue::Object(object)) => object,
        other => panic!("expected an object at k1, got {other:?}"),
    }
}

/// JS "can be increased by Counter type" (counter_test.ts:29). JS builds
/// `length` from `new Counter(10.5)`, which an Integer counter truncates to
/// 10; the Rust constructor takes the `i32` directly. JS also checks that
/// `increase(true)` throws; the Rust signature rejects a bool at compile
/// time.
#[test]
fn increases_counters() {
    let mut doc = Document::new("counter-increase");
    let mut states = Vec::new();

    doc.update(|root| {
        root.set("k1", JsonObject::new())?;
        let k1 = root.get_object_mut("k1")?;
        k1.set("age", JsonCounter::integer(1))?;
        k1.set("length", JsonCounter::integer(10))?;
        k1.get_counter_mut("age")?.increase(5)?;
        k1.get_counter_mut("length")?.increase(3.5)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":{"age":6,"length":13}}"#, doc.to_sorted_json());
    states.push(doc.to_sorted_json());
    assert_eq!(CounterValue::Integer(6), counter_value(k1(&doc), "age"));
    assert_eq!(CounterValue::Integer(13), counter_value(k1(&doc), "length"));

    doc.update(|root| {
        let k1 = root.get_object_mut("k1")?;
        k1.get_counter_mut("age")?.increase(1.5)?.increase(1)?;
        k1.get_counter_mut("length")?.increase(3.5)?.increase(1)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":{"age":8,"length":17}}"#, doc.to_sorted_json());
    states.push(doc.to_sorted_json());
    assert_eq!(CounterValue::Integer(8), counter_value(k1(&doc), "age"));
    assert_eq!(CounterValue::Integer(17), counter_value(k1(&doc), "length"));

    assert_undo_redo(&mut doc, &states);
}

/// JS "Can handle increase operation" (counter_test.ts:70).
#[tokio::test]
async fn syncs_an_increase() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "counter-increase-sync").await;

    {
        let mut d1 = t.d1.lock().unwrap();
        d1.update(|root| root.set("age", JsonCounter::integer(0)).map(|_| ()))
            .unwrap();
        d1.update(|root| {
            root.get_counter_mut("age")?.increase(1)?.increase(2)?;
            root.set("length", JsonCounter::integer(10))?;
            Ok(())
        })
        .unwrap();
    }

    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent increase operation" (counter_test.ts:87). JS
/// resets `width` with `new Counter(2.5)`, an Integer counter of 2.
#[tokio::test]
#[allow(clippy::approx_constant)] // JS increases by the literal 3.14.
async fn converges_after_concurrent_increases() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "counter-concurrent-increase").await;

    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.set("age", JsonCounter::integer(0))?;
            root.set("width", JsonCounter::integer(0))?;
            root.set("height", JsonCounter::integer(0))?;
            Ok(())
        })
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.get_counter_mut("age")?.increase(1)?.increase(2)?;
            root.get_counter_mut("width")?.increase(10)?;
            Ok(())
        })
        .unwrap();
    t.d2.lock()
        .unwrap()
        .update(|root| {
            root.get_counter_mut("age")?.increase(3.14)?.increase(2)?;
            root.set("width", JsonCounter::integer(2))?;
            Ok(())
        })
        .unwrap();
    t.sync_all().await;

    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "can handle overflow" (counter_test.ts:118). JS also builds counters
/// from `2147483648` (a number, so an Integer counter that wraps it) and
/// `9223372036854775808n` (a bigint the Long counter wraps); neither fits
/// the `i32` or `i64` the Rust constructors take, so those two steps are
/// left out.
#[test]
fn wraps_on_overflow() {
    let mut doc = Document::new("counter-overflow");
    doc.update(|root| {
        root.set("age", JsonCounter::integer(2147483647))?;
        root.get_counter_mut("age")?.increase(1)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"age":-2147483648}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.set("age", JsonCounter::long(9223372036854775807))?;
        root.get_counter_mut("age")?.increase(1)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"age":-9223372036854775808}"#, doc.to_sorted_json());
}

/// JS "Can handle increase operation with out-of-int32 Long"
/// (counter_test.ts:144).
#[tokio::test]
async fn syncs_an_out_of_int32_long_increase() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "counter-out-of-int32").await;

    t.d1.lock()
        .unwrap()
        .update(|root| root.set("age", JsonCounter::integer(1)).map(|_| ()))
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.get_counter_mut("age")?.increase(5_000_000_000i64)?;
            Ok(())
        })
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();

    // 1 + 5_000_000_000 wraps to 705032705 under int32 arithmetic.
    assert_eq!(r#"{"age":705032705}"#, sorted_json(&t.d1));
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "can get proper reverse operations" (counter_test.ts:168). JS also
/// compares the undo and redo stacks with `getUndoStackForTest` and
/// `getRedoStackForTest`, which the Rust public API does not expose; the
/// core unit tests check the reverse increase operations instead.
#[test]
fn undoes_integer_and_long_increases() {
    let mut doc = Document::new("counter-reverse");

    doc.update(|root| {
        root.set("cnt", JsonCounter::integer(0))?;
        root.set("longCnt", JsonCounter::long(0))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"cnt":0,"longCnt":0}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.get_counter_mut("cnt")?.increase(1.5)?;
        root.get_counter_mut("longCnt")?
            .increase(9223372036854775807i64)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        r#"{"cnt":1,"longCnt":9223372036854775807}"#,
        doc.to_sorted_json()
    );

    doc.history_mut().undo().unwrap();
    assert_eq!(r#"{"cnt":0,"longCnt":0}"#, doc.to_sorted_json());
}

/// JS "Can undo/redo for increase operation" (counter_test.ts:197).
#[test]
fn undoes_and_redoes_an_increase() {
    let mut doc = Document::new("counter-undo-redo");
    doc.update(|root| root.set("counter", JsonCounter::integer(100)).map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());

    doc.update(|root| root.get_counter_mut("counter")?.increase(1).map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());

    doc.history_mut().undo().unwrap();
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());

    doc.history_mut().redo().unwrap();
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());

    doc.history_mut().undo().unwrap();
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());
}

/// JS "should handle undo/redo for long type and overflow"
/// (counter_test.ts:221).
#[test]
fn undoes_and_redoes_long_and_overflowing_increases() {
    let mut doc = Document::new("counter-undo-overflow");
    let mut states = Vec::new();

    doc.update(|root| {
        root.set("cnt", JsonCounter::integer(0))?;
        root.set("longCnt", JsonCounter::long(0))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"cnt":0,"longCnt":0}"#, doc.to_sorted_json());
    states.push(doc.to_sorted_json());

    doc.update(|root| {
        root.get_counter_mut("cnt")?.increase(2147483647)?;
        root.get_counter_mut("longCnt")?
            .increase(9223372036854775807i64)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        r#"{"cnt":2147483647,"longCnt":9223372036854775807}"#,
        doc.to_sorted_json()
    );
    states.push(doc.to_sorted_json());

    doc.update(|root| {
        root.get_counter_mut("cnt")?.increase(1)?;
        root.get_counter_mut("longCnt")?.increase(1i64)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        r#"{"cnt":-2147483648,"longCnt":-9223372036854775808}"#,
        doc.to_sorted_json()
    );
    states.push(doc.to_sorted_json());

    assert_undo_redo(&mut doc, &states);
}

/// JS "Can undo/redo for concurrent users" (counter_test.ts:256): client2
/// attaches after client1 has pushed the counter, and only client1 can redo
/// its own undo.
#[tokio::test]
async fn undoes_an_increase_while_another_client_increases() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("counter-undo-concurrent");
    let client1 = active_client(&addr).await;
    let client2 = active_client(&addr).await;

    let doc1 = client1.attach(shared(&key), manual_attach()).await.unwrap();
    doc1.lock()
        .unwrap()
        .update(|root| root.set("counter", JsonCounter::integer(100)).map(|_| ()))
        .unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":100}"#, sorted_json(&doc1));

    let doc2 = client2.attach(shared(&key), manual_attach()).await.unwrap();
    assert_eq!(r#"{"counter":100}"#, sorted_json(&doc2));

    let increase = |doc: &SharedDocument, by: i32| {
        doc.lock()
            .unwrap()
            .update(|root| root.get_counter_mut("counter")?.increase(by).map(|_| ()))
            .unwrap();
    };
    increase(&doc1, 1);
    increase(&doc2, 2);
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    client1.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":103}"#, sorted_json(&doc1));
    assert_eq!(r#"{"counter":103}"#, sorted_json(&doc2));

    doc1.lock().unwrap().history_mut().undo().unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":102}"#, sorted_json(&doc1));
    assert_eq!(r#"{"counter":102}"#, sorted_json(&doc2));

    // Only client1 can redo the undone operation.
    assert!(doc1.lock().unwrap().history().can_redo());
    assert!(!doc2.lock().unwrap().history().can_redo());

    doc1.lock().unwrap().history_mut().redo().unwrap();
    client1.sync(None).await.unwrap();
    client2.sync(None).await.unwrap();
    assert_eq!(r#"{"counter":103}"#, sorted_json(&doc1));
    assert_eq!(r#"{"counter":103}"#, sorted_json(&doc2));

    for client in [client1, client2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}
