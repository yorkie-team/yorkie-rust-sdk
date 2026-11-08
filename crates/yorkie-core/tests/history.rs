//! Undo and redo of local changes through `Document::history_mut`.
//!
//! JS references, all on a single `Document` with no server:
//! - `test/integration/object_test.ts` (`Undo/Redo`: simple and nested
//!   object).
//! - `test/integration/counter_test.ts` (increase, long type and overflow).
//! - `test/integration/history_array_test.ts` (`Array Undo Operations`).
//! - `test/integration/history_text_test.ts` (`Text History - single client
//!   ...`).
//! - `test/integration/doc_presence_test.ts` (`Undo/Redo` with presence; the
//!   client attach is replaced by `apply_status(DocStatus::Attached)`).
//! - `test/unit/document/undo_depth_test.ts` (default depth).
//! - `src/document/document.ts` `update`/`executeUndoRedo` for the redo-stack
//!   clearing and change queueing rules.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use yorkie_core::{
    DocEvent, DocStatus, Document, JsonArray, JsonObject, JsonText, JsonValue, Presence, Result,
};

/// JS `assertUndoRedo`: undoes back to the first state, redoes to the last,
/// and undoes back again, checking every state on the way.
fn assert_undo_redo(doc: &mut Document, states: &[String]) -> Result<()> {
    for state in states.iter().rev().skip(1) {
        doc.history_mut().undo()?;
        assert_eq!(*state, doc.to_sorted_json(), "undo");
    }
    for state in states.iter().skip(1) {
        doc.history_mut().redo()?;
        assert_eq!(*state, doc.to_sorted_json(), "redo");
    }
    for state in states.iter().rev().skip(1) {
        doc.history_mut().undo()?;
        assert_eq!(*state, doc.to_sorted_json(), "undo again");
    }
    Ok(())
}

// --- Object (object_test.ts) ---

#[test]
fn undoes_and_redoes_a_simple_object() -> Result<()> {
    let mut doc = Document::new("test-doc");
    assert_eq!("{}", doc.to_sorted_json());
    assert!(!doc.history().can_undo());
    assert!(!doc.history().can_redo());

    doc.update(|root| root.set("a", 1).map(|_| ()))?;
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    doc.update(|root| root.set("a", 2).map(|_| ()))?;
    assert_eq!(r#"{"a":2}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    doc.history_mut().undo()?;
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(doc.history().can_redo());

    doc.history_mut().redo()?;
    assert_eq!(r#"{"a":2}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(!doc.history().can_redo());

    doc.history_mut().undo()?;
    assert_eq!(r#"{"a":1}"#, doc.to_sorted_json());
    assert!(doc.history().can_undo());
    assert!(doc.history().can_redo());

    doc.history_mut().undo()?;
    assert_eq!("{}", doc.to_sorted_json());
    assert!(!doc.history().can_undo());
    assert!(doc.history().can_redo());
    Ok(())
}

#[test]
fn undoes_and_redoes_a_nested_object() -> Result<()> {
    let mut doc = Document::new("test-doc");
    let mut states = vec![doc.to_sorted_json()];
    assert_eq!("{}", doc.to_sorted_json());

    doc.update(|root| {
        root.set(
            "shape",
            object(json!({"point": {"x": 0, "y": 0}, "color": "red"}))?,
        )?;
        root.set("a", 0)?;
        Ok(())
    })?;
    states.push(doc.to_sorted_json());
    assert_eq!(
        r#"{"a":0,"shape":{"color":"red","point":{"x":0,"y":0}}}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| {
        let shape = root.get_object_mut("shape")?;
        shape.set("point", object(json!({"x": 1, "y": 1}))?)?;
        shape.set("color", "blue")?;
        Ok(())
    })?;
    states.push(doc.to_sorted_json());
    assert_eq!(
        r#"{"a":0,"shape":{"color":"blue","point":{"x":1,"y":1}}}"#,
        doc.to_sorted_json()
    );

    doc.update(|root| {
        root.set("a", 1)?;
        root.set("a", 2)?;
        Ok(())
    })?;
    states.push(doc.to_sorted_json());
    assert_eq!(
        r#"{"a":2,"shape":{"color":"blue","point":{"x":1,"y":1}}}"#,
        doc.to_sorted_json()
    );

    assert_undo_redo(&mut doc, &states)
}

#[test]
fn undoes_an_object_member_removal() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        root.set("shape", object(json!({"color": "black", "size": 3}))?)?;
        Ok(())
    })?;
    let states = vec![
        doc.to_sorted_json(),
        {
            doc.update(|root| {
                root.get_object_mut("shape")?.remove("color");
                Ok(())
            })?;
            doc.to_sorted_json()
        },
        {
            doc.update(|root| {
                root.remove("shape");
                Ok(())
            })?;
            doc.to_sorted_json()
        },
    ];
    assert_eq!(r#"{"shape":{"size":3}}"#, states[1]);
    assert_eq!("{}", states[2]);

    assert_undo_redo(&mut doc, &states)
}

