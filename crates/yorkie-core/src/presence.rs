//! Presence model, mirroring JS `document/presence/{presence,change}.ts`.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A client's presence: a JSON object of arbitrary keys.
pub type Presence = serde_json::Map<String, Value>;

/// The presence values an undo restores, by key; `None` removes a key the
/// undone change introduced (JS `Partial<P>` with an `undefined` value).
pub(crate) type ReversePresence = BTreeMap<String, Option<Value>>;

/// A presence change carried by a `Change`.
///
/// `Put` carries the whole presence as of the change. `Clear` removes the
/// actor's presence. JS also has `Patch`; it is an upload-side optimisation
/// the server folds into `Put`.
///
/// `Patch` is receive-only: Rust never produces one. It is decoded so a pack
/// carrying one (a server without the presence-patch capability stores a patch
/// as-is) does not wedge sync. On apply, its keys merge onto the actor's
/// existing presence, as in JS `Change.execute`. It encodes back to PATCH.
#[derive(Debug, Clone, PartialEq)]
pub enum PresenceChange {
    Put(Presence),
    Patch(Presence),
    Clear,
}

/// The presence handle passed to `Document::update_with_presence`
/// (JS `Presence` class in `presence/presence.ts`).
#[derive(Debug, Clone)]
pub struct PresenceProxy {
    presence: Presence,
    change: Option<PresenceChange>,
    /// The presence before this update (JS `ChangeContext.previousPresence`).
    previous: Presence,
    /// The keys whose previous values an undo restores (JS
    /// `reversePresenceKeys`).
    reverse_keys: BTreeSet<String>,
}

impl PresenceProxy {
    pub(crate) fn new(presence: Presence) -> Self {
        Self {
            previous: presence.clone(),
            presence,
            change: None,
            reverse_keys: BTreeSet::new(),
        }
    }

    /// Merges the given keys into the presence (JS `presence.set`). An undo
    /// does not restore these keys, and a later `set` of a key in the same
    /// update decides for that key.
    pub fn set(&mut self, presence: Presence) {
        self.set_keys(presence, false);
    }

    /// Merges the given keys into the presence, like `set`, and lets an undo
    /// of this update restore their previous values (JS
    /// `presence.set(presence, { addToHistory: true })`).
    pub fn set_with_history(&mut self, presence: Presence) {
        self.set_keys(presence, true);
    }

    fn set_keys(&mut self, presence: Presence, add_to_history: bool) {
        for (key, value) in presence {
            if add_to_history {
                self.reverse_keys.insert(key.clone());
            } else {
                self.reverse_keys.remove(&key);
            }
            self.presence.insert(key, value);
        }
        self.change = Some(PresenceChange::Put(self.presence.clone()));
    }

    /// Applies the values an undo or a redo restores, adding their keys to
    /// the history of this update (JS `presence.set(value, { addToHistory:
    /// true })` in `executeUndoRedo`).
    pub(crate) fn restore(&mut self, values: ReversePresence) {
        for (key, value) in values {
            self.reverse_keys.insert(key.clone());
            match value {
                Some(value) => {
                    self.presence.insert(key, value);
                }
                None => {
                    self.presence.remove(&key);
                }
            }
        }
        self.change = Some(PresenceChange::Put(self.presence.clone()));
    }

    /// Returns the value of the given key (JS `presence.get`).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.presence.get(key)
    }

    /// Clears the presence (JS `presence.clear`).
    pub fn clear(&mut self) {
        self.presence = Presence::new();
        self.change = Some(PresenceChange::Clear);
    }

    #[cfg(test)]
    pub(crate) fn into_change(self) -> Option<PresenceChange> {
        self.change
    }

    /// Returns the presence change of this update and the values an undo of
    /// it restores (JS `getReversePresence`).
    pub(crate) fn into_parts(self) -> (Option<PresenceChange>, Option<ReversePresence>) {
        let reverse = (!self.reverse_keys.is_empty()).then(|| {
            self.reverse_keys
                .iter()
                .map(|key| (key.clone(), self.previous.get(key).cloned()))
                .collect()
        });
        (self.change, reverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn presence(value: Value) -> Presence {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn set_merges_keys_and_records_whole_presence() {
        let mut proxy = PresenceProxy::new(presence(json!({"a": 1})));
        proxy.set(presence(json!({"b": 2})));
        assert_eq!(Some(&json!(1)), proxy.get("a"));
        assert_eq!(
            Some(PresenceChange::Put(presence(json!({"a": 1, "b": 2})))),
            proxy.into_change()
        );
    }

    #[test]
    fn clear_then_set_puts_only_new_keys() {
        let mut proxy = PresenceProxy::new(presence(json!({"a": 1})));
        proxy.clear();
        assert_eq!(None, proxy.get("a"));
        proxy.set(presence(json!({"c": 3})));
        assert_eq!(
            Some(PresenceChange::Put(presence(json!({"c": 3})))),
            proxy.into_change()
        );
    }
}
