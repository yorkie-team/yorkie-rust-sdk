use super::element::{CrdtElement, DataSize};
use crate::{Result, TimeTicket, YorkieError, TIME_TICKET_SIZE};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RgaTreeListNode {
    element: Option<CrdtElement>,
    position_created_at: TimeTicket,
    position_moved_at: Option<TimeTicket>,
    removed_at: Option<TimeTicket>,
}

impl RgaTreeListNode {
    fn with_element(element: CrdtElement) -> Self {
        Self {
            position_created_at: element.created_at().clone(),
            element: Some(element),
            position_moved_at: None,
            removed_at: None,
        }
    }

    fn bare_position(position_created_at: TimeTicket) -> Self {
        Self {
            element: None,
            position_created_at,
            position_moved_at: None,
            removed_at: None,
        }
    }

    pub(crate) fn element(&self) -> Option<&CrdtElement> {
        self.element.as_ref()
    }

    pub(crate) fn element_mut(&mut self) -> Option<&mut CrdtElement> {
        self.element.as_mut()
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        self.element
            .as_ref()
            .map(CrdtElement::created_at)
            .unwrap_or(&self.position_created_at)
    }

    pub(crate) fn positioned_at(&self) -> &TimeTicket {
        if let Some(position_moved_at) = &self.position_moved_at {
            return position_moved_at;
        }

        self.element
            .as_ref()
            .map(CrdtElement::created_at)
            .unwrap_or(&self.position_created_at)
    }

    pub(crate) fn position_created_at(&self) -> &TimeTicket {
        &self.position_created_at
    }

    pub(crate) fn position_moved_at(&self) -> Option<&TimeTicket> {
        self.position_moved_at.as_ref()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.removed_at.as_ref()
    }

    pub(crate) fn id_string(&self) -> String {
        self.position_created_at.to_id_string()
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: TimeTicket) {
        self.removed_at = Some(removed_at);
    }

    pub(crate) fn remove(&mut self, removed_at: TimeTicket) -> bool {
        self.element
            .as_mut()
            .is_some_and(|element| element.remove(Some(removed_at)))
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.element
            .as_ref()
            .map(CrdtElement::is_removed)
            .unwrap_or(true)
    }

