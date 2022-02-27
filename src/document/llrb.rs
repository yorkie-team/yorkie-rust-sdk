use std::cell::{RefCell, RefMut};
use std::cmp::Ordering;
use std::rc::Rc;

/// Key represents key of Tree.
pub trait Key {
    fn cmp(&self, k: &dyn Key) -> Ordering;
}

/// Value represents the data stored in the nodes of Tree.
pub trait Value {
    fn to_string(&self) -> String;
}

/// Node is a node of Tree.
pub struct Node {
    key: Box<dyn Key>,
    value: Box<dyn Value>,
    parent: Option<Rc<RefCell<Node>>>,
    left: Option<Rc<RefCell<Node>>>,
    right: Option<Rc<RefCell<Node>>>,
    is_red: bool,
}

impl Node {
    pub fn new(key: Box<dyn Key>, value: Box<dyn Value>, is_red: bool) -> Node {
        Node {
            key,
            value,
            is_red,
            parent: None,
            left: None,
            right: None,
        }
    }

    fn key(&self) -> &dyn Key {
        &*self.key
    }

    fn clone_left(&self) -> Option<Rc<RefCell<Node>>> {
        match &self.left {
            Some(l) => Some(Rc::clone(&l)),
            _ => None,
        }
    }

    fn clone_right(&self) -> Option<Rc<RefCell<Node>>> {
        match &self.right {
            Some(r) => Some(Rc::clone(&r)),
            _ => None,
        }
    }

    fn left_mut(&self) -> Option<RefMut<Node>> {
        match &self.left {
            Some(l) => Some(l.borrow_mut()),
            _ => None,
        }
    }

    fn right_mut(&self) -> Option<RefMut<Node>> {
        match &self.right {
            Some(r) => Some(r.borrow_mut()),
            _ => None,
        }
    }
}

/// Tree is an implementation of Left-learning Red-Black Tree.
/// Original paper on Left-leaning Red-Black Trees:
///  - http://www.cs.princeton.edu/~rs/talks/LLRB/LLRB.pdf
///
/// Invariant 1: No red node has a red child
/// Invariant 2: Every leaf path has the same number of black nodes
/// Invariant 3: Only the left child can be red (left leaning)
pub struct Tree {
    root: Option<Rc<RefCell<Node>>>,
    size: u64,
}

impl Tree {
    pub fn new() -> Tree {
        Tree {
            root: None,
            size: 0,
        }
    }

    pub fn insert(&mut self, k: Box<dyn Key>, v: Box<dyn Value>) {
        let root = match &self.root {
            Some(rc) => Some(Rc::clone(&rc)),
            _ => None,
        };

        self.root = self.insert_fix_up(root, k, v);
        if let Some(node) = &self.root {
            node.borrow_mut().is_red = true;
        }
    }

    fn insert_fix_up(
        &mut self,
        node: Option<Rc<RefCell<Node>>>,
        key: Box<dyn Key>,
        value: Box<dyn Value>,
    ) -> Option<Rc<RefCell<Node>>> {
        if let None = node {
            self.size += 1;
            return Some(Rc::new(RefCell::new(Node::new(key, value, true))));
        }

        let mut node_rc = node.as_ref().unwrap();
        let mut node = node_rc.borrow_mut();
        match key.cmp(node.key()) {
            Ordering::Less => node.left = self.insert_fix_up(node.clone_left(), key, value),
            Ordering::Greater => node.right = self.insert_fix_up(node.clone_right(), key, value),
            _ => node.value = value,
        }

        if is_red(&node.right) && !is_red(&node.left) {
            let node_rc = &rotate_left(node_rc);
        }

        if is_red(&node.left) {
            if let Some(l) = &node.left {
                if is_red(&l.borrow().left) {
                    let node_rc = &rotate_right(node_rc);
                }
            }
        }

        if is_red(&node.left) && is_red(&node.right) {
            flip_colors(node_rc);
        }

        Some(Rc::clone(node_rc))
    }
}

fn is_red(node: &Option<Rc<RefCell<Node>>>) -> bool {
    match node {
        Some(n) => n.borrow().is_red,
        _ => false,
    }
}

fn rotate_left(node_rc: &Rc<RefCell<Node>>) -> Rc<RefCell<Node>> {
    let mut node = node_rc.borrow_mut();

    let right = node.clone_right();
    let right_rc = right.as_ref().unwrap();
    let mut right = right_rc.borrow_mut();

    node.right = right.clone_left();
    right.left = Some(Rc::clone(node_rc));
    right.is_red = node.is_red;
    node.is_red = true;

    Rc::clone(&right_rc)
}

fn rotate_right(node_rc: &Rc<RefCell<Node>>) -> Rc<RefCell<Node>> {
    let mut node = node_rc.borrow_mut();

    let left = node.clone_left();
    let left_rc = left.as_ref().unwrap();
    let mut left = left_rc.borrow_mut();

    node.left = left.clone_right();
    left.right = Some(Rc::clone(node_rc));
    left.is_red = node.is_red;
    node.is_red = true;

    Rc::clone(&left_rc)
}

fn flip_colors(node_rc: &Rc<RefCell<Node>>) {
    let mut node = node_rc.borrow_mut();
    node.is_red = !node.is_red;

    if let Some(mut left) = node.left_mut() {
        left.is_red = !left.is_red;
    };

    if let Some(mut right) = node.right_mut() {
        right.is_red = !right.is_red;
    };
}

#[cfg(test)]
mod test {
    use super::*;
    struct TestKey {}

    impl TestKey {
        pub fn cmp(&self, k: &dyn Key) -> Ordering {
            Ordering::Less
        }
    }

    struct TestValue {}

    impl TestValue {
        pub fn to_string(&self) -> String {
            String::new()
        }
    }

    #[test]
    fn insert() {
        let tree = Tree::new();
        // tree.put();
    }
}
