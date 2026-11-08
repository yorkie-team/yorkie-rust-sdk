//! Integration tests for arrays.
//!
//! JS reference: `packages/sdk/test/integration/array_test.ts`. The cases
//! that edit a local document only run without a server. Run the rest with
//! `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.
//!
//! JS `root.toJSON()` and `doc.toJSON()` are `to_sorted_json` here; the
//! documents in this file have one key, so the order is the same.

mod integration_support;

use integration_support::rpc_addr;
use integration_support::two_clients::TwoClients;
use yorkie::{Document, JsonArray, JsonObject, JsonValue, SharedDocument, TimeTicket};

fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

/// The ID of the element at `index` (JS `getElementByIndex(index).getID()`).
fn id_at(array: &JsonArray, index: usize) -> TimeTicket {
    array
        .get_element_by_index(index)
        .map(|element| element.id().clone())
        .unwrap_or_else(|| panic!("no element at {index}"))
}

/// Runs `edit` on the array at `key` inside one update, then checks the root
/// against `expected` when given (JS `root.toJSON()` inside the update).
fn edit_array(
    doc: &SharedDocument,
    key: &str,
    expected: Option<&str>,
    edit: impl FnOnce(&mut JsonArray) -> yorkie::Result<()>,
) {
    doc.lock()
        .unwrap()
        .update(|root| {
            edit(root.get_array_mut(key)?)?;
            if let Some(expected) = expected {
                assert_eq!(expected, root.to_sorted_json());
            }
            Ok(())
        })
        .unwrap();
}

fn set_array(doc: &SharedDocument, key: &str, values: Vec<i32>, expected: Option<&str>) {
    doc.lock()
        .unwrap()
        .update(|root| {
            root.set(key, values)?;
            if let Some(expected) = expected {
                assert_eq!(expected, root.to_sorted_json());
            }
            Ok(())
        })
        .unwrap();
}

fn array_len(doc: &SharedDocument, key: &str) -> usize {
    match doc.lock().unwrap().get_root().get(key) {
        Some(JsonValue::Array(array)) => array.len(),
        other => panic!("expected an array at {key}, got {other:?}"),
    }
}

/// Syncs c1, c2, then c1 again, and reports whether the replicas converged
/// (JS `syncClientsThenCheckEqual` for two pairs).
async fn sync_clients_then_check_equal(t: &TwoClients) -> bool {
    t.sync_all().await;
    sorted_json(&t.d1) == sorted_json(&t.d2)
}

/// JS "should handle delete operations" (array_test.ts:30).
#[test]
fn handles_delete_operations() {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| root.set("k1", vec!["1", "2", "3"]).map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"k1":["1","2","3"]}"#, doc.to_sorted_json());

    doc.update(|root| {
        let k1 = root.get_array_mut("k1")?;
        k1.delete(1);
        k1.push("4")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"k1":["1","3","4"]}"#, doc.to_sorted_json());
}

/// The setup shared by the two "can push ... after delete operation" cases.
fn doc_with_deleted_element() -> Document {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| root.set("k1", vec!["1", "2", "3"]).map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"k1":["1","2","3"]}"#, doc.to_sorted_json());

    doc.update(|root| {
        let k1 = root.get_array_mut("k1")?;
        k1.delete(1);
        k1.push("4")?;
        Ok(())
    })
    .unwrap();
    doc
}