    pub(crate) fn data_size(&self) -> DataSize {
        let mut meta = TIME_TICKET_SIZE;
        if self.removed_at.is_some() {
            meta += TIME_TICKET_SIZE;
        }

        DataSize { data: 0, meta }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RgaTreeList {
    dummy_head: RgaTreeListNode,
    nodes: Vec<RgaTreeListNode>,
}

impl RgaTreeList {
    pub(crate) fn new() -> Self {
        let mut dummy_head = RgaTreeListNode::with_element(CrdtElement::primitive(
            super::primitive::CrdtPrimitive::new(
                super::primitive::PrimitiveValue::Integer(0),
                TimeTicket::initial(),
            ),
        ));
        dummy_head
            .element_mut()
            .unwrap()
            .set_removed_at(Some(TimeTicket::initial()));

        Self {
            dummy_head,
            nodes: Vec::new(),
        }
    }

    pub(crate) fn create() -> Self {
        Self::new()
    }

    pub(crate) fn add(&mut self, element: CrdtElement) -> Result<()> {
        self.insert_after(&self.last_created_at(), element, None)
            .map(|_| ())
    }

    pub(crate) fn add_dead_position(
        &mut self,
        position_created_at: TimeTicket,
        removed_at: TimeTicket,
    ) {
        let mut node = RgaTreeListNode::bare_position(position_created_at);
        node.set_removed_at(removed_at);
        self.nodes.push(node);
    }

    pub(crate) fn add_moved_element(
        &mut self,
        element: CrdtElement,
        position_created_at: TimeTicket,
        position_moved_at: TimeTicket,
    ) {
        let mut node = RgaTreeListNode::bare_position(position_created_at);
        node.position_moved_at = Some(position_moved_at);
        node.element = Some(element);
        self.nodes.push(node);
    }

    pub(crate) fn insert_after(
        &mut self,
        prev_created_at: &TimeTicket,
        element: CrdtElement,
        executed_at: Option<TimeTicket>,
    ) -> Result<RgaTreeListNode> {
        let executed_at = executed_at.unwrap_or_else(|| element.created_at().clone());
        let start_position = self
            .position_index_by_position_created_at(prev_created_at)
            .or_else(|| self.position_index_by_element_created_at(prev_created_at))
            .ok_or_else(|| YorkieError::MissingCrdtElement(prev_created_at.to_id_string()))?;

        let prev_position = self.find_next_before_executed_at(start_position, &executed_at);
        let insert_index = node_index_after_position(prev_position);
        let node = RgaTreeListNode::with_element(element);
        self.nodes.insert(insert_index, node.clone());
        Ok(node)
    }

    pub(crate) fn move_after(
        &mut self,
        prev_created_at: &TimeTicket,
        created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<Option<RgaTreeListNode>> {
        if self
            .position_index_by_position_created_at(prev_created_at)
            .is_none()
        {
            return Err(YorkieError::MissingCrdtElement(
                prev_created_at.to_id_string(),
            ));
        }

        let target_index = self
            .node_index_by_element_created_at(created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?;

        if self.nodes[target_index]
            .position_moved_at
            .as_ref()
            .is_some_and(|current| !executed_at.after(current))
        {
            if self
                .position_index_by_position_created_at(&executed_at)
                .is_some()
            {
                return Ok(None);
            }

            let dead_position = self.insert_position_after(prev_created_at, executed_at.clone())?;
            let dead_node = &mut self.nodes[dead_position - 1];
            dead_node.set_removed_at(executed_at);
            return Ok(Some(dead_node.clone()));
        }

        let inserted_position = self.insert_position_after(prev_created_at, executed_at.clone())?;
        let target_index = self
            .node_index_by_element_created_at(created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?;

        let mut element = self.nodes[target_index]
            .element
            .take()
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?;
        element.set_moved_at(Some(executed_at.clone()));

        self.nodes[target_index].set_removed_at(executed_at.clone());
        let old_node = self.nodes[target_index].clone();

        let new_index = inserted_position - 1;
        self.nodes[new_index].position_moved_at = Some(executed_at);
        self.nodes[new_index].element = Some(element);

        Ok(Some(old_node))
    }

    pub(crate) fn get_by_id(&self, created_at: &TimeTicket) -> Option<&RgaTreeListNode> {
        self.nodes
            .iter()
            .find(|node| node.element_created_at() == Some(created_at))
            .or_else(|| {
                self.nodes
                    .iter()
                    .find(|node| node.position_created_at() == created_at)
            })
    }

    pub(crate) fn get_by_index(&self, index: usize) -> Option<&RgaTreeListNode> {
        self.nodes
            .iter()
            .filter(|node| node.element.is_some() && !node.is_removed())
            .nth(index)
    }

    pub(crate) fn get_by_index_mut(&mut self, index: usize) -> Option<&mut RgaTreeListNode> {
        self.nodes
            .iter_mut()
            .filter(|node| node.element.is_some() && !node.is_removed())
            .nth(index)
    }

    pub(crate) fn sub_path_of(&self, created_at: &TimeTicket) -> Option<String> {
        let target_index = self
            .node_index_by_element_created_at(created_at)
            .or_else(|| self.node_index_by_position_created_at(created_at))?;

        let visible_index = self.nodes[..target_index]
            .iter()
            .filter(|node| node.element.is_some() && !node.is_removed())
            .count();

        Some(visible_index.to_string())
    }

    pub(crate) fn purge(&mut self, element: &CrdtElement) -> Result<()> {
        let created_id = element.created_at().to_id_string();
        let index = self
            .node_index_by_element_created_at(element.created_at())
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_id))?;
        self.nodes.remove(index);
        Ok(())
    }

    pub(crate) fn purge_by_id(&mut self, child_id: &str) -> bool {
        let Some(index) = self.nodes.iter().position(|node| {
            node.element.is_none() && node.removed_at.is_some() && node.id_string() == child_id
        }) else {
            return false;
        };

        self.nodes.remove(index);
        true
    }

    pub(crate) fn delete(
        &mut self,
        created_at: &TimeTicket,
        removed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        let node = self
            .nodes
            .iter_mut()
            .find(|node| node.element_created_at() == Some(created_at))
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?;

        node.remove(removed_at);
        node.element
            .as_ref()
            .map(CrdtElement::deepcopy)
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))
    }

    pub(crate) fn delete_by_index(
        &mut self,
        index: usize,
        removed_at: TimeTicket,
    ) -> Result<Option<CrdtElement>> {
        let Some(node) = self.get_by_index_mut(index) else {
            return Ok(None);
        };

        node.remove(removed_at);
        Ok(node.element.as_ref().map(CrdtElement::deepcopy))
    }

    pub(crate) fn set(
        &mut self,
        created_at: &TimeTicket,
        element: CrdtElement,
        executed_at: TimeTicket,
    ) -> Result<CrdtElement> {
        if self.node_index_by_element_created_at(created_at).is_none() {
            return Err(YorkieError::MissingCrdtElement(created_at.to_id_string()));
        }

        self.insert_after(created_at, element, Some(executed_at.clone()))?;
        self.delete(created_at, executed_at)
    }

    pub(crate) fn find_prev_created_at(&self, created_at: &TimeTicket) -> Result<TimeTicket> {
        let mut index = self
            .node_index_by_element_created_at(created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(created_at.to_id_string()))?;

        while index > 0 {
            index -= 1;
            let node = &self.nodes[index];
            if node.element.is_some() && !node.is_removed() {
                return Ok(node.position_created_at().clone());
            }
        }

        Ok(self.dummy_head.position_created_at().clone())
    }

    pub(crate) fn pos_created_at(&self, element_created_at: &TimeTicket) -> Result<TimeTicket> {
        let node = self
            .nodes
            .iter()
            .find(|node| node.element_created_at() == Some(element_created_at))
            .ok_or_else(|| YorkieError::MissingCrdtElement(element_created_at.to_id_string()))?;

        Ok(node.position_created_at().clone())
    }

    pub(crate) fn last_created_at(&self) -> TimeTicket {
        self.nodes
            .last()
            .map(|node| node.position_created_at().clone())
            .unwrap_or_else(|| self.dummy_head.position_created_at().clone())
    }

    pub(crate) fn len(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.element.is_some() && !node.is_removed())
            .count()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &RgaTreeListNode> {
        self.nodes
            .iter()
            .filter(|node| node.element.is_some() && !node.is_removed())
    }

    pub(crate) fn iter_all(&self) -> impl Iterator<Item = &RgaTreeListNode> {
        self.nodes.iter()
    }

    pub(crate) fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut RgaTreeListNode> {
        self.nodes.iter_mut()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        Self {
            dummy_head: self.dummy_head.clone(),
            nodes: self.nodes.clone(),
        }
    }

    pub(crate) fn to_json(&self) -> String {
        let elements = self
            .iter()
            .filter_map(RgaTreeListNode::element)
            .map(CrdtElement::to_json)
            .collect::<Vec<_>>()
            .join(",");

        format!("[{elements}]")
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.to_json()
    }

    fn insert_position_after(
        &mut self,
        prev_created_at: &TimeTicket,
        executed_at: TimeTicket,
    ) -> Result<usize> {
        let start_position = self
            .position_index_by_position_created_at(prev_created_at)
            .ok_or_else(|| YorkieError::MissingCrdtElement(prev_created_at.to_id_string()))?;

        let prev_position = self.find_next_before_executed_at(start_position, &executed_at);
        let insert_index = node_index_after_position(prev_position);
        self.nodes
            .insert(insert_index, RgaTreeListNode::bare_position(executed_at));
        Ok(insert_index + 1)
    }

    fn find_next_before_executed_at(
        &self,
        mut position_index: usize,
        executed_at: &TimeTicket,
    ) -> usize {
        while let Some(next) = self.node_at_position_index(position_index + 1) {
            if !next.positioned_at().after(executed_at) {
                break;
            }

            position_index += 1;
        }

        position_index
    }

    fn position_index_by_position_created_at(&self, created_at: &TimeTicket) -> Option<usize> {
        if self.dummy_head.position_created_at() == created_at {
            return Some(0);
        }

        self.node_index_by_position_created_at(created_at)
            .map(|index| index + 1)
    }

    fn position_index_by_element_created_at(&self, created_at: &TimeTicket) -> Option<usize> {
        if self.dummy_head.element_created_at() == Some(created_at) {
            return Some(0);
        }

        self.node_index_by_element_created_at(created_at)
            .map(|index| index + 1)
    }

    fn node_index_by_position_created_at(&self, created_at: &TimeTicket) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.position_created_at() == created_at)
    }

    fn node_index_by_element_created_at(&self, created_at: &TimeTicket) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.element_created_at() == Some(created_at))
    }

    fn node_at_position_index(&self, position_index: usize) -> Option<&RgaTreeListNode> {
        if position_index == 0 {
            return Some(&self.dummy_head);
        }

        self.nodes.get(position_index - 1)
    }
}

