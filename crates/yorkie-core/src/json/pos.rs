//! Plain-data forms of Text and Tree positions, for an editor to keep a
//! selection that survives concurrent edits (JS `TextPosStruct`,
//! `CRDTTreePosStruct` and their ID structs).
//!
//! The field names follow JS in Rust style; `created_at.lamport` is the
//! decimal string, as in `TimeTicketStruct`.

use crate::TimeTicketStruct;

/// The ID of a Text node (JS `RGATreeSplitNodeIDStruct`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextNodeIdStruct {
    pub created_at: TimeTicketStruct,
    pub offset: usize,
}

/// A position in a Text (JS `TextPosStruct`, the struct of
/// `RGATreeSplitPos`): `relative_offset` UTF-16 code units into the node
/// named by `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPosStruct {
    pub id: TextNodeIdStruct,
    pub relative_offset: usize,
}

/// A range of Text positions (JS `TextPosStructRange`).
pub type TextPosStructRange = (TextPosStruct, TextPosStruct);

/// The ID of a Tree node (JS `CRDTTreeNodeIDStruct`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNodeIdStruct {
    pub created_at: TimeTicketStruct,
    pub offset: usize,
}

/// A position in a Tree (JS `CRDTTreePosStruct`): right after
/// `left_sibling_id` inside `parent_id`, or at the start of the parent when
/// the two are the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreePosStruct {
    pub parent_id: TreeNodeIdStruct,
    pub left_sibling_id: TreeNodeIdStruct,
}

/// A range of Tree positions (JS `TreePosStructRange`).
pub type TreePosStructRange = (TreePosStruct, TreePosStruct);
