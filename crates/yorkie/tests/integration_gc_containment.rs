//! Integration test that garbage collection never leaves a document unable
//! to sync.
//!
//! JS reference: `packages/sdk/test/integration/gc_containment_test.ts`. Run
//! with `YORKIE_TEST_RPC_ADDR=http://localhost:18080 cargo test -p yorkie`.

mod integration_support;

use integration_support::two_clients::{active_client, manual_attach, shared};
use integration_support::{json_value, rpc_addr, unique_key};
use yorkie::{Client, DeactivateOptions, JsonArray, JsonObject, JsonValue, Result, SharedDocument};

/// Runs an undo or redo that may have nothing left to apply (JS
/// `undoTolerantly`). A reverse operation names the element it was recorded
/// against, and a peer may have removed it in the meantime; that is reported
/// as an error and leaves the document untouched.
fn undo_tolerantly(doc: &SharedDocument, undo: bool) {
    let mut doc = doc.lock().unwrap();
    let _ = if undo {
        doc.history_mut().undo()
    } else {
        doc.history_mut().redo()
    };
}

fn items(root: &mut JsonObject) -> Result<&mut JsonArray> {
    root.get_array_mut("items")
}

/// The index of the item whose `id` is `id`, as JS `findIndex`.
fn index_of(items: &JsonArray, id: &str) -> Option<usize> {
    items.iter().position(|item| match item {
        JsonValue::Object(object) => object.get("id") == Some(&JsonValue::String(id.to_owned())),
        _ => false,
    })
}

/// A plain copy of the item at `index`, as JS
/// `JSON.parse(JSON.stringify(item))`.
fn plain_copy(items: &JsonArray, index: usize) -> JsonValue {
    let json = items.get(index).unwrap().to_sorted_json();
    json_value(serde_json::from_str(&json).unwrap())
}

fn update(doc: &SharedDocument, f: impl FnOnce(&mut JsonObject) -> Result<()>) {
    doc.lock().unwrap().update(f).unwrap();
}

/// Moves the item `x` to the front: removes it, then inserts a plain copy.
fn reorder(root: &mut JsonObject) -> Result<()> {
    let items = items(root)?;
    let Some(i) = index_of(items, "x") else {
        return Ok(());
    };
    let plain = plain_copy(items, i);
    items.splice(i as isize, Some(1), Vec::<JsonValue>::new())?;
    items.splice(0, Some(0), [plain])?;
    Ok(())
}

async fn sync(c1: &Client, d1: &SharedDocument, c2: &Client, d2: &SharedDocument) {
    for _ in 0..4 {
        c1.sync(Some(d1)).await.unwrap();
        c2.sync(Some(d2)).await.unwrap();
    }
}

/// JS "keeps syncing while two peers reorder, edit and undo one element"
/// (gc_containment_test.ts:46).
#[tokio::test]
async fn keeps_syncing_while_two_peers_reorder_edit_and_undo_one_element() {
    let Some(addr) = rpc_addr() else { return };
    let key = unique_key("gc-containment");
    let c1 = active_client(&addr).await;
    let c2 = active_client(&addr).await;
    let d1 = c1.attach(shared(&key), manual_attach()).await.unwrap();
    let d2 = c2.attach(shared(&key), manual_attach()).await.unwrap();

    update(&d1, |root| {
        root.set(
            "items",
            json_value(serde_json::json!([
                {"id": "x", "box": {"w": 10, "h": 10}, "body": {"text": ""}},
                {"id": "y", "box": {"w": 10, "h": 10}, "body": {"text": ""}},
            ])),
        )
        .map(|_| ())
    });
    sync(&c1, &d1, &c2, &d2).await;

    // One element removed and re-inserted over and over, both peers
    // rewriting its nested objects and walking their undo stacks.
    for round in 0..8 {
        update(&d1, reorder);
        update(&d2, |root| {
            let items = items(root)?;
            let Some(i) = index_of(items, "x") else {
                return Ok(());
            };
            let item = items.get_object_mut(i)?;
            item.set(
                "body",
                json_value(serde_json::json!({"text": "o".repeat(round + 1)})),
            )?;
            item.set(
                "box",
                json_value(serde_json::json!({"w": 10, "h": 20 + round})),
            )?;
            Ok(())
        });

        sync(&c1, &d1, &c2, &d2).await;

        if round % 3 == 0 {
            undo_tolerantly(&d1, true);
        }
        if round % 4 == 0 {
            update(&d2, |root| {
                let items = items(root)?;
                if let Some(i) = index_of(items, "y") {
                    items.splice(i as isize, Some(1), Vec::<JsonValue>::new())?;
                }
                Ok(())
            });
            undo_tolerantly(&d2, true);
        }
        if round % 5 == 0 {
            undo_tolerantly(&d1, false);
            undo_tolerantly(&d2, false);
        }

        // No sync in between, so both peers stack a reverse operation on top
        // of state the other has not seen.
        update(&d1, |root| {
            let items = items(root)?;
            if let Some(i) = index_of(items, "x") {
                items.get_object_mut(i)?.set(
                    "body",
                    json_value(serde_json::json!({"text": format!("d1-{round}")})),
                )?;
            }
            Ok(())
        });
        update(&d2, reorder);
        undo_tolerantly(&d1, true);
        undo_tolerantly(&d2, true);

        sync(&c1, &d1, &c2, &d2).await;
    }

    // A poisoned client stops applying change packs, so an ordinary edit
    // followed by a sync is what tells a healthy one from a dead one.
    update(&d1, |root| {
        let items = items(root)?;
        if let Some(i) = index_of(items, "x") {
            items
                .get_object_mut(i)?
                .set("box", json_value(serde_json::json!({"w": 1, "h": 1})))?;
        }
        Ok(())
    });
    sync(&c1, &d1, &c2, &d2).await;

    assert_eq!(
        d1.lock().unwrap().to_sorted_json(),
        d2.lock().unwrap().to_sorted_json(),
        "the two documents disagree after the final edit"
    );

    for client in [c1, c2] {
        client
            .deactivate(DeactivateOptions::default())
            .await
            .unwrap();
    }
}
