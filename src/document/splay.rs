use std::cell::RefCell;
use std::rc::Rc;

type RcNode<V> = Rc<RefCell<Node<V>>>;
type OptionNode<V> = Option<RcNode<V>>;

pub trait Value: Clone {
    fn len(&self) -> usize;
    fn to_string(&self) -> String;
}

#[derive(PartialEq)]
pub struct Node<V: Value> {
    value: V,
    weight: u64,

    parent: OptionNode<V>,
    left: OptionNode<V>,
    right: OptionNode<V>,
}

impl<V: Value> Node<V> {
    pub fn new(value: V) -> Self {
        let mut n = Node {
            value,
            weight: 0,
            parent: None,
            left: None,
            right: None,
        };

        n.init_weight();
        n
    }

    pub fn value(&self) -> V {
        self.value.clone()
    }

    fn left_weight(&self) -> u64 {
        if self.left.is_none() {
            return 0;
        }

        self.left.as_ref().unwrap().borrow().weight
    }

    fn right_weight(&self) -> u64 {
        if self.right.is_none() {
            return 0;
        }

        self.right.as_ref().unwrap().borrow().weight
    }

    fn left_right(&self) -> u64 {
        if self.right.is_none() {
            return 0;
        }

        self.right.as_ref().unwrap().borrow().weight
    }

    fn init_weight(&mut self) {
        self.weight = self.value.len() as u64;
    }

    fn increase_weight(&mut self, weight: u64) {
        self.weight += weight;
    }

    fn unlink(&mut self) {
        self.parent = None;
        self.left = None;
        self.right = None;
    }

    fn has_links(&self) -> bool {
        !self.parent.is_none() || !self.left.is_none() || !self.right.is_none()
    }
}

pub struct Tree<V: Value> {
    root: OptionNode<V>,
}

impl<V: Value> Tree<V> {
    pub fn new(root: Node<V>) -> Self {
        Tree {
            root: Some(Rc::new(RefCell::new(root))),
        }
    }

    pub fn insert(&mut self, node: Node<V>) -> RcNode<V> {
        if self.root.is_none() {
            let node_rc = Rc::new(RefCell::new(node));
            self.root = Some(node_rc);
            return Rc::clone(&node_rc);
        }

        let root = self.root.unwrap().as_ref();
        self.insert_after(Rc::clone(root), node)
    }

    // pub fn insert_after(&mut self, prev: RcNode<V>, node: Node<V>) -> RcNode<V> {
    //     self.splay(Rc::clone(&prev));
    //     self.root = Rc::new(RefCell::new(node));
        
    //     node.right = prev.right

    // }

    pub fn splay(&mut self, node_rc: RcNode<V>) {
        loop {
            let node = node_rc.borrow();
            let parent_rc = node.parent.as_ref().unwrap();

            if is_left_child(Rc::clone(&parent_rc)) && is_right_child(Rc::clone(&node_rc)) {
                // zig-zag
                self.rotate_left(Rc::clone(&node_rc));
                self.rotate_right(Rc::clone(&node_rc));
            } else if is_right_child(Rc::clone(&parent_rc)) && is_left_child(Rc::clone(&node_rc)) {
                self.rotate_right(Rc::clone(&node_rc));
                self.rotate_left(Rc::clone(&node_rc));
            } else if is_left_child(Rc::clone(&parent_rc)) && is_left_child(Rc::clone(&node_rc)) {
                self.rotate_left(Rc::clone(&parent_rc));
                self.rotate_left(Rc::clone(&node_rc));
            } else {
                if is_left_child(Rc::clone(&node_rc)) {
                    self.rotate_right(Rc::clone(&node_rc));
                } else if is_right_child(Rc::clone(&node_rc)) {
                    self.rotate_left(Rc::clone(&node_rc));
                }
                return;
            }
        }
    }