/// JS "can push array element after delete operation" (array_test.ts:46).
#[test]
fn pushes_an_array_after_a_delete() {
    let mut doc = doc_with_deleted_element();

    doc.update(|root| {
        root.get_array_mut("k1")?.push(vec![4, 5, 6])?;
        assert_eq!(r#"{"k1":["1","3","4",[4,5,6]]}"#, root.to_sorted_json());
        Ok(())
    })
    .unwrap();

    assert_eq!(r#"{"k1":["1","3","4",[4,5,6]]}"#, doc.to_sorted_json());
}

/// JS "can push object element after delete operation" (array_test.ts:68).
#[test]
fn pushes_an_object_after_a_delete() {
    let mut doc = doc_with_deleted_element();

    doc.update(|root| {
        let mut object = JsonObject::new();
        object.set("a", "1")?;
        object.set("b", "2")?;
        root.get_array_mut("k1")?.push(object)?;
        assert_eq!(
            r#"{"k1":["1","3","4",{"a":"1","b":"2"}]}"#,
            root.to_sorted_json()
        );
        Ok(())
    })
    .unwrap();

    assert_eq!(
        r#"{"k1":["1","3","4",{"a":"1","b":"2"}]}"#,
        doc.to_sorted_json()
    );
}

/// JS "can push array" (array_test.ts:92).
#[test]
fn pushes_an_array() {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("arr", vec![1, 2, 3])?;
        root.get_array_mut("arr")?.push(vec![4, 5, 6])?;
        assert_eq!(r#"{"arr":[1,2,3,[4,5,6]]}"#, root.to_sorted_json());
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"arr":[1,2,3,[4,5,6]]}"#, doc.to_sorted_json());
}

/// JS "can push element then delete it by ID in array" (array_test.ts:105).
/// JS `push` returns the new length; Rust `push` returns the array, so the
/// length is read after each push.
#[test]
fn pushes_then_deletes_by_id() {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    let mut target = None;
    doc.update(|root| {
        root.set("list", JsonArray::new())?;
        let list = root.get_array_mut("list")?;
        for (value, len) in [(4, 1), (3, 2), (2, 3), (1, 4)] {
            list.push(value)?;
            assert_eq!(len, list.len());
        }
        target = Some(id_at(list, 2));
        Ok(())
    })
    .unwrap();
    let target = target.unwrap();

    assert_eq!(r#"{"list":[4,3,2,1]}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.get_array_mut("list")?.delete_by_id(&target);
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"list":[4,3,1]}"#, doc.to_sorted_json());

    doc.update(|root| {
        let list = root.get_array_mut("list")?;
        list.push(2)?;
        assert_eq!(4, list.len());
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"list":[4,3,1,2]}"#, doc.to_sorted_json());
}

/// JS "can insert an element after the given element in array"
/// (array_test.ts:132).
#[test]
fn inserts_after_an_element() {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());

    let mut prev = None;
    doc.update(|root| {
        root.set("list", JsonArray::new())?;
        let list = root.get_array_mut("list")?;
        list.push(1)?;
        list.push(2)?;
        list.push(4)?;
        prev = Some(id_at(list, 1));
        Ok(())
    })
    .unwrap();
    let prev = prev.unwrap();

    assert_eq!(r#"{"list":[1,2,4]}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.get_array_mut("list")?
            .insert_after(&prev, 3)
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"list":[1,2,3,4]}"#, doc.to_sorted_json());

    doc.update(|root| {
        root.get_array_mut("list")?.delete(1);
        Ok(())
    })
    .unwrap();
    assert_eq!(r#"{"list":[1,3,4]}"#, doc.to_sorted_json());

    doc.update(|root| {
        let list = root.get_array_mut("list")?;
        let prev = id_at(list, 0);
        list.insert_after(&prev, 2).map(|_| ())
    })
    .unwrap();
    assert_eq!(r#"{"list":[1,2,3,4]}"#, doc.to_sorted_json());

    let Some(JsonValue::Array(list)) = doc.get_root().get("list") else {
        panic!("expected an array at list");
    };
    for idx in 0..list.len() {
        assert_eq!(
            Some(&JsonValue::Integer(idx as i32 + 1)),
            list.get(idx),
            "list[{idx}]"
        );
    }
}

/// A local document with `list` set to `[0, 1, 2]`.
fn doc_with_list() -> Document {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());
    doc.update(|root| root.set("list", vec![0, 1, 2]).map(|_| ()))
        .unwrap();
    doc
}

/// Runs `edit` on `list` inside one update and checks the root after it.
fn edit_list(
    doc: &mut Document,
    expected: &str,
    edit: impl FnOnce(&mut JsonArray) -> yorkie::Result<()>,
) {
    doc.update(|root| {
        edit(root.get_array_mut("list")?)?;
        assert_eq!(expected, root.to_sorted_json());
        Ok(())
    })
    .unwrap();
}

/// JS "can move an element before the given element in array"
/// (array_test.ts:169).
#[test]
fn moves_before_an_element() {
    let mut doc = doc_with_list();
    for expected in [r#"{"list":[2,0,1]}"#, r#"{"list":[1,2,0]}"#] {
        edit_list(&mut doc, expected, |list| {
            let (next, item) = (id_at(list, 0), id_at(list, 2));
            list.move_before(&next, &item).map(|_| ())
        });
    }
}

/// JS "can move an element after the given element in array"
/// (array_test.ts:192).
#[test]
fn moves_after_an_element() {
    let mut doc = doc_with_list();
    for expected in [r#"{"list":[0,2,1]}"#, r#"{"list":[0,1,2]}"#] {
        edit_list(&mut doc, expected, |list| {
            let (prev, item) = (id_at(list, 0), id_at(list, 2));
            list.move_after(&prev, &item).map(|_| ())
        });
    }
}

/// JS "can insert an element at the first of array" (array_test.ts:215),
/// which moves elements to the front.
#[test]
fn moves_to_the_front() {
    let mut doc = doc_with_list();
    for (index, expected) in [
        (2, r#"{"list":[2,0,1]}"#),
        (1, r#"{"list":[0,2,1]}"#),
        (0, r#"{"list":[0,2,1]}"#),
    ] {
        edit_list(&mut doc, expected, |list| {
            let item = id_at(list, index);
            list.move_front(&item).map(|_| ())
        });
    }
}

/// JS "can move an element at the last of array" (array_test.ts:242).
#[test]
fn moves_to_the_last() {
    let mut doc = doc_with_list();
    for (index, expected) in [
        (2, r#"{"list":[0,1,2]}"#),
        (1, r#"{"list":[0,2,1]}"#),
        (0, r#"{"list":[2,1,0]}"#),
    ] {
        edit_list(&mut doc, expected, |list| {
            let item = id_at(list, index);
            list.move_last(&item).map(|_| ())
        });
    }
}

/// JS "Can handle concurrent insertAfter operations" (array_test.ts:269).
#[tokio::test]
async fn converges_after_concurrent_insert_afters() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-concurrent-insert-after").await;

    let mut prev = None;
    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.set("k1", vec![1, 2, 3, 4])?;
            prev = Some(id_at(root.get_array_mut("k1")?, 1));
            Ok(())
        })
        .unwrap();
    let prev = prev.unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,3,4]}"#), |k1| {
        k1.delete_by_id(&prev);
        Ok(())
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[1,2,2,3,4]}"#), |k1| {
        k1.insert_after(&prev, 2).map(|_| ())
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    assert_eq!(r#"{"k1":[1,2,3,4]}"#, sorted_json(&t.d1));

    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,2,"2.1",3,4]}"#), |k1| {
        let prev = id_at(k1, 1);
        k1.insert_after(&prev, "2.1").map(|_| ())
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[1,2,"2.2",3,4]}"#), |k1| {
        let prev = id_at(k1, 1);
        k1.insert_after(&prev, "2.2").map(|_| ())
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// Moves the element at `item` before the one at `next`.
fn move_before_by_index(k1: &mut JsonArray, next: usize, item: usize) -> yorkie::Result<()> {
    let (next, item) = (id_at(k1, next), id_at(k1, item));
    k1.move_before(&next, &item).map(|_| ())
}

/// Moves the element at `item` after the one at `prev`.
fn move_after_by_ids(k1: &mut JsonArray, prev: usize, item: usize) -> yorkie::Result<()> {
    let (prev, item) = (id_at(k1, prev), id_at(k1, item));
    k1.move_after(&prev, &item).map(|_| ())
}

fn move_front_at(k1: &mut JsonArray, index: usize) -> yorkie::Result<()> {
    let item = id_at(k1, index);
    k1.move_front(&item).map(|_| ())
}

fn move_last_at(k1: &mut JsonArray, index: usize) -> yorkie::Result<()> {
    let item = id_at(k1, index);
    k1.move_last(&item).map(|_| ())
}

/// Two clients whose documents both hold `k1 = [0, 1, 2]`.
async fn with_012(addr: &str, name: &str) -> TwoClients {
    let t = TwoClients::attach(addr, name).await;
    set_array(&t.d1, "k1", vec![0, 1, 2], Some(r#"{"k1":[0,1,2]}"#));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    t
}

/// JS "Can handle concurrent moveBefore operations with the same position"
/// (array_test.ts:312).
#[tokio::test]
async fn converges_after_concurrent_move_befores_to_the_same_position() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_012(&addr, "array-move-before-same").await;

    edit_array(&t.d1, "k1", Some(r#"{"k1":[2,0,1]}"#), |k1| {
        move_before_by_index(k1, 0, 2)
    });
    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,2,0]}"#), |k1| {
        move_before_by_index(k1, 0, 2)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[1,0,2]}"#), |k1| {
        move_before_by_index(k1, 0, 1)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[0,1,2]}"#), |k1| {
        move_before_by_index(k1, 0, 1)
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent moveBefore operations from the different
/// position" (array_test.ts:360).
#[tokio::test]
async fn converges_after_concurrent_move_befores_from_different_positions() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_012(&addr, "array-move-before-different").await;

    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,0,2]}"#), |k1| {
        move_before_by_index(k1, 0, 1)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[0,2,1]}"#), |k1| {
        move_before_by_index(k1, 1, 2)
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent moveFront operations with the item which has
/// the different index" (array_test.ts:394).
#[tokio::test]
async fn converges_after_concurrent_move_fronts_of_different_items() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_012(&addr, "array-move-front-different").await;

    edit_array(&t.d1, "k1", Some(r#"{"k1":[2,0,1]}"#), |k1| {
        move_front_at(k1, 2)
    });
    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,2,0]}"#), |k1| {
        move_front_at(k1, 2)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[1,0,2]}"#), |k1| {
        move_front_at(k1, 1)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[0,1,2]}"#), |k1| {
        move_front_at(k1, 1)
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent moveFront operations with the item which has
/// the same index" (array_test.ts:438).
#[tokio::test]
async fn converges_after_concurrent_move_fronts_of_the_same_item() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_012(&addr, "array-move-front-same").await;

    edit_array(&t.d1, "k1", Some(r#"{"k1":[2,0,1]}"#), |k1| {
        move_front_at(k1, 2)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[2,0,1]}"#), |k1| {
        move_front_at(k1, 2)
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent moveAfter operations" (array_test.ts:470),
/// which moves elements to the last.
#[tokio::test]
async fn converges_after_concurrent_move_lasts() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_012(&addr, "array-move-last").await;

    edit_array(&t.d1, "k1", Some(r#"{"k1":[0,2,1]}"#), |k1| {
        move_last_at(k1, 1)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[1,2,0]}"#), |k1| {
        move_last_at(k1, 0)
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent insertAfter and moveBefore operations"
/// (array_test.ts:500).
#[tokio::test]
async fn converges_after_concurrent_insert_afters_and_move_befores() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-insert-after-move-before").await;

    let mut prev = None;
    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.set("k1", vec![0])?;
            prev = Some(id_at(root.get_array_mut("k1")?, 0));
            Ok(())
        })
        .unwrap();
    let prev = prev.unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d1, "k1", Some(r#"{"k1":[0,1]}"#), |k1| {
        k1.insert_after(&prev, 1).map(|_| ())
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[0,2]}"#), |k1| {
        k1.insert_after(&prev, 2).map(|_| ())
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d1, "k1", None, |k1| move_before_by_index(k1, 0, 1));
    edit_array(&t.d2, "k1", None, |k1| move_before_by_index(k1, 0, 2));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent moveAfter" (array_test.ts:548).