// --- Counter (counter_test.ts) ---

#[test]
fn undoes_and_redoes_an_increase() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| root.set_counter("counter", 100).map(|_| ()))?;
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());

    doc.update(|root| root.get_counter_mut("counter")?.increase(1).map(|_| ()))?;
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());

    doc.history_mut().undo()?;
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());

    doc.history_mut().redo()?;
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());

    doc.history_mut().undo()?;
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());
    Ok(())
}

#[test]
fn undoes_and_redoes_long_counters_and_overflow() -> Result<()> {
    let mut doc = Document::new("test-doc");
    let mut states = Vec::new();

    doc.update(|root| {
        root.set_counter("cnt", 0)?;
        root.set_long_counter("longCnt", 0)?;
        Ok(())
    })?;
    assert_eq!(r#"{"cnt":0,"longCnt":0}"#, doc.to_sorted_json());
    states.push(doc.to_sorted_json());

    doc.update(|root| {
        root.get_counter_mut("cnt")?.increase(2147483647)?;
        root.get_counter_mut("longCnt")?
            .increase(9223372036854775807i64)?;
        Ok(())
    })?;
    assert_eq!(
        r#"{"cnt":2147483647,"longCnt":9223372036854775807}"#,
        doc.to_sorted_json()
    );
    states.push(doc.to_sorted_json());

    doc.update(|root| {
        root.get_counter_mut("cnt")?.increase(1)?;
        root.get_counter_mut("longCnt")?.increase(1i64)?;
        Ok(())
    })?;
    assert_eq!(
        r#"{"cnt":-2147483648,"longCnt":-9223372036854775808}"#,
        doc.to_sorted_json()
    );
    states.push(doc.to_sorted_json());

    assert_undo_redo(&mut doc, &states)
}

// --- Array (history_array_test.ts) ---

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArrayOp {
    Add,
    Remove,
    Move,
    Set,
}

const ARRAY_OPS: [ArrayOp; 4] = [ArrayOp::Add, ArrayOp::Remove, ArrayOp::Move, ArrayOp::Set];

fn letters_doc() -> Result<Document> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        let mut list = JsonArray::new();
        for letter in ["a", "b", "c", "d", "e"] {
            list.push(letter)?;
        }
        root.set("list", list).map(|_| ())
    })?;
    Ok(doc)
}

/// JS `applyOp1`.
fn apply_array_op(doc: &mut Document, op: ArrayOp) -> Result<()> {
    doc.update(|root| {
        let list = root.get_array_mut("list")?;
        match op {
            ArrayOp::Add => {
                if let Some(prev) = list.element_id(0).cloned() {
                    list.insert_after(&prev, "insV")?;
                }
            }
            ArrayOp::Move => {
                if list.len() >= 3 {
                    let from = list.element_id(0).cloned().unwrap();
                    let to = list.element_id(2).cloned().unwrap();
                    list.move_after(&to, &from)?;
                }
            }
            ArrayOp::Remove => {
                if !list.is_empty() {
                    list.delete(0);
                }
            }
            ArrayOp::Set => {
                if list.len() > 1 {
                    list.set_value(1, "s")?;
                }
            }
        }
        Ok(())
    })
}

#[test]
fn undoes_each_array_operation() -> Result<()> {
    for op in ARRAY_OPS {
        let mut doc = letters_doc()?;
        let initial = doc.to_sorted_json();

        apply_array_op(&mut doc, op)?;
        doc.history_mut().undo()?;

        assert_eq!(initial, doc.to_sorted_json(), "undo of {op:?}");
    }
    Ok(())
}

