//! Presence model, mirroring JS `document/presence/{presence,change}.ts`.

use serde_json::Value;

/// A client's presence: a JSON object of arbitrary keys.
pub type Presence = serde_json::Map<String, Value>;

/// A presence change carried by a `Change`.
///
/// `Put` carries the whole presence as of the change. `Clear` removes the
/// actor's presence. JS also has `Patch`; it is an upload-side optimisation
/// the server folds into `Put`, and is not ported.
#[derive(Debug, Clone, PartialEq)]
pub enum PresenceChange {
    Put(Presence),
    Clear,
}

/// The presence handle passed to `Document::update_with_presence`
/// (JS `Channel`/`presence` proxy in `presence.ts`).
#[derive(Debug, Clone)]
pub struct PresenceProxy {
    presence: Presence,
    change: Option<PresenceChange>,
}

impl PresenceProxy {
    pub(crate) fn new(presence: Presence) -> Self {
        Self {
            presence,
            change: None,
        }
    }

    /// Merges the given keys into the presence (JS `presence.set`).
    pub fn set(&mut self, presence: Presence) {
        for (key, value) in presence {
            self.presence.insert(key, value);
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

    pub(crate) fn into_change(self) -> Option<PresenceChange> {
        self.change
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
