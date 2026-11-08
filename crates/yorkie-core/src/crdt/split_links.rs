//! Port of JS `src/document/crdt/split_links.ts` (Go
//! `pkg/document/crdt/split_links.go`).

use super::element::CrdtElement;

/// Strips the split-sibling links from every tree reachable from `element`
/// (JS `dropSplitLinksInElement`).
///
/// A Set, Add or ArraySet payload carries a whole element, and the wire
/// format carries `insPrevID`/`insNextID` on every tree node it holds. The
/// tree follows those links as trusted structural pointers, so they are
/// dropped on both ways into the document: the decoder drops them from every
/// such payload, and an undo drops them from the copy a reverse operation
/// captured, which never passes the decoder on the replica that runs it.
/// Without the second, that replica keeps links every other replica and the
/// server decoded away, and the two disagree from the next same-boundary
/// split on.
///
/// Removed members are walked too. The merge lineage (`merged_from`,
/// `merged_at`) is left alone: a payload can be a copy of real document state
/// that underwent those merges. A Text inside the payload is not touched,
/// as in JS.
pub(crate) fn drop_split_links_in_element(element: &mut CrdtElement) {
    match element {
        CrdtElement::Tree(tree) => tree.drop_split_links(),
        CrdtElement::Object(object) => object.drop_split_links(),
        CrdtElement::Array(array) => array.drop_split_links(),
        CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Text(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::drop_split_links_in_element;
    use crate::crdt::array::CrdtArray;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::object::CrdtObject;
    use crate::crdt::tree::{CrdtTree, TreeNode, TreeNodeId};
    use crate::{Result, TimeTicket};

    fn ticket(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, "a")
    }

    fn id(lamport: i64) -> TreeNodeId {
        TreeNodeId::new(ticket(lamport), 0)
    }

    /// `<r><p/><p/></r>` whose paragraphs are linked split siblings, and
    /// whose second one carries a merge stamp.
    fn linked_tree(lamport: i64) -> CrdtElement {
        let mut left = TreeNode::create_element(id(lamport + 1), "p", None, Vec::new());
        left.set_ins_next_id(Some(id(lamport + 2)));
        let mut right = TreeNode::create_element(id(lamport + 2), "p", None, Vec::new());
        right.set_ins_prev_id(Some(id(lamport + 1)));
        right.set_merged_from(Some(id(lamport + 1)));
        right.set_merged_at(Some(ticket(lamport + 3)));
        let root = TreeNode::create_element(id(lamport), "r", None, vec![left, right]);
        CrdtElement::tree(CrdtTree::create(root, ticket(lamport)))
    }

    /// Lists, for every tree under `element`, removed members included,
    /// whether a node still carries a link and whether the merge stamp is
    /// still there.
    fn trees_under(element: &CrdtElement, found: &mut Vec<(bool, bool)>) {
        match element {
            CrdtElement::Tree(tree) => {
                let nodes: Vec<&TreeNode> = tree.root().all_children().collect();
                let linked = nodes
                    .iter()
                    .any(|node| node.ins_prev_id().is_some() || node.ins_next_id().is_some());
                let merged = nodes.iter().any(|node| node.merged_from().is_some());
                found.push((linked, merged));
            }
            CrdtElement::Object(object) => {
                for (_, child) in object.iter_all() {
                    trees_under(child, found);
                }
            }
            CrdtElement::Array(array) => {
                for child in array.iter_all() {
                    trees_under(child, found);
                }
            }
            CrdtElement::Primitive(_) | CrdtElement::Counter(_) | CrdtElement::Text(_) => {}
        }
    }

    /// JS `dropSplitLinksInElement` walks removed members too, and keeps the
    /// merge lineage.
    #[test]
    fn drops_links_from_every_tree_in_a_container_and_keeps_merge_stamps() -> Result<()> {
        let mut array = CrdtArray::create(ticket(100));
        for lamport in [110, 120] {
            let last = array.get_last_created_at();
            array.insert_after(&last, linked_tree(lamport), Some(ticket(lamport)))?;
        }
        array.delete(&ticket(120), ticket(130))?;

        let mut object = CrdtObject::create(ticket(1));
        object.set("live", linked_tree(10), ticket(10));
        object.set("gone", linked_tree(20), ticket(20));
        object.delete_by_key("gone", ticket(30));
        object.set("list", CrdtElement::array(array), ticket(100));
        let mut element = CrdtElement::object(object);

        let mut before = Vec::new();
        trees_under(&element, &mut before);
        assert_eq!(vec![(true, true); 4], before);

        drop_split_links_in_element(&mut element);

        let mut after = Vec::new();
        trees_under(&element, &mut after);
        assert_eq!(vec![(false, true); 4], after);
        Ok(())
    }

    #[test]
    fn drops_links_from_a_tree_payload() {
        let mut element = linked_tree(1);
        drop_split_links_in_element(&mut element);

        let mut found = Vec::new();
        trees_under(&element, &mut found);
        assert_eq!(vec![(false, true)], found);
    }
}