#[test]
fn returns_to_each_array_state() -> Result<()> {
    for op1 in ARRAY_OPS {
        for op2 in ARRAY_OPS {
            for op3 in ARRAY_OPS {
                // Skipped upstream: a set after a move restores the value at
                // the moved element's original position (see
                // docs/crdt-parity/upstream-skipped-tests.md).
                if op1 == ArrayOp::Move && op3 == ArrayOp::Set {
                    continue;
                }

                let mut doc = letters_doc()?;
                let mut states = vec![doc.to_sorted_json()];
                for op in [op1, op2, op3] {
                    apply_array_op(&mut doc, op)?;
                    states.push(doc.to_sorted_json());
                }

                for i in (1..=3).rev() {
                    doc.history_mut().undo()?;
                    assert_eq!(
                        states[i - 1],
                        doc.to_sorted_json(),
                        "undo back to S{} on {op1:?}-{op2:?}-{op3:?}",
                        i - 1
                    );
                }
            }
        }
    }
    Ok(())
}

// --- Text (history_text_test.ts, single client) ---

#[derive(Debug, Clone, Copy)]
enum TextOp {
    Insert,
    Delete,
    Replace,
    Style,
}

const TEXT_OPS: [TextOp; 3] = [TextOp::Insert, TextOp::Delete, TextOp::Replace];

fn text_doc(content: &str) -> Result<Document> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        let text = root.set_text("t")?;
        if !content.is_empty() {
            text.edit(0, 0, content, None)?;
        }
        Ok(())
    })?;
    Ok(doc)
}

fn content_of(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected text at t, got {other:?}"),
    }
}

fn edit_text(doc: &mut Document, edit: impl FnOnce(&mut JsonText) -> Result<()>) -> Result<()> {
    doc.update(|root| edit(root.get_text_mut("t")?))
}

/// JS `applyTextOp1`.
fn apply_text_op(doc: &mut Document, op: TextOp) -> Result<()> {
    edit_text(doc, |t| {
        let len = t.len();
        match op {
            TextOp::Insert => {
                t.edit(len, len, "X", None)?;
            }
            TextOp::Delete => {
                if len >= 3 {
                    t.edit(1, 2, "", None)?;
                } else if len > 0 {
                    t.edit(0, 1, "", None)?;
                }
            }
            TextOp::Replace => {
                if len >= 3 {
                    t.edit(1, 3, "12", None)?;
                } else {
                    t.edit(0, len.min(1), "R", None)?;
                }
            }
            TextOp::Style => {
                if len == 0 {
                    t.edit(0, 0, "A", None)?;
                }
                let len = t.len();
                t.set_style(0, len, attrs(json!({"bold": true})))?;
            }
        }
        Ok(())
    })
}

#[test]
fn undoes_and_redoes_each_text_operation() -> Result<()> {
    for op in TEXT_OPS {
        let mut doc = text_doc("The fox jumped.")?;
        let before = content_of(&doc);
        apply_text_op(&mut doc, op)?;
        let after = content_of(&doc);

        doc.history_mut().undo()?;
        assert_eq!(before, content_of(&doc), "undo {op:?}");

        doc.history_mut().redo()?;
        assert_eq!(after, content_of(&doc), "redo {op:?}");
    }
    Ok(())
}

#[test]
fn round_trips_text_undo_and_redo_several_times() -> Result<()> {
    let mut doc = text_doc("ABCD")?;
    let initial = content_of(&doc);
    edit_text(&mut doc, |t| t.edit(2, 2, "XY", None).map(|_| ()))?;
    let modified = content_of(&doc);

    for round in 0..3 {
        doc.history_mut().undo()?;
        assert_eq!(initial, content_of(&doc), "round {round} undo");
        doc.history_mut().redo()?;
        assert_eq!(modified, content_of(&doc), "round {round} redo");
    }
    Ok(())
}

