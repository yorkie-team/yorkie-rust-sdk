use super::{ActorId, TimeTicket};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionVector {
    vector: BTreeMap<ActorId, i64>,
}

impl VersionVector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, actor_id: impl Into<ActorId>, lamport: i64) {
        self.vector.insert(actor_id.into(), lamport);
    }

    pub fn unset(&mut self, actor_id: &str) {
        self.vector.remove(actor_id);
    }

    pub fn get(&self, actor_id: &str) -> Option<i64> {
        self.vector.get(actor_id).copied()
    }

    pub fn has(&self, actor_id: &str) -> bool {
        self.vector.contains_key(actor_id)
    }

    pub fn max_lamport(&self) -> i64 {
        self.vector.values().copied().max().unwrap_or_default()
    }

    pub fn max(&self, other: &Self) -> Self {
        let mut max_vector = Self::new();

        for (actor_id, lamport) in &other.vector {
            let current_lamport = self.vector.get(actor_id).copied();
            max_vector.set(
                actor_id.clone(),
                current_lamport
                    .map(|current| current.max(*lamport))
                    .unwrap_or(*lamport),
            );
        }

        for (actor_id, lamport) in &self.vector {
            let other_lamport = other.vector.get(actor_id).copied();
            max_vector.set(
                actor_id.clone(),
                other_lamport
                    .map(|other| other.max(*lamport))
                    .unwrap_or(*lamport),
            );
        }

        max_vector
    }

    pub fn after_or_equal(&self, other: &TimeTicket) -> bool {
        self.get(other.actor_id().as_str())
            .map(|lamport| lamport >= other.lamport())
            .unwrap_or(false)
    }

    pub fn filter(&self, version_vector: &Self) -> Self {
        let mut filtered = Self::new();

        for actor_id in version_vector.vector.keys() {
            if let Some(lamport) = self.vector.get(actor_id) {
                filtered.set(actor_id.clone(), *lamport);
            }
        }

        filtered
    }

    pub fn size(&self) -> usize {
        self.vector.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ActorId, i64)> {
        self.vector
            .iter()
            .map(|(actor_id, lamport)| (actor_id, *lamport))
    }
}

impl<'a> IntoIterator for &'a VersionVector {
    type Item = (&'a ActorId, i64);
    type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}
