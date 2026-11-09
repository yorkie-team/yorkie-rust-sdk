//! The depth of the undo and redo stacks.
//!
//! JS reference: `test/unit/document/undo_depth_test.ts`, `describe('History
//! depth')` (117-160). JS `new History()` is `History::default()` here.

use super::{History, HistoryOperation, MAX_UNDO_REDO_STACK_DEPTH};
use crate::presence::ReversePresence;
use serde_json::json;

/// JS `entry` (103-106): an entry that carries `n` so the eviction order is
/// observable.
fn entry(n: usize) -> Vec<HistoryOperation> {
    let mut values = ReversePresence::new();
    values.insert("n".to_owned(), Some(json!(n)));
    vec![HistoryOperation::Presence(values)]
}

/// JS `marks` (109-111): reads back the `n` of each entry on a stack.
fn marks(stack: &[Vec<HistoryOperation>]) -> Vec<usize> {
    stack
        .iter()
        .map(|ops| match &ops[0] {
            HistoryOperation::Presence(values) => values["n"]
                .as_ref()
                .and_then(|n| n.as_u64())
                .map(|n| n as usize)
                .expect("a marked entry"),
            other => panic!("expected a presence entry, got {other:?}"),
        })
        .collect()
}

/// JS "falls back to the default depth when constructed without one".
#[test]
fn falls_back_to_the_default_depth_when_constructed_without_one() {
    assert_eq!(MAX_UNDO_REDO_STACK_DEPTH, History::default().max_depth());
}

/// JS "reports the depth it was constructed with".
#[test]
fn reports_the_depth_it_was_constructed_with() {
    assert_eq!(7, History::new(7).max_depth());
}

/// JS "drops the oldest redo entry once the depth is exceeded".
#[test]
fn drops_the_oldest_redo_entry_once_the_depth_is_exceeded() {
    let mut history = History::new(3);
    for n in 0..5 {
        history.push_redo(entry(n));
    }

    assert_eq!(vec![2, 3, 4], marks(history.redo_stack()));
}

/// JS "drops the oldest undo entry once the depth is exceeded".
#[test]
fn drops_the_oldest_undo_entry_once_the_depth_is_exceeded() {
    let mut history = History::new(3);
    for n in 0..5 {
        history.push_undo(entry(n));
    }

    assert_eq!(vec![2, 3, 4], marks(history.undo_stack()));
}

/// JS "bounds the redo stack independently of the default depth".
#[test]
fn bounds_the_redo_stack_independently_of_the_default_depth() {
    let mut history = History::new(MAX_UNDO_REDO_STACK_DEPTH + 5);
    for n in 0..MAX_UNDO_REDO_STACK_DEPTH + 5 {
        history.push_redo(entry(n));
    }

    assert_eq!(MAX_UNDO_REDO_STACK_DEPTH + 5, history.redo_stack().len());
    assert_eq!(0, marks(history.redo_stack())[0]);
}
