//! Undo and redo helpers shared by the Tree history integration tests.

use super::two_clients::TwoClients;
use yorkie::SharedDocument;

/// JS `doc.history.undo()`.
pub fn undo(doc: &SharedDocument) {
    doc.lock().unwrap().history_mut().undo().unwrap();
}

/// JS `doc.history.redo()`.
pub fn redo(doc: &SharedDocument) {
    doc.lock().unwrap().history_mut().redo().unwrap();
}

pub fn sorted_json(doc: &SharedDocument) -> String {
    doc.lock().unwrap().to_sorted_json()
}

/// JS `assert.equal(d1.toSortedJSON(), d2.toSortedJSON(), what)`.
pub fn assert_converged(t: &TwoClients, what: &str) {
    assert_eq!(sorted_json(&t.d1), sorted_json(&t.d2), "{what}");
}

impl TwoClients {
    /// JS `settle` of `history_tree_concurrent_test.ts`: three push/pull
    /// rounds on both clients, so each replica's min synced version vector
    /// moves far enough for garbage collection to purge the tombstones.
    pub async fn settle(&self) {
        for _ in 0..3 {
            self.c1.sync(None).await.unwrap();
            self.c2.sync(None).await.unwrap();
        }
    }
}
