use std::cell::RefCell;
use std::rc::Rc;

type RcNode<V> = Rc<RefCell<Node<V>>>;
type OptionNode<V> = Option<RcNode<V>>;

pub trait Value: Clone {
    fn len(&self) -> usize;
    fn to_string(&self) -> String;
}

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

#[cfg(test)]
mod test {
    use super::*;
    
    #[derive(Clone)]
    struct TestValue {
        value: String,
    }

    impl TestValue {
        pub fn new(value: &str) -> Self {
            TestValue { value: value.to_string() }
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

    fn new_node() {
        let value = TestValue::new("hello");
        let node = Node::new(value);
    }
}