#[tokio::test]
async fn converges_after_concurrent_move_afters() {
    let Some(addr) = rpc_addr() else { return };
    let t = with_012(&addr, "array-move-after").await;

    edit_array(&t.d1, "k1", Some(r#"{"k1":[0,1,2]}"#), |k1| {
        move_after_by_ids(k1, 0, 1)
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[0,2,1]}"#), |k1| {
        move_after_by_ids(k1, 0, 2)
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent add operations" (array_test.ts:580).
#[tokio::test]
async fn converges_after_concurrent_pushes() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-concurrent-add").await;

    t.d1.lock()
        .unwrap()
        .update(|root| root.set("k1", vec!["1"]).map(|_| ()))
        .unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d1, "k1", None, |k1| k1.push("2").map(|_| ()));
    edit_array(&t.d2, "k1", None, |k1| k1.push("3").map(|_| ()));
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    t.deactivate().await;
}

/// JS "Can handle concurrent delete operations" (array_test.ts:603).
#[tokio::test]
async fn converges_after_concurrent_deletes_of_the_same_element() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-concurrent-delete").await;

    set_array(&t.d1, "k1", vec![1, 2, 3, 4], None);
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    for doc in [&t.d1, &t.d2] {
        edit_array(doc, "k1", None, |k1| {
            let prev = id_at(k1, 2);
            k1.delete_by_id(&prev);
            Ok(())
        });
    }
    t.sync_all().await;
    assert_eq!(3, array_len(&t.d1, "k1"));

    t.deactivate().await;
}