#[test]
fn clears_redo_when_a_new_edit_follows_an_undo() -> Result<()> {
    let mut doc = text_doc("ABCD")?;
    edit_text(&mut doc, |t| t.edit(4, 4, "EF", None).map(|_| ()))?;

    doc.history_mut().undo()?;
    assert!(doc.history().can_redo());

    edit_text(&mut doc, |t| t.edit(0, 0, "Z", None).map(|_| ()))?;
    assert!(!doc.history().can_redo());
    Ok(())
}

#[test]
fn undoes_and_redoes_a_text_style() -> Result<()> {
    let mut doc = text_doc("The fox jumped.")?;
    let initial = doc.to_sorted_json();
    let styled = r#"{"t":[{"attrs":{"bold":true},"val":"The fox jumped."}]}"#;

    apply_text_op(&mut doc, TextOp::Style)?;
    assert_eq!(styled, doc.to_sorted_json());

    doc.history_mut().undo()?;
    assert_eq!(initial, doc.to_sorted_json());

    doc.history_mut().redo()?;
    assert_eq!(styled, doc.to_sorted_json());
    Ok(())
}

#[test]
fn undoes_and_redoes_chained_text_operations() -> Result<()> {
    for op1 in TEXT_OPS {
        for op2 in TEXT_OPS {
            for op3 in TEXT_OPS {
                let case = format!("{op1:?}-{op2:?}-{op3:?}");
                let mut doc = text_doc("ABCD")?;
                let mut snapshots = vec![content_of(&doc)];
                for op in [op1, op2, op3] {
                    apply_text_op(&mut doc, op)?;
                    snapshots.push(content_of(&doc));
                }

                for i in (1..=3).rev() {
                    doc.history_mut().undo()?;
                    assert_eq!(
                        snapshots[i - 1],
                        content_of(&doc),
                        "{case}: undo to S{}",
                        i - 1
                    );
                }
                for i in 0..3 {
                    doc.history_mut().redo()?;
                    assert_eq!(
                        snapshots[i + 1],
                        content_of(&doc),
                        "{case}: redo to S{}",
                        i + 1
                    );
                }
            }
        }
    }
    Ok(())
}

/// The single-client edge cases of `history_text_test.ts`: an edit, its
/// expected content, then undo and redo.
#[test]
fn undoes_and_redoes_text_edge_cases() -> Result<()> {
    let cases = [
        ("delete at start", "ABCD", 0, 2, "", "CD"),
        ("delete at end", "ABCD", 2, 4, "", "AB"),
        ("insert into empty text", "", 0, 0, "Hello", "Hello"),
        ("full deletion", "ABCD", 0, 4, "", ""),
        ("full replacement", "OLD", 0, 3, "NEW", "NEW"),
    ];
    for (name, initial, from, to, content, edited) in cases {
        let mut doc = text_doc(initial)?;
        edit_text(&mut doc, |t| t.edit(from, to, content, None).map(|_| ()))?;
        assert_eq!(edited, content_of(&doc), "{name}");

        doc.history_mut().undo()?;
        assert_eq!(initial, content_of(&doc), "{name}: undo");

        doc.history_mut().redo()?;
        assert_eq!(edited, content_of(&doc), "{name}: redo");
    }
    Ok(())
}

#[test]
fn undoes_single_character_operations() -> Result<()> {
    let mut doc = text_doc("ABC")?;

    edit_text(&mut doc, |t| t.edit(1, 1, "X", None).map(|_| ()))?;
    assert_eq!("AXBC", content_of(&doc));
    doc.history_mut().undo()?;
    assert_eq!("ABC", content_of(&doc));

    edit_text(&mut doc, |t| t.edit(1, 2, "", None).map(|_| ()))?;
    assert_eq!("AC", content_of(&doc));
    doc.history_mut().undo()?;
    assert_eq!("ABC", content_of(&doc));
    Ok(())
}

#[test]
fn reports_empty_text_history_stacks() -> Result<()> {
    let mut doc = text_doc("ABCD")?;
    assert!(!doc.history().can_redo());

    assert!(doc.history().can_undo());
    doc.history_mut().undo()?;
    assert!(!doc.history().can_undo());
    Ok(())
}