impl RgaTreeListNode {
    fn element_created_at(&self) -> Option<&TimeTicket> {
        self.element.as_ref().map(CrdtElement::created_at)
    }
}

impl Default for RgaTreeList {
    fn default() -> Self {
        Self::new()
    }
}

fn node_index_after_position(position_index: usize) -> usize {
    position_index
}

#[cfg(test)]
mod tests {
    use super::RgaTreeList;
    use crate::crdt::element::CrdtElement;
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::TimeTicket;

    #[test]
    fn inserts_elements_after_positions() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        list.insert_after(&TimeTicket::initial(), primitive("one", t1.clone()), None)?;
        list.insert_after(&t1, primitive("two", t2.clone()), None)?;

        assert_eq!(2, list.len());
        assert_eq!(t2, list.last_created_at());
        assert_eq!(r#"["one","two"]"#, list.to_json());
        assert_eq!(Some("1".to_owned()), list.sub_path_of(&t2));
        Ok(())
    }

    #[test]
    fn orders_concurrent_inserts_by_position_time() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t1 = ticket(1, "b");
        let t2 = ticket(1, "a");

        list.insert_after(&TimeTicket::initial(), primitive("later", t1.clone()), None)?;
        list.insert_after(&TimeTicket::initial(), primitive("earlier", t2), None)?;