/// JS "Can handle concurrent insertBefore and delete operations"
/// (array_test.ts:633).
#[tokio::test]
async fn converges_after_concurrent_insert_before_and_delete() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-insert-before-delete").await;

    let mut prev = None;
    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.set("k1", vec![1])?;
            prev = Some(id_at(root.get_array_mut("k1")?, 0));
            Ok(())
        })
        .unwrap();
    let prev = prev.unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d1, "k1", Some(r#"{"k1":[]}"#), |k1| {
        k1.delete_by_id(&prev);
        assert_eq!(0, k1.len());
        Ok(())
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[2,1]}"#), |k1| {
        k1.insert_before(&prev, 2)?;
        assert_eq!(2, k1.len());
        Ok(())
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    assert_eq!(r#"{"k1":[2]}"#, sorted_json(&t.d1));
    assert_eq!(1, array_len(&t.d1, "k1"));

    t.deactivate().await;
}

/// JS "Can handle complex concurrent insertBefore and delete operations"
/// (array_test.ts:669).
#[tokio::test]
async fn converges_after_complex_concurrent_insert_befores_and_deletes() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-complex-insert-before-delete").await;

    let mut prev = None;
    t.d1.lock()
        .unwrap()
        .update(|root| {
            root.set("k1", vec![1, 2, 3, 4])?;
            prev = Some(id_at(root.get_array_mut("k1")?, 1));
            Ok(())
        })
        .unwrap();
    let prev = prev.unwrap();
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,3,4]}"#), |k1| {
        k1.delete_by_id(&prev);
        assert_eq!(3, k1.len());
        Ok(())
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[1,5,2,3,4]}"#), |k1| {
        k1.insert_before(&prev, 5)?;
        assert_eq!(5, k1.len());
        Ok(())
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    assert_eq!(r#"{"k1":[1,5,3,4]}"#, sorted_json(&t.d1));

    edit_array(&t.d1, "k1", Some(r#"{"k1":[1,5,3,6,4]}"#), |k1| {
        let prev = id_at(k1, 3);
        assert_eq!(4, k1.len());
        k1.insert_before(&prev, 6)?;
        assert_eq!(5, k1.len());
        Ok(())
    });
    edit_array(&t.d2, "k1", Some(r#"{"k1":[7,1,5,3,4]}"#), |k1| {
        let prev = id_at(k1, 0);
        assert_eq!(4, k1.len());
        k1.insert_before(&prev, 7)?;
        assert_eq!(5, k1.len());
        Ok(())
    });
    t.sync_all().await;
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    assert_eq!(r#"{"k1":[7,1,5,3,6,4]}"#, sorted_json(&t.d1));
    assert_eq!(6, array_len(&t.d1, "k1"));

    t.deactivate().await;
}

/// JS "Returns undefined when looking up an element that doesnt exist after
/// GC" (array_test.ts:725). JS collects the garbage with
/// `doc.garbageCollect(maxVectorOf([actor]))` before the lookup; the Rust
/// `Document` has no public garbage collection, and without it the lookup
/// passes trivially.
#[test]
#[ignore = "Document::garbage_collect (JS garbageCollect) is not public"]
fn finds_nothing_for_a_garbage_collected_id() {
    let mut doc = Document::new("test-doc");
    let mut target = None;

    doc.update(|root| {
        root.set("list", vec![0, 1, 2])?;
        target = Some(id_at(root.get_array_mut("list")?, 2));
        Ok(())
    })
    .unwrap();
    let target = target.unwrap();

    doc.update(|root| {
        root.get_array_mut("list")?.delete_by_id(&target);
        Ok(())
    })
    .unwrap();

    assert_eq!(r#"{"list":[0,1]}"#, doc.to_sorted_json());

    // JS: doc.garbageCollect(maxVectorOf([doc.getChangeID().getActorID()]))
    doc.update(|root| {
        assert!(root
            .get_array_mut("list")?
            .get_element_by_id(&target)
            .is_none());
        Ok(())
    })
    .unwrap();
}

/// JS "Returns undefined when looking up an element that doesnt exist"
/// (array_test.ts:747).
#[test]
fn finds_nothing_for_a_deleted_id() {
    let mut doc = Document::new("test-doc");
    let mut target = None;

    doc.update(|root| {
        root.set("list", vec![0, 1, 2])?;
        target = Some(id_at(root.get_array_mut("list")?, 2));
        Ok(())
    })
    .unwrap();
    let target = target.unwrap();

    doc.update(|root| {
        let list = root.get_array_mut("list")?;
        let element = list.get_element_by_id(&target).unwrap();
        assert_eq!(&JsonValue::Integer(2), element.value());
        Ok(())
    })
    .unwrap();

    doc.update(|root| {
        let list = root.get_array_mut("list")?;
        list.delete_by_id(&target);
        assert!(list.get_element_by_id(&target).is_none());
        Ok(())
    })
    .unwrap();

    assert_eq!(r#"{"list":[0,1]}"#, doc.to_sorted_json());
}

// --- Array Concurrency Table Tests (array_test.ts:770) ---

const ONE_IDX: usize = 1;
const OTHER_IDXS: [usize; 2] = [2, 3];
const NEW_VALUES: [i32; 2] = [5, 6];

/// The operations of the JS table. Each references `ONE_IDX` through the
/// parameter its name gives.
#[derive(Debug, Clone, Copy)]
enum TableOp {
    InsertPrev,
    InsertPrevNext,
    MovePrev,
    MovePrevNext,
    MoveTarget,
    SetTarget,
    RemoveTarget,
}

const TABLE_OPS: [TableOp; 7] = [
    TableOp::InsertPrev,
    TableOp::InsertPrevNext,
    TableOp::MovePrev,
    TableOp::MovePrevNext,
    TableOp::MoveTarget,
    TableOp::SetTarget,
    TableOp::RemoveTarget,
];

impl TableOp {
    /// JS `opName`.
    fn name(self) -> &'static str {
        match self {
            Self::InsertPrev => "insert.prev",
            Self::InsertPrevNext => "insert.prev.next",
            Self::MovePrev => "move.prev",
            Self::MovePrevNext => "move.prev.next",
            Self::MoveTarget => "move.target",
            Self::SetTarget => "set.target",
            Self::RemoveTarget => "remove.target",
        }
    }

    /// JS `executor`, for client `cid`.
    fn execute(self, arr: &mut JsonArray, cid: usize) -> yorkie::Result<()> {
        match self {
            Self::InsertPrev => arr
                .insert_integer_after(ONE_IDX, NEW_VALUES[cid])
                .map(|_| ()),
            Self::InsertPrevNext => arr
                .insert_integer_after(ONE_IDX - 1, NEW_VALUES[cid])
                .map(|_| ()),
            Self::MovePrev => arr
                .move_after_by_index(ONE_IDX, OTHER_IDXS[cid])
                .map(|_| ()),
            Self::MovePrevNext => arr
                .move_after_by_index(ONE_IDX - 1, OTHER_IDXS[cid])
                .map(|_| ()),
            Self::MoveTarget => arr
                .move_after_by_index(OTHER_IDXS[cid], ONE_IDX)
                .map(|_| ()),
            Self::SetTarget => arr.set_value(ONE_IDX, NEW_VALUES[cid]).map(|_| ()),
            Self::RemoveTarget => {
                arr.delete(ONE_IDX);
                Ok(())
            }
        }
    }
}

/// Two clients whose documents both hold `a = [1, 2, 3, 4]`.
async fn with_1234(addr: &str, name: &str) -> TwoClients {
    let t = TwoClients::attach(addr, name).await;
    set_array(&t.d1, "a", vec![1, 2, 3, 4], Some(r#"{"a":[1,2,3,4]}"#));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));
    t
}

/// JS `${op1} vs ${op2}` (array_test.ts:842), for all 49 pairs. Every pair
/// runs, and the test fails with the list of pairs that did not converge.
#[tokio::test]
async fn converges_for_every_pair_of_concurrent_array_operations() {
    let Some(addr) = rpc_addr() else { return };
    let mut diverged = Vec::new();
    for op1 in TABLE_OPS {
        for op2 in TABLE_OPS {
            let case = format!("{} vs {}", op1.name(), op2.name());
            let t = with_1234(&addr, &format!("array-table-{op1:?}-{op2:?}")).await;

            edit_array(&t.d1, "a", None, |a| op1.execute(a, 0));
            edit_array(&t.d2, "a", None, |a| op2.execute(a, 1));

            if !sync_clients_then_check_equal(&t).await {
                diverged.push(format!(
                    "{case}: d1 {} != d2 {}",
                    sorted_json(&t.d1),
                    sorted_json(&t.d2)
                ));
            }
            t.deactivate().await;
        }
    }
    assert!(diverged.is_empty(), "diverged: {diverged:#?}");
}

// --- Can handle complicated concurrent array operations (array_test.ts:880) ---

/// The operations client 1 runs against client 2's two moves.
#[derive(Debug, Clone, Copy)]
enum ComplicatedOp {
    Insert,
    Move,
    Set,
    Remove,
}

impl ComplicatedOp {
    /// JS `executor`.
    fn execute(self, arr: &mut JsonArray) -> yorkie::Result<()> {
        const OTHER_IDX: usize = 0;
        const NEW_VALUE: i32 = 5;
        match self {
            Self::Insert => {
                let prev = id_at(arr, ONE_IDX);
                arr.insert_after(&prev, NEW_VALUE).map(|_| ())
            }
            Self::Move => move_after_by_ids(arr, OTHER_IDX, ONE_IDX),
            // A set implemented as a delete and an insert.
            Self::Set => {
                let target = id_at(arr, ONE_IDX);
                arr.delete_by_id(&target);
                let prev = id_at(arr, ONE_IDX - 1);
                arr.insert_after(&prev, NEW_VALUE).map(|_| ())
            }
            Self::Remove => {
                let target = id_at(arr, ONE_IDX);
                arr.delete_by_id(&target);
                Ok(())
            }
        }
    }
}

/// JS `insert`, `move`, `set` and `remove` (array_test.ts:944): client 1
/// runs one operation at `ONE_IDX` while client 2 moves index 2 after
/// `ONE_IDX` and index 3 after index 2. Every case runs, and the test fails
/// with the list of cases that did not converge.
#[tokio::test]
async fn converges_for_complicated_concurrent_array_operations() {
    let Some(addr) = rpc_addr() else { return };
    let mut diverged = Vec::new();
    for op in [
        ComplicatedOp::Insert,
        ComplicatedOp::Move,
        ComplicatedOp::Set,
        ComplicatedOp::Remove,
    ] {
        let t = with_1234(&addr, &format!("array-complicated-{op:?}")).await;

        edit_array(&t.d1, "a", None, |a| op.execute(a));
        edit_array(&t.d2, "a", None, |a| {
            move_after_by_ids(a, ONE_IDX, 2)?;
            move_after_by_ids(a, 2, 3)
        });

        if !sync_clients_then_check_equal(&t).await {
            diverged.push(format!(
                "{op:?}: d1 {} != d2 {}",
                sorted_json(&t.d1),
                sorted_json(&t.d2)
            ));
        }
        t.deactivate().await;
    }
    assert!(diverged.is_empty(), "diverged: {diverged:#?}");
}

// --- Array Set By Index Tests (array_test.ts:989) ---

/// JS "Can handle simple array set operations" (array_test.ts:990).
#[tokio::test]
async fn converges_after_concurrent_sets_by_index() {
    let Some(addr) = rpc_addr() else { return };
    let t = TwoClients::attach(&addr, "array-set-by-index").await;

    set_array(&t.d1, "k1", vec![-1, -2, -3], Some(r#"{"k1":[-1,-2,-3]}"#));
    t.c1.sync(None).await.unwrap();
    t.c2.sync(None).await.unwrap();
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2));

    edit_array(&t.d2, "k1", Some(r#"{"k1":[-1,-4,-3]}"#), |k1| {
        k1.set_value(1, -4).map(|_| ())
    });
    edit_array(&t.d1, "k1", Some(r#"{"k1":[-5,-2,-3]}"#), |k1| {
        k1.set_value(0, -5).map(|_| ())
    });

    assert!(sync_clients_then_check_equal(&t).await);

    t.deactivate().await;
}

/// JS "can handle array set operation by Proxy" (array_test.ts:1021). JS
/// assigns through the proxy (`root.list[idx] = v`), which is `set_value`
/// here, and finds the index with `findIndex`, which is `index_of`.
#[test]
fn sets_array_elements_by_index() {
    fn update(doc: &mut Document, edit: impl FnOnce(&mut JsonArray) -> yorkie::Result<()>) {
        doc.update(|root| edit(root.get_array_mut("list")?))
            .unwrap();
    }
    let mut doc = Document::new("test-doc");
    let set_found = |list: &mut JsonArray, find: &str, value: JsonValue| {
        if let Some(idx) = list.index_of(find, None) {
            list.set_value(idx, value)?;
        }
        Ok(())
    };

    doc.update(|root| root.set("list", vec!["a", "b", "c"]).map(|_| ()))
        .unwrap();
    assert_eq!(r#"{"list":["a","b","c"]}"#, doc.to_sorted_json());

    // insertAfter #1 and #2
    for expected in [
        r#"{"list":["a","newV","b","c"]}"#,
        r#"{"list":["a","newV","newV","b","c"]}"#,
    ] {
        update(&mut doc, |list| {
            let prev = id_at(list, 0);
            list.insert_after(&prev, "newV").map(|_| ())
        });
        assert_eq!(expected, doc.to_sorted_json());
    }

    // set #1
    update(&mut doc, |list| list.set_value(0, "setV").map(|_| ()));
    assert_eq!(
        r#"{"list":["setV","newV","newV","b","c"]}"#,
        doc.to_sorted_json()
    );

    // set #2
    update(&mut doc, |list| set_found(list, "setV", "setV2".into()));
    assert_eq!(
        r#"{"list":["setV2","newV","newV","b","c"]}"#,
        doc.to_sorted_json()
    );

    // set #2, to an array
    update(&mut doc, |list| {
        set_found(list, "setV2", vec!["s", "e", "t", "V", "3"].into())
    });
    assert_eq!(
        r#"{"list":[["s","e","t","V","3"],"newV","newV","b","c"]}"#,
        doc.to_sorted_json()
    );
}