#[test]
fn undoes_and_redoes_rapid_consecutive_edits() -> Result<()> {
    let mut doc = text_doc("")?;
    let mut states = vec![String::new()];
    for i in 0..10 {
        edit_text(&mut doc, |t| {
            let len = t.len();
            t.edit(len, len, &i.to_string(), None).map(|_| ())
        })?;
        states.push(content_of(&doc));
    }

    for i in (0..10).rev() {
        doc.history_mut().undo()?;
        assert_eq!(states[i], content_of(&doc));
    }
    for state in &states[1..] {
        doc.history_mut().redo()?;
        assert_eq!(*state, content_of(&doc));
    }
    Ok(())
}

// --- Presence (doc_presence_test.ts) ---

fn attached_doc_with(initial: Value) -> Result<Document> {
    let mut doc = Document::new("test-doc");
    doc.set_actor("000000000000000000000001");
    doc.apply_status(DocStatus::Attached);
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(initial));
        Ok(())
    })?;
    Ok(doc)
}

#[test]
fn undoes_and_redoes_presence() -> Result<()> {
    let mut doc = attached_doc_with(json!({"color": "red"}))?;
    doc.update(|root| root.set_counter("counter", 100).map(|_| ()))?;

    // 1. Presence update only.
    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(json!({"color": "blue"})));
        Ok(())
    })?;
    assert_eq!(presence_of(json!({"color": "blue"})), doc.my_presence());

    doc.history_mut().undo()?;
    assert_eq!(presence_of(json!({"color": "red"})), doc.my_presence());

    doc.history_mut().redo()?;
    assert_eq!(presence_of(json!({"color": "blue"})), doc.my_presence());

    // 2. Presence update with root update.
    doc.update_with_presence(|root, presence| {
        root.get_counter_mut("counter")?.increase(1)?;
        presence.set_with_history(presence_of(json!({"color": "green"})));
        Ok(())
    })?;
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());
    assert_eq!(presence_of(json!({"color": "green"})), doc.my_presence());

    doc.history_mut().undo()?;
    assert_eq!(r#"{"counter":100}"#, doc.to_sorted_json());
    assert_eq!(presence_of(json!({"color": "blue"})), doc.my_presence());

    doc.history_mut().redo()?;
    assert_eq!(r#"{"counter":101}"#, doc.to_sorted_json());
    assert_eq!(presence_of(json!({"color": "green"})), doc.my_presence());
    Ok(())
}

#[test]
fn leaves_presence_keys_out_of_history_unless_asked() -> Result<()> {
    let mut doc = attached_doc_with(json!({"color": "red", "cursor": {"x": 0, "y": 0}}))?;

    // 1. Both keys go to the history.
    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(
            json!({"color": "blue", "cursor": {"x": 1, "y": 1}}),
        ));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(
        presence_of(json!({"color": "red", "cursor": {"x": 0, "y": 0}})),
        doc.my_presence()
    );
    doc.history_mut().redo()?;
    assert_eq!(
        presence_of(json!({"color": "blue", "cursor": {"x": 1, "y": 1}})),
        doc.my_presence()
    );

    // 2. Only the cursor goes to the history.
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(json!({"color": "green"})));
        presence.set_with_history(presence_of(json!({"cursor": {"x": 2, "y": 2}})));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(
        presence_of(json!({"color": "green", "cursor": {"x": 1, "y": 1}})),
        doc.my_presence()
    );
    doc.history_mut().redo()?;
    assert_eq!(
        presence_of(json!({"color": "green", "cursor": {"x": 2, "y": 2}})),
        doc.my_presence()
    );

    // 3. Nothing goes to the history: the undo reverts the cursor of step 2.
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(json!({"color": "black"})));
        presence.set(presence_of(json!({"cursor": {"x": 3, "y": 3}})));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(
        presence_of(json!({"color": "black", "cursor": {"x": 1, "y": 1}})),
        doc.my_presence()
    );
    doc.history_mut().redo()?;
    assert_eq!(
        presence_of(json!({"color": "black", "cursor": {"x": 3, "y": 3}})),
        doc.my_presence()
    );
    Ok(())
}

