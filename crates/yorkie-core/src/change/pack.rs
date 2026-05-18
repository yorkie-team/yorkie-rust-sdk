use super::{Change, Checkpoint};
use crate::crdt::object::CrdtObject;
use crate::VersionVector;

#[derive(Debug, Clone, PartialEq)]
pub struct ChangePack {
    document_key: String,
    checkpoint: Checkpoint,
    is_removed: bool,
    changes: Vec<Change>,
    snapshot: Option<Vec<u8>>,
    snapshot_root: Option<CrdtObject>,
    version_vector: Option<VersionVector>,
}

impl ChangePack {
    pub(crate) fn new(
        document_key: impl Into<String>,
        checkpoint: Checkpoint,
        is_removed: bool,
        changes: Vec<Change>,
        version_vector: Option<VersionVector>,
        snapshot: Option<Vec<u8>>,
    ) -> Self {
        Self {
            document_key: document_key.into(),
            checkpoint,
            is_removed,
            changes,
            snapshot,
            snapshot_root: None,
            version_vector,
        }
    }

    pub(crate) fn create(
        document_key: impl Into<String>,
        checkpoint: Checkpoint,
        is_removed: bool,
        changes: Vec<Change>,
        version_vector: Option<VersionVector>,
        snapshot: Option<Vec<u8>>,
    ) -> Self {
        Self::new(
            document_key,
            checkpoint,
            is_removed,
            changes,
            version_vector,
            snapshot,
        )
    }

    pub(crate) fn create_with_snapshot_root(
        document_key: impl Into<String>,
        checkpoint: Checkpoint,
        is_removed: bool,
        changes: Vec<Change>,
        version_vector: Option<VersionVector>,
        snapshot: Option<Vec<u8>>,
        snapshot_root: Option<CrdtObject>,
    ) -> Self {
        let mut pack = Self::new(
            document_key,
            checkpoint,
            is_removed,
            changes,
            version_vector,
            snapshot,
        );
        pack.snapshot_root = snapshot_root;
        pack
    }

    pub fn document_key(&self) -> &str {
        &self.document_key
    }

    pub fn checkpoint(&self) -> Checkpoint {
        self.checkpoint
    }

    pub fn is_removed(&self) -> bool {
        self.is_removed
    }

    pub fn set_removed(&mut self, is_removed: bool) {
        self.is_removed = is_removed;
    }

    pub(crate) fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }

    pub fn change_size(&self) -> usize {
        self.changes.len()
    }

    pub(crate) fn operations_len(&self) -> usize {
        self.changes
            .iter()
            .map(|change| change.operations().len())
            .sum()
    }

    pub fn has_snapshot(&self) -> bool {
        self.snapshot
            .as_ref()
            .map(|snapshot| !snapshot.is_empty())
            .unwrap_or(false)
            || self.snapshot_root.is_some()
    }

    pub fn snapshot(&self) -> Option<&[u8]> {
        self.snapshot.as_deref()
    }

    pub(crate) fn snapshot_root(&self) -> Option<&CrdtObject> {
        self.snapshot_root.as_ref()
    }

    pub fn version_vector(&self) -> Option<&VersionVector> {
        self.version_vector.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::ChangePack;
    use crate::change::{Change, ChangeId, Checkpoint};
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::operation::{Operation, SetOperation};
    use crate::{TimeTicket, VersionVector};

    #[test]
    fn creates_change_pack_with_metadata() {
        let change = Change::create(
            ChangeId::initial().next(false),
            vec![Operation::Set(SetOperation::create(
                "title",
                primitive_str("hello", TimeTicket::new(1, 1, "actor-a")),
                TimeTicket::initial(),
                Some(TimeTicket::new(1, 1, "actor-a")),
            ))],
            None,
        );
        let mut version_vector = VersionVector::new();
        version_vector.set("actor-a", 1);

        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::new(1, 2),
            false,
            vec![change],
            Some(version_vector.clone()),
            None,
        );

        assert_eq!("doc-key", pack.document_key());
        assert_eq!(Checkpoint::new(1, 2), pack.checkpoint());
        assert!(!pack.is_removed());
        assert!(pack.has_changes());
        assert_eq!(1, pack.change_size());
        assert_eq!(1, pack.operations_len());
        assert!(!pack.has_snapshot());
        assert_eq!(Some(&version_vector), pack.version_vector());
    }

    #[test]
    fn detects_non_empty_snapshot() {
        let pack = ChangePack::create(
            "doc-key",
            Checkpoint::initial(),
            false,
            Vec::new(),
            None,
            Some(vec![1, 2, 3]),
        );

        assert!(!pack.has_changes());
        assert_eq!(0, pack.change_size());
        assert!(pack.has_snapshot());
        assert_eq!(Some([1, 2, 3].as_slice()), pack.snapshot());
    }

    fn primitive_str(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }
}