        assert_eq!(r#"["later","earlier"]"#, list.to_json());
        Ok(())
    }

    #[test]
    fn deletes_elements_by_created_time() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");

        list.add(primitive("one", t1.clone()))?;
        let removed = list.delete(&t1, t2.clone())?;

        assert_eq!("\"one\"", removed.to_json());
        assert_eq!("[]", list.to_json());
        assert_eq!(
            Some(&t2),
            list.get_by_id(&t1).unwrap().element().unwrap().removed_at()
        );
        Ok(())
    }

    #[test]
    fn moves_elements_with_position_nodes() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");

        list.add(primitive("one", t1.clone()))?;
        list.add(primitive("two", t2.clone()))?;
        let dead_node = list.move_after(&t2, &t1, t3.clone())?.unwrap();

        assert_eq!(r#"["two","one"]"#, list.to_json());
        assert!(dead_node.element().is_none());
        assert_eq!(Some(&t3), dead_node.removed_at());
        assert_eq!(t3, list.pos_created_at(&t1)?);
        assert_eq!(Some("1".to_owned()), list.sub_path_of(&t1));
        Ok(())
    }

    #[test]
    fn keeps_newer_move_when_late_move_arrives() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");
        let t4 = ticket(4, "a");

        list.add(primitive("one", t1.clone()))?;
        list.add(primitive("two", t2.clone()))?;
        list.move_after(&t2, &t1, t4.clone())?;
        list.move_after(&TimeTicket::initial(), &t1, t3.clone())?;

        assert_eq!(r#"["two","one"]"#, list.to_json());
        assert_eq!(t4, list.pos_created_at(&t1)?);
        assert!(list.get_by_id(&t3).is_some());
        Ok(())
    }

    #[test]
    fn purges_dead_position_nodes_by_id() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");

        list.add(primitive("one", t1.clone()))?;
        list.add(primitive("two", t2.clone()))?;
        let dead_node = list.move_after(&t2, &t1, t3)?.unwrap();
        let child_id = dead_node.id_string();

        assert!(list.get_by_id(&t1).unwrap().element().is_some());
        assert!(list.purge_by_id(&child_id));
        assert!(!list.purge_by_id(&child_id));
        assert_eq!(r#"["two","one"]"#, list.to_json());
        assert_eq!(Some("1".to_owned()), list.sub_path_of(&t1));
        Ok(())
    }

    #[test]
    fn losing_move_keeps_position_for_following_insert() -> crate::Result<()> {
        let mut list = RgaTreeList::new();
        let t_a = ticket(1, "a");
        let t_b = ticket(2, "a");
        let t_x = ticket(3, "a");
        let losing_move_at = ticket(4, "a");
        let winning_move_at = ticket(5, "a");
        let y_at = ticket(6, "a");

        list.add(primitive("A", t_a.clone()))?;
        list.add(primitive("B", t_b.clone()))?;
        list.add(primitive("X", t_x.clone()))?;

        list.move_after(&t_b, &t_x, winning_move_at)?;
        list.move_after(&t_a, &t_x, losing_move_at.clone())?;
        list.insert_after(&losing_move_at, primitive("Y", y_at), None)?;

        assert_eq!(r#"["A","Y","B","X"]"#, list.to_json());
        Ok(())
    }

    #[test]
    fn concurrent_moves_of_same_element_converge() -> crate::Result<()> {
        let t_a = ticket(1, "a");
        let t_b = ticket(2, "a");
        let t_c = ticket(3, "a");
        let move_after_b = ticket(4, "a");
        let move_after_c = ticket(5, "a");

        let mut first = list_with(&[("A", t_a.clone()), ("B", t_b.clone()), ("C", t_c.clone())])?;
        first.move_after(&t_b, &t_a, move_after_b.clone())?;
        first.move_after(&t_c, &t_a, move_after_c.clone())?;

        let mut second = list_with(&[("A", t_a.clone()), ("B", t_b.clone()), ("C", t_c.clone())])?;
        second.move_after(&t_c, &t_a, move_after_c)?;
        second.move_after(&t_b, &t_a, move_after_b)?;

        assert_eq!(first.to_json(), second.to_json());
        assert_eq!(r#"["B","C","A"]"#, first.to_json());
        Ok(())
    }

    #[test]
    fn moves_of_different_elements_converge() -> crate::Result<()> {
        let t_a = ticket(1, "a");
        let t_b = ticket(2, "a");
        let t_c = ticket(3, "a");
        let move_a_after_c = ticket(4, "a");
        let move_b_after_a = ticket(5, "a");

        let mut first = list_with(&[("A", t_a.clone()), ("B", t_b.clone()), ("C", t_c.clone())])?;
        first.move_after(&t_c, &t_a, move_a_after_c.clone())?;
        first.move_after(&t_a, &t_b, move_b_after_a.clone())?;

        let mut second = list_with(&[("A", t_a.clone()), ("B", t_b.clone()), ("C", t_c.clone())])?;
        second.move_after(&t_a, &t_b, move_b_after_a)?;
        second.move_after(&t_c, &t_a, move_a_after_c)?;

        assert_eq!(first.to_json(), second.to_json());
        Ok(())
    }

    #[test]
    fn chained_moves_converge_across_permutations() -> crate::Result<()> {
        let orders = [
            [1, 2, 3],
            [1, 3, 2],
            [2, 1, 3],
            [2, 3, 1],
            [3, 1, 2],
            [3, 2, 1],
        ];

        let expected = apply_chained_moves(&orders[0])?;
        for order in orders.iter().skip(1) {
            assert_eq!(expected, apply_chained_moves(order)?, "order: {order:?}");
        }

        Ok(())
    }

    #[test]
    fn independent_destination_moves_converge() -> crate::Result<()> {
        let t_a = ticket(1, "a");
        let t_b = ticket(2, "a");
        let t_c = ticket(3, "a");
        let t_d = ticket(4, "a");
        let move_a_after_d = ticket(5, "a");
        let move_b_after_c = ticket(6, "a");

        let mut first = list_with(&[
            ("A", t_a.clone()),
            ("B", t_b.clone()),
            ("C", t_c.clone()),
            ("D", t_d.clone()),
        ])?;
        first.move_after(&t_d, &t_a, move_a_after_d.clone())?;
        first.move_after(&t_c, &t_b, move_b_after_c.clone())?;

        let mut second = list_with(&[
            ("A", t_a.clone()),
            ("B", t_b.clone()),
            ("C", t_c.clone()),
            ("D", t_d.clone()),
        ])?;
        second.move_after(&t_c, &t_b, move_b_after_c)?;
        second.move_after(&t_d, &t_a, move_a_after_d)?;

        assert_eq!(first.to_json(), second.to_json());
        assert_eq!(r#"["C","B","D","A"]"#, first.to_json());
        Ok(())
    }

    #[test]
    fn insert_after_moved_element_uses_original_position_identity() -> crate::Result<()> {
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");
        let t4 = ticket(4, "a");
        let insert_at = ticket(5, "a");
        let move_at = ticket(6, "a");
        let new_at = ticket(7, "a");

        let mut first = list_with(&[
            ("1", t1.clone()),
            ("2", t2.clone()),
            ("3", t3.clone()),
            ("4", t4.clone()),
        ])?;
        first.insert_after(&t2, primitive("5", new_at.clone()), Some(insert_at.clone()))?;
        first.move_after(&t4, &t2, move_at.clone())?;

        let mut second = list_with(&[("1", t1), ("2", t2.clone()), ("3", t3), ("4", t4.clone())])?;
        second.move_after(&t4, &t2, move_at)?;
        second.insert_after(&t2, primitive("5", new_at), Some(insert_at))?;

        assert_eq!(first.to_json(), second.to_json());
        Ok(())
    }

    #[test]
    fn set_and_move_of_same_element_converge() -> crate::Result<()> {
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");
        let t4 = ticket(4, "a");
        let set_at = ticket(5, "a");
        let move_at = ticket(6, "a");

        let mut first = list_with(&[
            ("1", t1.clone()),
            ("2", t2.clone()),
            ("3", t3.clone()),
            ("4", t4.clone()),
        ])?;
        first.set(&t2, primitive("5", set_at.clone()), set_at.clone())?;
        first.move_after(&t4, &t2, move_at.clone())?;

        let mut second = list_with(&[("1", t1), ("2", t2.clone()), ("3", t3), ("4", t4.clone())])?;
        second.move_after(&t4, &t2, move_at)?;
        second.set(&t2, primitive("5", set_at.clone()), set_at)?;

        assert_eq!(first.to_json(), second.to_json());
        assert_eq!(r#"["1","5","3","4"]"#, first.to_json());
        Ok(())
    }

    #[test]
    fn remove_and_move_of_same_element_converge() -> crate::Result<()> {
        let t1 = ticket(1, "a");
        let t2 = ticket(2, "a");
        let t3 = ticket(3, "a");
        let t4 = ticket(4, "a");
        let remove_at = ticket(5, "a");
        let move_at = ticket(6, "a");

        let mut first = list_with(&[
            ("1", t1.clone()),
            ("2", t2.clone()),
            ("3", t3.clone()),
            ("4", t4.clone()),
        ])?;
        first.delete(&t2, remove_at.clone())?;
        first.move_after(&t4, &t2, move_at.clone())?;

        let mut second = list_with(&[("1", t1), ("2", t2.clone()), ("3", t3), ("4", t4.clone())])?;
        second.move_after(&t4, &t2, move_at)?;
        second.delete(&t2, remove_at)?;

        assert_eq!(first.to_json(), second.to_json());
        assert_eq!(r#"["1","3","4"]"#, first.to_json());
        Ok(())
    }

    #[test]
    fn concurrency_matrix_operations_converge() -> crate::Result<()> {
        let operations = [
            MatrixOperation::InsertPrev,
            MatrixOperation::InsertPrevNext,
            MatrixOperation::MovePrev,
            MatrixOperation::MovePrevNext,
            MatrixOperation::MoveTarget,
            MatrixOperation::SetTarget,
            MatrixOperation::RemoveTarget,
        ];

        for &first_op in &operations {
            for &second_op in &operations {
                let tickets = MatrixTickets::new();
                let mut first = matrix_list(&tickets)?;
                apply_matrix_operation(&mut first, first_op, 0, &tickets)?;
                apply_matrix_operation(&mut first, second_op, 1, &tickets)?;

                let mut second = matrix_list(&tickets)?;
                apply_matrix_operation(&mut second, second_op, 1, &tickets)?;
                apply_matrix_operation(&mut second, first_op, 0, &tickets)?;

                assert_eq!(
                    first.to_json(),
                    second.to_json(),
                    "{first_op:?} vs {second_op:?}"
                );
            }
        }

        Ok(())
    }

    #[derive(Debug, Clone, Copy)]
    enum MatrixOperation {
        InsertPrev,
        InsertPrevNext,
        MovePrev,
        MovePrevNext,
        MoveTarget,
        SetTarget,
        RemoveTarget,
    }

    struct MatrixTickets {
        one: TimeTicket,
        two: TimeTicket,
        three: TimeTicket,
        four: TimeTicket,
        operation_times: [TimeTicket; 2],
    }

    impl MatrixTickets {
        fn new() -> Self {
            Self {
                one: ticket(1, "a"),
                two: ticket(2, "a"),
                three: ticket(3, "a"),
                four: ticket(4, "a"),
                operation_times: [ticket(5, "a"), ticket(6, "a")],
            }
        }

        fn other_target(&self, client_id: usize) -> &TimeTicket {
            if client_id == 0 {
                &self.three
            } else {
                &self.four
            }
        }

        fn operation_time(&self, client_id: usize) -> TimeTicket {
            self.operation_times[client_id].clone()
        }
    }

    fn apply_chained_moves(order: &[usize; 3]) -> crate::Result<String> {
        let t_a = ticket(1, "a");
        let t_b = ticket(2, "a");
        let t_c = ticket(3, "a");
        let t_d = ticket(4, "a");
        let move_a_after_d = ticket(5, "a");
        let move_b_after_a = ticket(6, "a");
        let move_c_after_b = ticket(7, "a");
        let mut list = list_with(&[
            ("A", t_a.clone()),
            ("B", t_b.clone()),
            ("C", t_c.clone()),
            ("D", t_d.clone()),
        ])?;

        for operation in order {
            match operation {
                1 => list.move_after(&t_d, &t_a, move_a_after_d.clone())?,
                2 => list.move_after(&t_a, &t_b, move_b_after_a.clone())?,
                3 => list.move_after(&t_b, &t_c, move_c_after_b.clone())?,
                _ => unreachable!(),
            };
        }

        Ok(list.to_json())
    }

    fn matrix_list(tickets: &MatrixTickets) -> crate::Result<RgaTreeList> {
        list_with(&[
            ("1", tickets.one.clone()),
            ("2", tickets.two.clone()),
            ("3", tickets.three.clone()),
            ("4", tickets.four.clone()),
        ])
    }

    fn apply_matrix_operation(
        list: &mut RgaTreeList,
        operation: MatrixOperation,
        client_id: usize,
        tickets: &MatrixTickets,
    ) -> crate::Result<()> {
        let executed_at = tickets.operation_time(client_id);
        let new_value = if client_id == 0 { "5" } else { "6" };

        match operation {
            MatrixOperation::InsertPrev => list
                .insert_after(
                    &tickets.two,
                    primitive(new_value, executed_at.clone()),
                    Some(executed_at),
                )
                .map(|_| ()),
            MatrixOperation::InsertPrevNext => list
                .insert_after(
                    &tickets.one,
                    primitive(new_value, executed_at.clone()),
                    Some(executed_at),
                )
                .map(|_| ()),
            MatrixOperation::MovePrev => list
                .move_after(&tickets.two, tickets.other_target(client_id), executed_at)
                .map(|_| ()),
            MatrixOperation::MovePrevNext => list
                .move_after(&tickets.one, tickets.other_target(client_id), executed_at)
                .map(|_| ()),
            MatrixOperation::MoveTarget => list
                .move_after(tickets.other_target(client_id), &tickets.two, executed_at)
                .map(|_| ()),
            MatrixOperation::SetTarget => list
                .set(
                    &tickets.two,
                    primitive(new_value, executed_at.clone()),
                    executed_at,
                )
                .map(|_| ()),
            MatrixOperation::RemoveTarget => list.delete(&tickets.two, executed_at).map(|_| ()),
        }
    }

    fn list_with(values: &[(&str, TimeTicket)]) -> crate::Result<RgaTreeList> {
        let mut list = RgaTreeList::new();
        for (value, created_at) in values {
            list.add(primitive(value, created_at.clone()))?;
        }
        Ok(list)
    }

    fn primitive(value: &str, created_at: TimeTicket) -> CrdtElement {
        CrdtElement::primitive(CrdtPrimitive::new(
            PrimitiveValue::String(value.to_owned()),
            created_at,
        ))
    }

    fn ticket(lamport: i64, actor_id: &str) -> TimeTicket {
        TimeTicket::new(lamport, 0, actor_id)
    }
}