    fn rotate_left(&mut self, pivot_rc: RcNode<V>) {
        let pivot = pivot_rc.borrow();
        let root_rc = pivot.parent.as_ref().unwrap();
        let mut root = root_rc.borrow_mut();

        if let Some(parent) = root.parent.as_ref() {
            let mut parent = parent.borrow_mut();
            if Rc::ptr_eq(&root_rc, &parent.left.as_ref().unwrap()) {
                parent.left = Some(Rc::clone(&pivot_rc));
            } else {
                parent.right = Some(Rc::clone(&pivot_rc));
            }
        } else {
            self.root = Some(Rc::clone(&pivot_rc));
        }

        let mut pivot = pivot_rc.borrow_mut();
        pivot.parent = root.parent.clone();

        root.right = pivot.left.clone();
        if let Some(right) = root.right.as_ref() {
            let mut right = right.borrow_mut();
            right.parent = Some(Rc::clone(&root_rc));
        }

        pivot.left = Some(Rc::clone(&root_rc));
        let pivot_left = pivot.left.as_ref().unwrap();
        let mut pivot_left = pivot_left.borrow_mut();
        pivot_left.parent = Some(Rc::clone(&pivot_rc));

        self.update_subtree(Rc::clone(&root_rc));
        self.update_subtree(Rc::clone(&pivot_rc));
    }

    fn rotate_right(&mut self, pivot_rc: RcNode<V>) {
        let pivot = pivot_rc.borrow();
        let root_rc = pivot.parent.as_ref().unwrap();
        let mut root = root_rc.borrow_mut();

        if let Some(parent) = root.parent.as_ref() {
            let mut parent = parent.borrow_mut();
            if Rc::ptr_eq(&root_rc, &parent.left.as_ref().unwrap()) {
                parent.left = Some(Rc::clone(&pivot_rc));
            } else {
                parent.right = Some(Rc::clone(&pivot_rc));
            }
        } else {
            self.root = Some(Rc::clone(&pivot_rc));
        }

        let mut pivot = pivot_rc.borrow_mut();
        pivot.parent = root.parent.clone();

        root.left = pivot.right.clone();
        if let Some(left) = root.left.as_ref() {
            let mut left = left.borrow_mut();
            left.parent = Some(Rc::clone(&root_rc));
        }

        pivot.right = Some(Rc::clone(&root_rc));
        let pivot_right = pivot.right.as_ref().unwrap();
        let mut pivot_right = pivot_right.borrow_mut();
        pivot_right.parent = Some(Rc::clone(&pivot_rc));

        self.update_subtree(Rc::clone(&root_rc));
        self.update_subtree(Rc::clone(&pivot_rc));
    }

    pub fn update_subtree(&self, node_rc: RcNode<V>) {
        let mut node = node_rc.borrow_mut();
        node.init_weight();

        if !node.left.is_none() {
            let left_weight = node.left_weight();
            node.increase_weight(left_weight);
        }

        if !node.right.is_none() {
            let right_weight = node.right_weight();
            node.increase_weight(right_weight);
        }
    }
}

fn is_left_child<V: Value>(node: RcNode<V>) -> bool {
    match node.borrow().parent.as_ref() {
        Some(n) => {
            let parent = n.borrow();
            let left = parent.left.as_ref();
            if left.is_none() {
                return false;
            }

            Rc::ptr_eq(&left.unwrap(), &node)
        }
        _ => false,
    }
}

fn is_right_child<V: Value>(node: RcNode<V>) -> bool {
    match node.borrow().parent.as_ref() {
        Some(n) => {
            let parent = n.borrow();
            let right = parent.right.as_ref();
            if right.is_none() {
                return false;
            }

            Rc::ptr_eq(&right.unwrap(), &node)
        }
        _ => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Clone)]
    struct TestValue {
        value: String,
    }

    impl TestValue {
        pub fn new(value: &str) -> Self {
            TestValue {
                value: value.to_string(),
            }
        }
    }

    impl Value for TestValue {
        fn len(&self) -> usize {
            self.value.len()
        }

        fn to_string(&self) -> String {
            self.value.to_string()
        }
    }

    // fn new_node() {
    //     let value = TestValue::new("hello");
    //     let node = Node::new(value);
    // }
}
