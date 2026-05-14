use super::array::CrdtArray;
use super::counter::CrdtCounter;
use super::object::CrdtObject;
use super::primitive::CrdtPrimitive;
use super::text::CrdtText;
use super::tree::CrdtTree;
use crate::{TimeTicket, TIME_TICKET_SIZE};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DataSize {
    pub(crate) data: usize,
    pub(crate) meta: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrdtElementMeta {
    created_at: TimeTicket,
    moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
}

impl CrdtElementMeta {
    pub(crate) fn new(created_at: TimeTicket) -> Self {
        Self {
            created_at,
            moved_at: None,
            removed_at: None,
        }
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        &self.created_at
    }

    pub(crate) fn id(&self) -> &TimeTicket {
        self.created_at()
    }

    pub(crate) fn moved_at(&self) -> Option<&TimeTicket> {
        self.moved_at.as_ref()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.removed_at.as_ref()
    }

    pub(crate) fn positioned_at(&self) -> &TimeTicket {
        self.moved_at.as_ref().unwrap_or(&self.created_at)
    }

    pub(crate) fn set_created_at(&mut self, created_at: TimeTicket) {
        self.created_at = created_at;
    }

    pub(crate) fn set_moved_at(&mut self, moved_at: Option<TimeTicket>) -> bool {
        if self.moved_at.is_none()
            || moved_at
                .as_ref()
                .is_some_and(|candidate| candidate.after(self.moved_at.as_ref().unwrap()))
        {
            self.moved_at = moved_at;
            return true;
        }

        false
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.removed_at = removed_at;
    }

    pub(crate) fn remove(&mut self, removed_at: Option<TimeTicket>) -> bool {
        let Some(removed_at) = removed_at else {
            return false;
        };

        if removed_at.after(&self.created_at)
            && self
                .removed_at
                .as_ref()
                .map(|current| removed_at.after(current))
                .unwrap_or(true)
        {
            self.removed_at = Some(removed_at);
            return true;
        }

        false
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    pub(crate) fn meta_usage(&self) -> usize {
        let mut meta = TIME_TICKET_SIZE;

        if self.moved_at.is_some() {
            meta += TIME_TICKET_SIZE;
        }

        if self.removed_at.is_some() {
            meta += TIME_TICKET_SIZE;
        }

        meta
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CrdtElement {
    Primitive(CrdtPrimitive),
    Counter(CrdtCounter),
    Object(Box<CrdtObject>),
    Array(Box<CrdtArray>),
    Text(Box<CrdtText>),
    Tree(Box<CrdtTree>),
}

impl CrdtElement {
    pub(crate) fn primitive(value: CrdtPrimitive) -> Self {
        Self::Primitive(value)
    }

    pub(crate) fn counter(value: CrdtCounter) -> Self {
        Self::Counter(value)
    }

    pub(crate) fn object(value: CrdtObject) -> Self {
        Self::Object(Box::new(value))
    }

    pub(crate) fn array(value: CrdtArray) -> Self {
        Self::Array(Box::new(value))
    }

    pub(crate) fn text(value: CrdtText) -> Self {
        Self::Text(Box::new(value))
    }

    pub(crate) fn tree(value: CrdtTree) -> Self {
        Self::Tree(Box::new(value))
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        match self {
            Self::Primitive(value) => value.created_at(),
            Self::Counter(value) => value.created_at(),
            Self::Object(value) => value.created_at(),
            Self::Array(value) => value.created_at(),
            Self::Text(value) => value.created_at(),
            Self::Tree(value) => value.created_at(),
        }
    }

    pub(crate) fn id(&self) -> &TimeTicket {
        match self {
            Self::Primitive(value) => value.id(),
            Self::Counter(value) => value.id(),
            Self::Object(value) => value.id(),
            Self::Array(value) => value.id(),
            Self::Text(value) => value.id(),
            Self::Tree(value) => value.id(),
        }
    }

    pub(crate) fn moved_at(&self) -> Option<&TimeTicket> {
        match self {
            Self::Primitive(value) => value.moved_at(),
            Self::Counter(value) => value.moved_at(),
            Self::Object(value) => value.moved_at(),
            Self::Array(value) => value.moved_at(),
            Self::Text(value) => value.moved_at(),
            Self::Tree(value) => value.moved_at(),
        }
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        match self {
            Self::Primitive(value) => value.removed_at(),
            Self::Counter(value) => value.removed_at(),
            Self::Object(value) => value.removed_at(),
            Self::Array(value) => value.removed_at(),
            Self::Text(value) => value.removed_at(),
            Self::Tree(value) => value.removed_at(),
        }
    }

    pub(crate) fn positioned_at(&self) -> &TimeTicket {
        match self {
            Self::Primitive(value) => value.positioned_at(),
            Self::Counter(value) => value.positioned_at(),
            Self::Object(value) => value.positioned_at(),
            Self::Array(value) => value.positioned_at(),
            Self::Text(value) => value.positioned_at(),
            Self::Tree(value) => value.positioned_at(),
        }
    }

    pub(crate) fn set_moved_at(&mut self, moved_at: Option<TimeTicket>) -> bool {
        match self {
            Self::Primitive(value) => value.set_moved_at(moved_at),
            Self::Counter(value) => value.set_moved_at(moved_at),
            Self::Object(value) => value.set_moved_at(moved_at),
            Self::Array(value) => value.set_moved_at(moved_at),
            Self::Text(value) => value.set_moved_at(moved_at),
            Self::Tree(value) => value.set_moved_at(moved_at),
        }
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        match self {
            Self::Primitive(value) => value.set_removed_at(removed_at),
            Self::Counter(value) => value.set_removed_at(removed_at),
            Self::Object(value) => value.set_removed_at(removed_at),
            Self::Array(value) => value.set_removed_at(removed_at),
            Self::Text(value) => value.set_removed_at(removed_at),
            Self::Tree(value) => value.set_removed_at(removed_at),
        }
    }

    pub(crate) fn remove(&mut self, removed_at: Option<TimeTicket>) -> bool {
        match self {
            Self::Primitive(value) => value.remove(removed_at),
            Self::Counter(value) => value.remove(removed_at),
            Self::Object(value) => value.remove(removed_at),
            Self::Array(value) => value.remove(removed_at),
            Self::Text(value) => value.remove(removed_at),
            Self::Tree(value) => value.remove(removed_at),
        }
    }

    pub(crate) fn is_removed(&self) -> bool {
        match self {
            Self::Primitive(value) => value.is_removed(),
            Self::Counter(value) => value.is_removed(),
            Self::Object(value) => value.is_removed(),
            Self::Array(value) => value.is_removed(),
            Self::Text(value) => value.is_removed(),
            Self::Tree(value) => value.is_removed(),
        }
    }

    pub(crate) fn meta_usage(&self) -> usize {
        match self {
            Self::Primitive(value) => value.meta_usage(),
            Self::Counter(value) => value.meta_usage(),
            Self::Object(value) => value.meta_usage(),
            Self::Array(value) => value.meta_usage(),
            Self::Text(value) => value.meta_usage(),
            Self::Tree(value) => value.meta_usage(),
        }
    }

    pub(crate) fn data_size(&self) -> DataSize {
        match self {
            Self::Primitive(value) => value.data_size(),
            Self::Counter(value) => value.data_size(),
            Self::Object(value) => value.data_size(),
            Self::Array(value) => value.data_size(),
            Self::Text(value) => value.data_size(),
            Self::Tree(value) => value.data_size(),
        }
    }

    pub(crate) fn to_json(&self) -> String {
        match self {
            Self::Primitive(value) => value.to_json(),
            Self::Counter(value) => value.to_json(),
            Self::Object(value) => value.to_json(),
            Self::Array(value) => value.to_json(),
            Self::Text(value) => value.to_json(),
            Self::Tree(value) => value.to_json(),
        }
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        match self {
            Self::Primitive(value) => value.to_sorted_json(),
            Self::Counter(value) => value.to_sorted_json(),
            Self::Object(value) => value.to_sorted_json(),
            Self::Array(value) => value.to_sorted_json(),
            Self::Text(value) => value.to_sorted_json(),
            Self::Tree(value) => value.to_sorted_json(),
        }
    }

    pub(crate) fn deepcopy(&self) -> Self {
        match self {
            Self::Primitive(value) => Self::Primitive(value.deepcopy()),
            Self::Counter(value) => Self::Counter(value.deepcopy()),
            Self::Object(value) => Self::Object(Box::new(value.deepcopy())),
            Self::Array(value) => Self::Array(Box::new(value.deepcopy())),
            Self::Text(value) => Self::Text(Box::new(value.deepcopy())),
            Self::Tree(value) => Self::Tree(Box::new(value.deepcopy())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CrdtElement, CrdtElementMeta};
    use crate::crdt::counter::{CounterType, CounterValue, CrdtCounter};
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::crdt::text::CrdtText;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::{TimeTicket, TIME_TICKET_SIZE};

    #[test]
    fn tracks_created_id_and_position() {
        let created_at = TimeTicket::new(1, 0, "a");
        let meta = CrdtElementMeta::new(created_at.clone());

        assert_eq!(&created_at, meta.created_at());
        assert_eq!(&created_at, meta.id());
        assert_eq!(&created_at, meta.positioned_at());
        assert_eq!(None, meta.moved_at());
        assert_eq!(TIME_TICKET_SIZE, meta.meta_usage());
    }

    #[test]
    fn updates_position_with_later_move_time() {
        let created_at = TimeTicket::new(1, 0, "a");
        let moved_at = TimeTicket::new(2, 0, "a");
        let older_moved_at = TimeTicket::new(1, 1, "a");
        let newer_moved_at = TimeTicket::new(3, 0, "a");
        let mut meta = CrdtElementMeta::new(created_at);

        assert!(meta.set_moved_at(Some(moved_at.clone())));
        assert_eq!(Some(&moved_at), meta.moved_at());
        assert_eq!(&moved_at, meta.positioned_at());
        assert_eq!(TIME_TICKET_SIZE * 2, meta.meta_usage());

        assert!(!meta.set_moved_at(Some(older_moved_at)));
        assert_eq!(Some(&moved_at), meta.moved_at());

        assert!(meta.set_moved_at(Some(newer_moved_at.clone())));
        assert_eq!(Some(&newer_moved_at), meta.moved_at());
    }

    #[test]
    fn removes_with_later_remove_time() {
        let created_at = TimeTicket::new(2, 0, "a");
        let older_removed_at = TimeTicket::new(1, 0, "a");
        let removed_at = TimeTicket::new(3, 0, "a");
        let newer_removed_at = TimeTicket::new(4, 0, "a");
        let mut meta = CrdtElementMeta::new(created_at);

        assert!(!meta.remove(None));
        assert!(!meta.remove(Some(older_removed_at)));
        assert!(!meta.is_removed());

        assert!(meta.remove(Some(removed_at.clone())));
        assert_eq!(Some(&removed_at), meta.removed_at());
        assert!(meta.is_removed());
        assert_eq!(TIME_TICKET_SIZE * 2, meta.meta_usage());

        assert!(!meta.remove(Some(TimeTicket::new(3, 0, "a"))));
        assert_eq!(Some(&removed_at), meta.removed_at());

        assert!(meta.remove(Some(newer_removed_at.clone())));
        assert_eq!(Some(&newer_removed_at), meta.removed_at());
    }

    #[test]
    fn sets_created_and_removed_times_directly() {
        let created_at = TimeTicket::new(1, 0, "a");
        let new_created_at = TimeTicket::new(2, 0, "a");
        let removed_at = TimeTicket::new(3, 0, "a");
        let mut meta = CrdtElementMeta::new(created_at);

        meta.set_created_at(new_created_at.clone());
        assert_eq!(&new_created_at, meta.created_at());

        meta.set_removed_at(Some(removed_at.clone()));
        assert_eq!(Some(&removed_at), meta.removed_at());

        meta.set_removed_at(None);
        assert_eq!(None, meta.removed_at());
    }

    #[test]
    fn delegates_element_operations_to_primitive() {
        let created_at = TimeTicket::new(1, 0, "a");
        let moved_at = TimeTicket::new(2, 0, "a");
        let removed_at = TimeTicket::new(3, 0, "a");
        let primitive = CrdtPrimitive::new(PrimitiveValue::String("hello".to_owned()), created_at);
        let mut element = CrdtElement::primitive(primitive);

        assert_eq!("\"hello\"", element.to_json());
        assert_eq!(element.created_at(), element.id());
        assert!(element.set_moved_at(Some(moved_at.clone())));
        assert_eq!(Some(&moved_at), element.moved_at());
        assert_eq!(&moved_at, element.positioned_at());

        assert!(element.remove(Some(removed_at.clone())));
        assert_eq!(Some(&removed_at), element.removed_at());
        assert!(element.is_removed());
        assert_eq!(TIME_TICKET_SIZE * 3, element.meta_usage());
        assert_eq!(element, element.deepcopy());
    }

    #[test]
    fn delegates_element_operations_to_counter() {
        let created_at = TimeTicket::new(1, 0, "a");
        let moved_at = TimeTicket::new(2, 0, "a");
        let removed_at = TimeTicket::new(3, 0, "a");
        let counter =
            CrdtCounter::create(CounterType::Integer, CounterValue::Integer(10), created_at);
        let mut element = CrdtElement::counter(counter);

        assert_eq!("10", element.to_json());
        assert_eq!(element.created_at(), element.id());
        assert!(element.set_moved_at(Some(moved_at.clone())));
        assert_eq!(Some(&moved_at), element.moved_at());
        assert_eq!(&moved_at, element.positioned_at());

        assert!(element.remove(Some(removed_at.clone())));
        assert_eq!(Some(&removed_at), element.removed_at());
        assert!(element.is_removed());
        assert_eq!(TIME_TICKET_SIZE * 3, element.meta_usage());
        assert_eq!(element, element.deepcopy());
    }

    #[test]
    fn delegates_element_operations_to_text() -> crate::Result<()> {
        let created_at = TimeTicket::new(1, 0, "a");
        let edited_at = TimeTicket::new(2, 0, "a");
        let moved_at = TimeTicket::new(3, 0, "a");
        let removed_at = TimeTicket::new(4, 0, "a");
        let mut text = CrdtText::create(created_at);
        text.edit_by_index(0, 0, "Hi", None, edited_at, None)?;

        let mut element = CrdtElement::text(text);

        assert_eq!(r#"[{"val":"Hi"}]"#, element.to_json());
        assert_eq!(element.created_at(), element.id());
        assert!(element.set_moved_at(Some(moved_at.clone())));
        assert_eq!(Some(&moved_at), element.moved_at());
        assert_eq!(&moved_at, element.positioned_at());

        assert!(element.remove(Some(removed_at.clone())));
        assert_eq!(Some(&removed_at), element.removed_at());
        assert!(element.is_removed());
        assert_eq!(element, element.deepcopy());
        Ok(())
    }

    #[test]
    fn delegates_element_operations_to_tree() {
        let created_at = TimeTicket::new(1, 0, "a");
        let moved_at = TimeTicket::new(4, 0, "a");
        let removed_at = TimeTicket::new(5, 0, "a");
        let tree = CrdtTree::create(
            TreeNode::create_element(
                TreeNodeId::new(TimeTicket::new(2, 0, "a"), 0),
                "root",
                None,
                vec![TreeNode::create_text(
                    TreeNodeId::new(TimeTicket::new(3, 0, "a"), 0),
                    "Hi",
                )],
            ),
            created_at,
        );
        let mut element = CrdtElement::tree(tree);

        assert_eq!(
            r#"{"type":"root","children":[{"type":"text","value":"Hi"}]}"#,
            element.to_json()
        );
        assert_eq!(element.created_at(), element.id());
        assert!(element.set_moved_at(Some(moved_at.clone())));
        assert_eq!(Some(&moved_at), element.moved_at());
        assert_eq!(&moved_at, element.positioned_at());

        assert!(element.remove(Some(removed_at.clone())));
        assert_eq!(Some(&removed_at), element.removed_at());
        assert!(element.is_removed());
        assert_eq!(element, element.deepcopy());
    }
}
