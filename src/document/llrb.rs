use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

type RcNode<K, V> = Rc<RefCell<Node<K, V>>>;
type OptionNode<K, V> = Option<RcNode<K, V>>;

/// Key represents key of Tree.
pub trait Key: Clone {
    fn cmp(&self, other: &Self) -> Ordering;
}

/// Value represents the data stored in the nodes of Tree.
pub trait Value: Clone {
    fn to_string(&self) -> String;
}

/// Node is a node of Tree.
pub struct Node<K: Key, V: Value> {
    key: K,
    value: V,
    parent: OptionNode<K, V>,
    left: OptionNode<K, V>,
    right: OptionNode<K, V>,
    is_red: bool,
}

impl<K: Key, V: Value> Node<K, V> {
    pub fn new(key: K, value: V, is_red: bool) -> Self {
        Node {
            key,
            value,
            is_red,
            parent: None,
            left: None,
            right: None,
        }
    }

    fn key(&self) -> &K {
        &self.key
    }

    fn clone_left(&self) -> OptionNode<K, V> {
        match &self.left {
            Some(l) => Some(Rc::clone(&l)),
            _ => None,
        }
    }

    fn clone_right(&self) -> OptionNode<K, V> {
        match &self.right {
            Some(r) => Some(Rc::clone(&r)),
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
pub struct Tree<K: Key, V: Value> {
    root: OptionNode<K, V>,
    size: u64,
}

impl<K: Key, V: Value> Tree<K, V> {
    pub fn new() -> Self {
        Tree {
            root: None,
            size: 0,
        }
    }

    /// insert inserts the value of the given key.
    pub fn insert(&mut self, k: K, v: V) {
        let root = match &self.root {
            Some(rc) => Some(Rc::clone(&rc)),
            _ => None,
        };

        self.root = self.insert_fix_up(root, k, v);
        if let Some(node) = &self.root {
            node.borrow_mut().is_red = true;
        }
    }

    fn insert_fix_up(&mut self, node: OptionNode<K, V>, key: K, value: V) -> OptionNode<K, V> {
        if node.is_none() {
            self.size += 1;
            return Some(Rc::new(RefCell::new(Node::new(key, value, true))));
        }

        let mut node_rc = node.unwrap();
        {
            let mut node = node_rc.borrow_mut();
            match key.cmp(node.key()) {
                Ordering::Less => node.left = self.insert_fix_up(node.clone_left(), key, value),
                Ordering::Greater => {
                    node.right = self.insert_fix_up(node.clone_right(), key, value)
                }
                _ => node.value = value,
            }
        }

        if need_rotate_left(&node_rc) {
            node_rc = rotate_left(&node_rc);
        }

        if need_rotate_right(&node_rc) {
            node_rc = rotate_right(&node_rc);
        }

        if need_flip_colors(&node_rc) {
            flip_colors(&node_rc);
        }

        Some(Rc::clone(&node_rc))
    }

    /// remove removes the value of the given key.
    pub fn remove(&mut self, key: K) {
        if self.root.is_none() {
            return;
        }

        let root = Rc::clone(self.root.as_ref().unwrap());
        {
            let mut root = root.borrow_mut();
            if !is_red(&root.left) && !is_red(&root.right) {
                root.is_red = true;
            }
        }

        self.root = self.remove_fix_up(root, key)
    }

    fn remove_fix_up(&mut self, mut node_rc: RcNode<K, V>, key: K) -> OptionNode<K, V> {
        let compared = key.cmp(&node_rc.borrow().key);
        match compared {
            Ordering::Less => {
                {
                    let node = node_rc.borrow();
                    if !is_red(&node.left) {
                        let left_rc = node.left.as_ref().unwrap();
                        let left = left_rc.borrow();
                        if !is_red(&left.left) {
                            drop(left);
                            drop(node);
                            node_rc = move_red_left(Rc::clone(&node_rc));
                        }
                    }
                }
                let mut node = node_rc.borrow_mut();
                node.left = self.remove_fix_up(Rc::clone(node.left.as_ref().unwrap()), key);
            }
            _ => {
                {
                    let node = node_rc.borrow();
                    if is_red(&node.left) {
                        drop(node);
                        node_rc = rotate_right(&node_rc);
                    }
                }

                {
                    let node = node_rc.borrow();
                    if let Ordering::Equal = key.cmp(&node.key) {
                        if node.right.is_none() {
                            self.size -= 1;
                            return None;
                        }
                    }

                    if !is_red(&node.right) {
                        let right_rc = node.right.as_ref().unwrap();
                        let right = right_rc.borrow();
                        if !is_red(&right.left) {
                            drop(right);
                            drop(node);
                            node_rc = move_red_right(Rc::clone(&node_rc));
                        }
                    }
                }

                let mut node = node_rc.borrow_mut();
                if let Ordering::Equal = key.cmp(&node.key) {
                    self.size -= 1;
                    let right_rc = node.right.as_ref().unwrap();
                    let smallest = min(&right_rc);
                    {
                        let smallest = smallest.borrow();
                        node.value = smallest.value.clone();
                        node.key = smallest.key.clone();
                    }

                    let right_rc = node.right.as_ref().unwrap();
                    node.right = remove_min(Rc::clone(right_rc));
                } else {
                    let right_rc = node.right.as_ref().unwrap();
                    node.right = self.remove_fix_up(Rc::clone(right_rc), key);
                }
            }
        }
        Some(fix_up(Rc::clone(&node_rc)))
    }

    /// to_string returns a data string sorted in ascending key order.
    pub fn to_string(&self) -> String {
        let mut strings: Vec<String> = Vec::new();
        traverse_in_order(&self.root.as_ref(), &mut |node: &Node<K, V>| {
            strings.push(node.value.to_string())
        });
        strings.join(",")
    }

    /// floor returns the greatest key less than or equal to the given key.
    pub fn floor(&self, key: K) -> Option<(K, V)> {
        let root = &self.root;
        if root.is_none() {
            return None;
        }

        let node_rc = root.as_ref().unwrap();
        let mut node_option = Some(Rc::clone(node_rc));

        while let Some(node_rc) = node_option {
            let node = node_rc.borrow();
            match key.cmp(&node.key) {
                Ordering::Greater => {
                    if let Some(right_rc) = &node.right {
                        let mut right = right_rc.borrow_mut();
                        right.parent = Some(Rc::clone(&node_rc));
                        node_option = Some(Rc::clone(right_rc));
                    } else {
                        return Some((node.key.clone(), node.value.clone()));
                    }
                }
                Ordering::Less => {
                    if let Some(left_rc) = &node.left {
                        let mut left = left_rc.borrow_mut();
                        left.parent = Some(Rc::clone(&node_rc));
                        node_option = Some(Rc::clone(left_rc));
                    } else {
                        let parent_rc = node.parent.as_ref().unwrap();
                        let mut parent = Some(Rc::clone(parent_rc));
                        let mut child = Some(Rc::clone(&node_rc));

                        loop {
                            if parent.is_none() {
                                return None;
                            }

                            if child.is_none() {
                                return None;
                            }

                            let parent_rc = parent.as_ref().unwrap();
                            let child_rc = child.as_ref().unwrap();
                            if !Rc::ptr_eq(parent_rc, child_rc) {
                                break;
                            }
                            drop(parent_rc);
                            drop(child_rc);

                            child = parent;
                            let child_rc = child.as_ref().unwrap();
                            let child = child_rc.borrow();
                            let parent_rc = &child.parent.as_ref().unwrap();
                            parent = Some(Rc::clone(&parent_rc));
                        }

                        match parent {
                            Some(parent_rc) => {
                                let parent = parent_rc.borrow();
                                return Some((parent.key.clone(), parent.value.clone()));
                            }
                            _ => return None,
                        }
                    }
                }
                _ => return Some((node.key.clone(), node.value.clone())),
            }
        }

        return None;
    }
}

fn is_red<K: Key, V: Value>(node: &OptionNode<K, V>) -> bool {
    match node {
        Some(n) => n.borrow().is_red,
        _ => false,
    }
}

fn need_rotate_left<K: Key, V: Value>(node_rc: &RcNode<K, V>) -> bool {
    let node = node_rc.borrow();
    is_red(&node.right) && !is_red(&node.left)
}

fn need_rotate_right<K: Key, V: Value>(node_rc: &RcNode<K, V>) -> bool {
    let node = node_rc.borrow();
    if is_red(&node.left) {
        if let Some(l) = &node.left {
            return is_red(&l.borrow().left);
        }
    }

    false
}

fn need_flip_colors<K: Key, V: Value>(node_rc: &RcNode<K, V>) -> bool {
    let node = node_rc.borrow();
    is_red(&node.left) && is_red(&node.right)
}

fn rotate_left<K: Key, V: Value>(node_rc: &RcNode<K, V>) -> RcNode<K, V> {
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

fn rotate_right<K: Key, V: Value>(node_rc: &RcNode<K, V>) -> RcNode<K, V> {
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

fn flip_colors<K: Key, V: Value>(node_rc: &RcNode<K, V>) {
    let mut node = node_rc.borrow_mut();
    node.is_red = !node.is_red;

    if let Some(left) = node.left.as_ref() {
        let mut left = left.borrow_mut();
        left.is_red = !left.is_red;
    };

    if let Some(right) = node.right.as_ref() {
        let mut right = right.borrow_mut();
        right.is_red = !right.is_red;
    };
}

fn move_red_left<K: Key, V: Value>(mut node_rc: RcNode<K, V>) -> RcNode<K, V> {
    flip_colors(&node_rc);

    let mut node = node_rc.borrow_mut();
    let right_rc = node.right.as_ref().unwrap();
    let right = right_rc.borrow();

    if is_red(&right.left) {
        drop(right);
        drop(right_rc);
        node.right = Some(rotate_right(node.right.as_ref().unwrap()));
        drop(node);
        node_rc = rotate_left(&node_rc);
        flip_colors(&node_rc);
    }

    Rc::clone(&node_rc)
}

fn move_red_right<K: Key, V: Value>(mut node_rc: RcNode<K, V>) -> RcNode<K, V> {
    flip_colors(&node_rc);

    let node = node_rc.borrow();
    let left_rc = node.left.as_ref().unwrap();
    let left = left_rc.borrow();

    if is_red(&left.left) {
        drop(left);
        drop(node);
        node_rc = rotate_right(&node_rc);
        flip_colors(&node_rc);
    }

    Rc::clone(&node_rc)
}

fn fix_up<K: Key, V: Value>(mut node_rc: RcNode<K, V>) -> RcNode<K, V> {
    {
        let node = node_rc.borrow();
        if is_red(&node.right) {
            drop(node);
            node_rc = rotate_left(&node_rc);
        }
    }

    {
        let node = node_rc.borrow();
        if is_red(&node.left) {
            let left = &node.left.as_ref().unwrap();
            if is_red(&left.borrow().left) {
                drop(node);
                node_rc = rotate_right(&node_rc);
            }
        }
    }

    let node = node_rc.borrow();
    if is_red(&node.left) {
        let left = &node.left.as_ref().unwrap();
        if is_red(&left.borrow().right) {
            drop(node);
            flip_colors(&node_rc);
        }
    }

    Rc::clone(&node_rc)
}

fn min<K: Key, V: Value>(node_rc: &RcNode<K, V>) -> RcNode<K, V> {
    let node = node_rc.borrow();
    if node.left.is_none() {
        return Rc::clone(node_rc);
    }

    min(node.left.as_ref().unwrap())
}

fn remove_min<K: Key, V: Value>(mut node_rc: RcNode<K, V>) -> OptionNode<K, V> {
    {
        let node = node_rc.borrow();
        if node.left.is_none() {
            return None;
        }
    }

    {
        let node = node_rc.borrow();
        if !is_red(&node.left) {
            let left_rc = node.left.as_ref().unwrap();
            let left = left_rc.borrow();
            if !is_red(&left.left) {
                drop(left);
                drop(node);
                node_rc = move_red_left(Rc::clone(&node_rc));
            }
        }
    }

    {
        let mut node = node_rc.borrow_mut();
        let left = node.left.as_ref().unwrap();
        node.left = remove_min(Rc::clone(left));
    }
    Some(fix_up(Rc::clone(&node_rc)))
}

fn traverse_in_order<K: Key, V: Value>(
    node: &Option<&RcNode<K, V>>,
    callback: &mut dyn FnMut(&Node<K, V>),
) {
    match node {
        Some(node_rc) => {
            let node = node_rc.borrow();
            traverse_in_order(&node.left.as_ref(), callback);
            callback(&node);
            traverse_in_order(&node.right.as_ref(), callback);
        }
        _ => (),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[derive(Clone)]
    struct TestKey {
        time: u8,
    }

    impl TestKey {
        pub fn new(time: u8) -> TestKey {
            TestKey { time }
        }
    }

    impl Key for TestKey {
        fn cmp(&self, other: &Self) -> Ordering {
            let other = other as &TestKey;
            if self.time < other.time {
                return Ordering::Less;
            } else if self.time > other.time {
                return Ordering::Greater;
            }
            Ordering::Equal
        }
    }

    #[derive(Clone)]
    struct TestValue {
        value: u8,
    }

    impl TestValue {
        pub fn new(value: u8) -> TestValue {
            TestValue { value }
        }
    }

    impl Value for TestValue {
        fn to_string(&self) -> String {
            self.value.to_string()
        }
    }

    fn create_key_value(key_time: u8, value: u8) -> (TestKey, TestValue) {
        (TestKey::new(key_time), TestValue::new(value))
    }

    #[test]
    fn keeping_order() {
        let cases = vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![8, 5, 7, 9, 1, 3, 6, 0, 4, 2],
            vec![7, 2, 0, 3, 1, 9, 8, 4, 6, 5],
            vec![2, 0, 3, 5, 8, 6, 4, 1, 9, 7],
            vec![8, 4, 7, 9, 2, 6, 0, 3, 1, 5],
            vec![7, 1, 5, 2, 8, 6, 3, 4, 0, 9],
            vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
        ];

        for case in cases {
            let mut tree: Tree<TestKey, TestValue> = Tree::new();
            for num in case {
                let (key, value) = create_key_value(num, num);
                tree.insert(key, value);
            }

            assert_eq!("0,1,2,3,4,5,6,7,8,9", tree.to_string());

            tree.remove(TestKey::new(8));
            assert_eq!("0,1,2,3,4,5,6,7,9", tree.to_string());

            tree.remove(TestKey::new(2));
            assert_eq!("0,1,3,4,5,6,7,9", tree.to_string());

            tree.remove(TestKey::new(5));
            assert_eq!("0,1,3,4,6,7,9", tree.to_string());
        }
    }

    #[test]
    fn floor() {
        let mut tree: Tree<TestKey, TestValue> = Tree::new();

        match tree.floor(TestKey::new(1)) {
            Some(_) => assert!(false),
            None => assert!(true),
        }

        //   2
        //  / |
        // 1   4
        let (key, value) = create_key_value(2, 2);
        tree.insert(key, value);
        let (key, value) = create_key_value(4, 4);
        tree.insert(key, value);
        let (key, value) = create_key_value(1, 1);
        tree.insert(key, value);

        // return equal key
        match tree.floor(TestKey::new(4)) {
            Some((_, value)) => assert_eq!("4", value.to_string()),
            _ => assert!(false),
        }
        // return greatest key
        match tree.floor(TestKey::new(5)) {
            Some((_, value)) => assert_eq!("4", value.to_string()),
            _ => assert!(false),
        }

        match tree.floor(TestKey::new(2)) {
            Some((_, value)) => assert_eq!("2", value.to_string()),
            _ => assert!(false),
        }
        match tree.floor(TestKey::new(1)) {
            Some((_, value)) => assert_eq!("1", value.to_string()),
            _ => assert!(false),
        }
        match tree.floor(TestKey::new(0)) {
            Some((_, value)) => assert_eq!("2", value.to_string()),
            _ => assert!(false),
        }
    }
}
