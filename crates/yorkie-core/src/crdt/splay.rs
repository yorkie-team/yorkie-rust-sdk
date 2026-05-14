use crate::{Result, YorkieError};

pub(crate) trait SplayValue {
    fn len(&self) -> usize;
    fn to_test_string(&self) -> String;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeId(usize);

#[derive(Debug, Clone, PartialEq)]
struct SplayNode<V>
where
    V: SplayValue,
{
    value: V,
    weight: usize,
    left: Option<NodeId>,
    right: Option<NodeId>,
    parent: Option<NodeId>,
}

impl<V> SplayNode<V>
where
    V: SplayValue,
{
    fn new(value: V) -> Self {
        let weight = value.len();
        Self {
            value,
            weight,
            left: None,
            right: None,
            parent: None,
        }
    }

    fn has_links(&self) -> bool {
        self.left.is_some() || self.right.is_some() || self.parent.is_some()
    }

    fn init_weight(&mut self) {
        self.weight = self.value.len();
    }

    fn unlink(&mut self) {
        self.left = None;
        self.right = None;
        self.parent = None;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SplayTree<V>
where
    V: SplayValue,
{
    root: Option<NodeId>,
    nodes: Vec<SplayNode<V>>,
}

impl<V> SplayTree<V>
where
    V: SplayValue,
{
    pub(crate) fn new() -> Self {
        Self {
            root: None,
            nodes: Vec::new(),
        }
    }

    pub(crate) fn with_root(value: V) -> Self {
        let mut tree = Self::new();
        tree.insert(value);
        tree
    }

    pub(crate) fn len(&self) -> usize {
        self.root.map(|root| self.node(root).weight).unwrap_or(0)
    }

    pub(crate) fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub(crate) fn value(&self, id: NodeId) -> &V {
        &self.node(id).value
    }

    pub(crate) fn value_mut(&mut self, id: NodeId) -> &mut V {
        &mut self.node_mut(id).value
    }

    pub(crate) fn insert(&mut self, value: V) -> NodeId {
        let id = self.push_node(value);
        match self.root {
            Some(root) => self.insert_existing_after(root, id),
            None => {
                self.root = Some(id);
                id
            }
        }
    }

    pub(crate) fn insert_after(&mut self, target: Option<NodeId>, value: V) -> NodeId {
        let id = self.push_node(value);
        match target {
            Some(target) => self.insert_existing_after(target, id),
            None => {
                self.root = Some(id);
                id
            }
        }
    }

    pub(crate) fn splay(&mut self, id: NodeId) {
        loop {
            let parent = self.node(id).parent;
            if self.is_left_child(parent) && self.is_right_child(Some(id)) {
                self.rotate_left(id);
                self.rotate_right(id);
            } else if self.is_right_child(parent) && self.is_left_child(Some(id)) {
                self.rotate_right(id);
                self.rotate_left(id);
            } else if self.is_left_child(parent) && self.is_left_child(Some(id)) {
                self.rotate_right(parent.unwrap());
                self.rotate_right(id);
            } else if self.is_right_child(parent) && self.is_right_child(Some(id)) {
                self.rotate_left(parent.unwrap());
                self.rotate_left(id);
            } else {
                if self.is_left_child(Some(id)) {
                    self.rotate_right(id);
                } else if self.is_right_child(Some(id)) {
                    self.rotate_left(id);
                }
                self.update_tree_weight(Some(id));
                return;
            }
        }
    }

    pub(crate) fn index_of(&mut self, id: NodeId) -> Option<usize> {
        if self.root != Some(id) && !self.node(id).has_links() {
            return None;
        }

        self.splay(id);
        Some(self.left_weight(id))
    }

    pub(crate) fn find_for_text(&mut self, mut position: usize) -> Result<Option<(NodeId, usize)>> {
        let Some(mut id) = self.root else {
            return Ok(None);
        };

        loop {
            if self.node(id).left.is_some() && position <= self.left_weight(id) {
                id = self.node(id).left.unwrap();
            } else if self.node(id).right.is_some()
                && self.left_weight(id) + self.value_len(id) < position
            {
                position -= self.left_weight(id) + self.value_len(id);
                id = self.node(id).right.unwrap();
            } else {
                position -= self.left_weight(id);
                break;
            }
        }

        if position > self.value_len(id) {
            return Err(YorkieError::InvalidIndex(format!(
                "position {position} > node length {}",
                self.value_len(id)
            )));
        }

        self.splay(id);
        Ok(Some((id, position)))
    }

    pub(crate) fn find_for_array(&mut self, mut index: usize) -> Result<Option<NodeId>> {
        let Some(mut id) = self.root else {
            return Ok(None);
        };

        if index >= self.len() {
            return Err(YorkieError::InvalidIndex(format!(
                "index {index}, length {}",
                self.len()
            )));
        }

        loop {
            if self.node(id).left.is_some() && index < self.left_weight(id) {
                id = self.node(id).left.unwrap();
            } else if self.node(id).right.is_some()
                && self.left_weight(id) + self.value_len(id) <= index
            {
                index -= self.left_weight(id) + self.value_len(id);
                id = self.node(id).right.unwrap();
            } else {
                break;
            }
        }

        self.splay(id);
        Ok(Some(id))
    }

    pub(crate) fn delete(&mut self, id: NodeId) {
        self.splay(id);

        let left_root = self.node(id).left;
        if let Some(left_root) = left_root {
            self.node_mut(left_root).parent = None;
        }

        let right_root = self.node(id).right;
        if let Some(right_root) = right_root {
            self.node_mut(right_root).parent = None;
        }

        if let Some(left_root) = left_root {
            self.root = Some(left_root);
            let rightmost = self.rightmost(left_root);
            self.splay(rightmost);
            let root = self.root.unwrap();
            self.node_mut(root).right = right_root;
            if let Some(right_root) = right_root {
                self.node_mut(right_root).parent = Some(root);
            }
        } else {
            self.root = right_root;
        }

        self.node_mut(id).unlink();
        if let Some(root) = self.root {
            self.update_weight(root);
        }
    }

    pub(crate) fn delete_range(&mut self, left_boundary: NodeId, right_boundary: Option<NodeId>) {
        if let Some(right_boundary) = right_boundary {
            self.splay(left_boundary);
            self.splay(right_boundary);
            if self.node(right_boundary).left != Some(left_boundary) {
                self.rotate_right(left_boundary);
            }
            self.cut_off_right(left_boundary);
        } else {
            self.splay(left_boundary);
            self.cut_off_right(left_boundary);
        }
    }

    pub(crate) fn to_test_string(&self) -> String {
        let mut ids = Vec::new();
        self.traverse_inorder(self.root, &mut ids);
        ids.into_iter()
            .map(|id| {
                let node = self.node(id);
                format!(
                    "[{},{}]{}",
                    node.weight,
                    node.value.len(),
                    node.value.to_test_string()
                )
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub(crate) fn check_weight(&self) -> bool {
        let mut ids = Vec::new();
        self.traverse_inorder(self.root, &mut ids);
        ids.into_iter().all(|id| {
            self.node(id).weight
                == self.value_len(id) + self.left_weight(id) + self.right_weight(id)
        })
    }

    fn push_node(&mut self, value: V) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(SplayNode::new(value));
        id
    }

    fn insert_existing_after(&mut self, target: NodeId, id: NodeId) -> NodeId {
        self.splay(target);

        let target_right = self.node(target).right;
        self.root = Some(id);
        self.node_mut(id).right = target_right;
        if let Some(target_right) = target_right {
            self.node_mut(target_right).parent = Some(id);
        }

        self.node_mut(id).left = Some(target);
        self.node_mut(target).parent = Some(id);
        self.node_mut(target).right = None;

        self.update_weight(target);
        self.update_weight(id);
        id
    }

    fn cut_off_right(&mut self, root: NodeId) {
        let mut ids = Vec::new();
        self.traverse_postorder(self.node(root).right, &mut ids);
        for id in ids {
            self.node_mut(id).init_weight();
        }
        self.update_tree_weight(Some(root));
    }

    fn update_weight(&mut self, id: NodeId) {
        let weight = self.value_len(id) + self.left_weight(id) + self.right_weight(id);
        self.node_mut(id).weight = weight;
    }

    fn update_tree_weight(&mut self, mut id: Option<NodeId>) {
        while let Some(current) = id {
            self.update_weight(current);
            id = self.node(current).parent;
        }
    }

    fn rotate_left(&mut self, pivot: NodeId) {
        let root = self.node(pivot).parent.unwrap();
        let grand_parent = self.node(root).parent;

        if let Some(grand_parent) = grand_parent {
            if self.node(grand_parent).left == Some(root) {
                self.node_mut(grand_parent).left = Some(pivot);
            } else {
                self.node_mut(grand_parent).right = Some(pivot);
            }
        } else {
            self.root = Some(pivot);
        }
        self.node_mut(pivot).parent = grand_parent;

        let pivot_left = self.node(pivot).left;
        self.node_mut(root).right = pivot_left;
        if let Some(pivot_left) = pivot_left {
            self.node_mut(pivot_left).parent = Some(root);
        }

        self.node_mut(pivot).left = Some(root);
        self.node_mut(root).parent = Some(pivot);

        self.update_weight(root);
        self.update_weight(pivot);
    }

    fn rotate_right(&mut self, pivot: NodeId) {
        let root = self.node(pivot).parent.unwrap();
        let grand_parent = self.node(root).parent;

        if let Some(grand_parent) = grand_parent {
            if self.node(grand_parent).left == Some(root) {
                self.node_mut(grand_parent).left = Some(pivot);
            } else {
                self.node_mut(grand_parent).right = Some(pivot);
            }
        } else {
            self.root = Some(pivot);
        }
        self.node_mut(pivot).parent = grand_parent;

        let pivot_right = self.node(pivot).right;
        self.node_mut(root).left = pivot_right;
        if let Some(pivot_right) = pivot_right {
            self.node_mut(pivot_right).parent = Some(root);
        }

        self.node_mut(pivot).right = Some(root);
        self.node_mut(root).parent = Some(pivot);

        self.update_weight(root);
        self.update_weight(pivot);
    }

    fn rightmost(&self, mut id: NodeId) -> NodeId {
        while let Some(right) = self.node(id).right {
            id = right;
        }
        id
    }

    fn traverse_inorder(&self, id: Option<NodeId>, ids: &mut Vec<NodeId>) {
        let Some(id) = id else {
            return;
        };
        self.traverse_inorder(self.node(id).left, ids);
        ids.push(id);
        self.traverse_inorder(self.node(id).right, ids);
    }

    fn traverse_postorder(&self, id: Option<NodeId>, ids: &mut Vec<NodeId>) {
        let Some(id) = id else {
            return;
        };
        self.traverse_postorder(self.node(id).left, ids);
        self.traverse_postorder(self.node(id).right, ids);
        ids.push(id);
    }

    fn is_left_child(&self, id: Option<NodeId>) -> bool {
        let Some(id) = id else {
            return false;
        };
        self.node(id)
            .parent
            .is_some_and(|parent| self.node(parent).left == Some(id))
    }

    fn is_right_child(&self, id: Option<NodeId>) -> bool {
        let Some(id) = id else {
            return false;
        };
        self.node(id)
            .parent
            .is_some_and(|parent| self.node(parent).right == Some(id))
    }

    fn node(&self, id: NodeId) -> &SplayNode<V> {
        &self.nodes[id.0]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut SplayNode<V> {
        &mut self.nodes[id.0]
    }

    fn value_len(&self, id: NodeId) -> usize {
        self.node(id).value.len()
    }

    fn left_weight(&self, id: NodeId) -> usize {
        self.node(id)
            .left
            .map(|left| self.node(left).weight)
            .unwrap_or(0)
    }

    fn right_weight(&self, id: NodeId) -> usize {
        self.node(id)
            .right
            .map(|right| self.node(right).weight)
            .unwrap_or(0)
    }
}

impl<V> Default for SplayTree<V>
where
    V: SplayValue,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{SplayTree, SplayValue};

    #[derive(Clone, PartialEq)]
    struct TextValue {
        content: String,
        removed: bool,
    }

    impl TextValue {
        fn new(content: &str) -> Self {
            Self {
                content: content.to_owned(),
                removed: false,
            }
        }
    }

    impl SplayValue for TextValue {
        fn len(&self) -> usize {
            if self.removed {
                return 0;
            }
            self.content.len()
        }

        fn to_test_string(&self) -> String {
            self.content.clone()
        }
    }

    #[derive(Clone, PartialEq)]
    struct ArrayValue {
        value: i32,
        removed: bool,
    }

    impl ArrayValue {
        fn new(value: i32) -> Self {
            Self {
                value,
                removed: false,
            }
        }
    }

    impl SplayValue for ArrayValue {
        fn len(&self) -> usize {
            (!self.removed) as usize
        }

        fn to_test_string(&self) -> String {
            self.value.to_string()
        }
    }

    #[test]
    fn inserts_and_splays_text_nodes() -> crate::Result<()> {
        let mut tree = SplayTree::new();

        assert_eq!(None, tree.find_for_text(0)?);

        let a = tree.insert(TextValue::new("A2"));
        assert_eq!("[2,2]A2", tree.to_test_string());
        let b = tree.insert(TextValue::new("B23"));
        assert_eq!("[2,2]A2[5,3]B23", tree.to_test_string());
        let c = tree.insert(TextValue::new("C234"));
        assert_eq!("[2,2]A2[5,3]B23[9,4]C234", tree.to_test_string());
        let d = tree.insert(TextValue::new("D2345"));
        assert_eq!("[2,2]A2[5,3]B23[9,4]C234[14,5]D2345", tree.to_test_string());

        tree.splay(b);
        assert_eq!("[2,2]A2[14,3]B23[9,4]C234[5,5]D2345", tree.to_test_string());

        assert_eq!(Some(0), tree.index_of(a));
        assert_eq!(Some(2), tree.index_of(b));
        assert_eq!(Some(5), tree.index_of(c));
        assert_eq!(Some(9), tree.index_of(d));

        let (node, offset) = tree.find_for_text(1)?.unwrap();
        assert_eq!(a, node);
        assert_eq!(1, offset);

        let (node, offset) = tree.find_for_text(7)?.unwrap();
        assert_eq!(c, node);
        assert_eq!(2, offset);

        let (node, offset) = tree.find_for_text(11)?.unwrap();
        assert_eq!(d, node);
        assert_eq!(2, offset);
        assert!(tree.check_weight());
        Ok(())
    }

    #[test]
    fn finds_array_nodes_after_tombstones() -> crate::Result<()> {
        let mut tree = SplayTree::new();

        assert_eq!(None, tree.find_for_array(0)?);

        let a = tree.insert(ArrayValue::new(2));
        assert_eq!("[1,1]2", tree.to_test_string());
        let b = tree.insert(ArrayValue::new(3));
        assert_eq!("[1,1]2[2,1]3", tree.to_test_string());
        let c = tree.insert(ArrayValue::new(4));
        assert_eq!("[1,1]2[2,1]3[3,1]4", tree.to_test_string());
        let d = tree.insert(ArrayValue::new(5));
        assert_eq!("[1,1]2[2,1]3[3,1]4[4,1]5", tree.to_test_string());

        tree.value_mut(b).removed = true;
        tree.splay(b);
        assert_eq!(3, tree.len());
        assert_eq!("[1,1]2[3,0]3[2,1]4[1,1]5", tree.to_test_string());
        assert_eq!(Some(0), tree.index_of(a));
        assert_eq!(Some(1), tree.index_of(c));
        assert_eq!(Some(2), tree.index_of(d));

        assert_eq!(a, tree.find_for_array(0)?.unwrap());
        assert_eq!(c, tree.find_for_array(1)?.unwrap());
        assert_eq!(d, tree.find_for_array(2)?.unwrap());
        assert!(tree.find_for_array(3).is_err());
        assert!(tree.check_weight());
        Ok(())
    }

    #[test]
    fn deletes_nodes() {
        let mut tree = SplayTree::new();

        let h = tree.insert(TextValue::new("H"));
        assert_eq!("[1,1]H", tree.to_test_string());
        let e = tree.insert(TextValue::new("E"));
        assert_eq!("[1,1]H[2,1]E", tree.to_test_string());
        let l = tree.insert(TextValue::new("LL"));
        assert_eq!("[1,1]H[2,1]E[4,2]LL", tree.to_test_string());
        let o = tree.insert(TextValue::new("O"));
        assert_eq!("[1,1]H[2,1]E[4,2]LL[5,1]O", tree.to_test_string());

        tree.delete(e);

        assert_eq!("[4,1]H[3,2]LL[1,1]O", tree.to_test_string());
        assert_eq!(4, tree.len());
        assert_eq!(Some(0), tree.index_of(h));
        assert_eq!(None, tree.index_of(e));
        assert_eq!(Some(1), tree.index_of(l));
        assert_eq!(Some(3), tree.index_of(o));
        assert!(tree.check_weight());
    }

    #[test]
    fn deletes_ranges_by_reweighting_tombstones() {
        let (mut tree, nodes) = sample_tree();
        remove_nodes(&mut tree, &nodes, 7, 8);
        tree.delete_range(nodes[6], None);
        assert_eq!(
            "[1,1]A[3,2]BB[6,3]CCC[10,4]DDDD[15,5]EEEEE[19,4]FFFF[22,3]GGG[0,0]HH[0,0]I",
            tree.to_test_string()
        );

        let (mut tree, nodes) = sample_tree();
        remove_nodes(&mut tree, &nodes, 3, 6);
        tree.delete_range(nodes[2], Some(nodes[7]));
        assert_eq!(
            "[1,1]A[3,2]BB[6,3]CCC[0,0]DDDD[0,0]EEEEE[0,0]FFFF[0,0]GGG[9,2]HH[1,1]I",
            tree.to_test_string()
        );

        let (mut tree, nodes) = sample_tree();
        tree.splay(nodes[6]);
        tree.splay(nodes[2]);
        remove_nodes(&mut tree, &nodes, 3, 7);
        tree.delete_range(nodes[2], Some(nodes[8]));
        assert_eq!(
            "[1,1]A[3,2]BB[6,3]CCC[0,0]DDDD[0,0]EEEEE[0,0]FFFF[0,0]GGG[0,0]HH[7,1]I",
            tree.to_test_string()
        );
    }

    fn sample_tree() -> (SplayTree<TextValue>, Vec<super::NodeId>) {
        let mut tree = SplayTree::new();
        let nodes = ["A", "BB", "CCC", "DDDD", "EEEEE", "FFFF", "GGG", "HH", "I"]
            .into_iter()
            .map(|value| tree.insert(TextValue::new(value)))
            .collect();
        (tree, nodes)
    }

    fn remove_nodes(
        tree: &mut SplayTree<TextValue>,
        nodes: &[super::NodeId],
        from: usize,
        to: usize,
    ) {
        for node in nodes.iter().take(to + 1).skip(from) {
            tree.value_mut(*node).removed = true;
            tree.node_mut(*node).init_weight();
        }
    }
}