#[test]
fn restores_the_presence_before_the_update_for_a_key_set_twice() -> Result<()> {
    let mut doc = attached_doc_with(json!({"color": "red"}))?;

    // 1. Two changes to one key revert to the value before the update.
    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(json!({"color": "blue"})));
        presence.set_with_history(presence_of(json!({"color": "green"})));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(presence_of(json!({"color": "red"})), doc.my_presence());
    doc.history_mut().redo()?;
    assert_eq!(presence_of(json!({"color": "green"})), doc.my_presence());

    // 2. The last `set` of a key decides: here it goes to the history.
    doc.update_with_presence(|_, presence| {
        presence.set(presence_of(json!({"color": "black"})));
        presence.set_with_history(presence_of(json!({"color": "purple"})));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(presence_of(json!({"color": "green"})), doc.my_presence());
    doc.history_mut().redo()?;
    assert_eq!(presence_of(json!({"color": "purple"})), doc.my_presence());

    // 3. Here it does not, so the undo reverts step 2 instead.
    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(json!({"color": "yellow"})));
        presence.set(presence_of(json!({"color": "orange"})));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(presence_of(json!({"color": "green"})), doc.my_presence());
    doc.history_mut().redo()?;
    assert_eq!(presence_of(json!({"color": "orange"})), doc.my_presence());
    Ok(())
}

#[test]
fn removes_a_presence_key_the_undone_update_added() -> Result<()> {
    let mut doc = attached_doc_with(json!({"color": "red"}))?;

    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(json!({"cursor": 1})));
        Ok(())
    })?;
    doc.history_mut().undo()?;
    assert_eq!(presence_of(json!({"color": "red"})), doc.my_presence());

    doc.history_mut().redo()?;
    assert_eq!(
        presence_of(json!({"color": "red", "cursor": 1})),
        doc.my_presence()
    );
    Ok(())
}

// --- Stack rules (document.ts, history.ts, undo_depth_test.ts) ---

#[test]
fn does_nothing_when_there_is_nothing_to_undo_or_redo() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.history_mut().undo()?;
    doc.history_mut().redo()?;
    assert_eq!("{}", doc.to_sorted_json());
    assert!(!doc.has_local_changes());
    Ok(())
}

#[test]
fn keeps_fifty_undo_entries() -> Result<()> {
    let mut doc = text_doc("")?;
    for i in 0..59 {
        edit_text(&mut doc, |t| t.edit(i, i, "a", None).map(|_| ()))?;
    }

    let mut steps = 0;
    while doc.history().can_undo() {
        doc.history_mut().undo()?;
        steps += 1;
    }
    assert_eq!(50, steps);
    assert_eq!("a".repeat(9), content_of(&doc));
    Ok(())
}

#[test]
fn queues_undo_and_redo_as_local_changes() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| root.set("a", 1).map(|_| ()))?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    doc.subscribe(move |event| {
        if let DocEvent::LocalChange(info) = event {
            sink.lock().unwrap().push(info.operations.len());
        }
    });

    doc.history_mut().undo()?;
    doc.history_mut().redo()?;

    assert!(doc.has_local_changes());
    assert_eq!(vec![1, 1], *events.lock().unwrap());
    Ok(())
}

#[test]
fn an_undone_presence_change_does_not_clear_redo() -> Result<()> {
    let mut doc = attached_doc_with(json!({"color": "red"}))?;
    doc.update(|root| root.set("a", 1).map(|_| ()))?;
    doc.history_mut().undo()?;
    assert!(doc.history().can_redo());

    doc.update_with_presence(|_, presence| {
        presence.set_with_history(presence_of(json!({"color": "blue"})));
        Ok(())
    })?;
    assert!(doc.history().can_redo());

    doc.update(|root| root.set("b", 1).map(|_| ()))?;
    assert!(!doc.history().can_redo());
    Ok(())
}

fn object(value: Value) -> Result<JsonObject> {
    let mut object = JsonObject::new();
    for (key, value) in value.as_object().unwrap() {
        match value {
            Value::Object(_) => object.set(key.clone(), self::object(value.clone())?)?,
            Value::Number(n) => object.set(key.clone(), n.as_i64().unwrap() as i32)?,
            Value::String(s) => object.set(key.clone(), s.as_str())?,
            other => panic!("unsupported test value {other}"),
        };
    }
    Ok(object)
}

fn presence_of(value: Value) -> Presence {
    value.as_object().unwrap().clone()
}

fn attrs(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}
