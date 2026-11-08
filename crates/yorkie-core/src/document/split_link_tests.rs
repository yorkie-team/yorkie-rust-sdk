//! Tests that a Set, Add or ArraySet payload holding a tree reaches the
//! document without split-sibling links, both on the replica that runs the
//! undo and on every replica that decodes it.
//!
//! JS references: `src/document/crdt/split_links.ts`
//! (`dropSplitLinksInElement`), `document.ts` `executeUndoRedo`,
//! `api/converter.ts` `fromElementSimple`, and
//! `test/unit/document/tree_split_link_payload_test.ts`. Go:
//! `pkg/document/document.go` (the undo) and `pkg/document/crdt/split_links.go`.
//! Changes are handed between replicas through the wire shape, as a client
//! would send them, instead of through a server.

use super::Document;
use crate::change::{Change, ChangePack, Checkpoint};
use crate::crdt::element::CrdtElement;
use crate::crdt::object::CrdtObject;
use crate::crdt::tree::{CrdtTree, TreeNode};
use crate::operation::{Operation, SetOperation};
use crate::wire::{WireChange, WireOperation};
use crate::{JsonObject, Result, TimeTicket, TreeNodeInput};

/// Lists every split-sibling link the tree carries, one line per node that
/// has one, removed nodes included. Empty when the tree carries none.
fn split_links(tree: &CrdtTree) -> Vec<String> {
    fn walk(node: &TreeNode, lines: &mut Vec<String>) {
        let prev = node.ins_prev_id().map(|id| id.to_id_string());
        let next = node.ins_next_id().map(|id| id.to_id_string());
        if prev.is_some() || next.is_some() {
            lines.push(format!("{} prev={prev:?} next={next:?}", node.id_string()));
        }
        for child in node.all_children() {
            walk(child, lines);
        }
    }
    let mut lines = Vec::new();
    walk(tree.root(), &mut lines);
    lines
}

/// Returns the tree at `path` in the document's CRDT root.
fn tree_at<'a>(doc: &'a Document, path: &[&str]) -> &'a CrdtTree {
    let (last, parents) = path.split_last().expect("path is not empty");
    let mut object = doc.crdt_root.object();
    for key in parents {
        match object.get(key) {
            Some(CrdtElement::Object(child)) => object = child,
            other => panic!("expected an object at {key}, got {other:?}"),
        }
    }
    match object.get(last) {
        Some(CrdtElement::Tree(tree)) => tree,
        other => panic!("expected a tree at {last}, got {other:?}"),
    }
}

fn xml_at(doc: &Document, path: &[&str]) -> String {
    tree_at(doc, path).to_xml()
}

fn replica(n: u8) -> Document {
    let mut doc = Document::new("doc-key");
    doc.set_actor(format!("{n:024}"));
    doc
}

/// Moves the changes `from` has not handed over yet into `to`, through the
/// wire shape.
fn deliver(from: &Document, sent: &mut usize, to: &mut Document) -> Result<()> {
    let changes = from.create_change_pack().changes()[*sent..]
        .iter()
        .map(|change| Change::try_from(WireChange::try_from(change)?))
        .collect::<Result<Vec<_>>>()?;
    *sent += changes.len();
    let pack = ChangePack::create("doc-key", Checkpoint::new(1, 0), false, changes, None, None);
    to.apply_change_pack(&pack)
}

/// A tree whose span and paragraph have both been split, so its nodes carry
/// split-sibling links: `<doc><p><span>abc</span><span>de</span></p></doc>`.
fn split_tree_input() -> TreeNodeInput {
    TreeNodeInput::element(
        "doc",
        vec![TreeNodeInput::element(
            "p",
            vec![TreeNodeInput::element(
                "span",
                vec![TreeNodeInput::text("abcde")],
            )],
        )],
    )
}

/// Returns a document holding the split tree at `t`.
fn with_split_tree() -> Result<Document> {
    let mut doc = replica(1);
    doc.update(|root| root.set_tree("t", split_tree_input()).map(|_| ()))?;
    doc.update(|root| {
        root.get_tree_mut("t")?
            .edit_by_path(&[0, 0, 3], &[0, 0, 3], Vec::new(), 1)
    })?;
    Ok(doc)
}

/// Hands every change of `doc` to a fresh replica through the wire shape.
fn replicate(doc: &Document) -> Result<Document> {
    let mut replica = replica(2);
    deliver(doc, &mut 0, &mut replica)?;
    Ok(replica)
}

#[test]
fn the_tree_a_split_leaves_behind_carries_links() -> Result<()> {
    let doc = with_split_tree()?;
    assert!(!split_links(tree_at(&doc, &["t"])).is_empty());
    Ok(())
}

/// JS `an undone Set restores the same links here and on a replica`. JS
/// drops the links on both sides, so neither carries any.
#[test]
fn an_undone_set_restores_the_tree_without_links_here_and_on_a_replica() -> Result<()> {
    let mut doc = with_split_tree()?;
    let restored = xml_at(&doc, &["t"]);
    doc.update(|root| {
        root.set_tree(
            "t",
            TreeNodeInput::element("doc", vec![TreeNodeInput::element("p", Vec::new())]),
        )
        .map(|_| ())
    })?;
    doc.history_mut().undo()?;

    let replica = replicate(&doc)?;
    assert_eq!(restored, xml_at(&doc, &["t"]));
    assert_eq!(restored, xml_at(&replica, &["t"]));
    assert_eq!(Vec::<String>::new(), split_links(tree_at(&doc, &["t"])));
    assert_eq!(Vec::<String>::new(), split_links(tree_at(&replica, &["t"])));
    Ok(())
}

/// JS `an undone Remove restores the same links here and on a replica`.
#[test]
fn an_undone_remove_restores_the_tree_without_links_here_and_on_a_replica() -> Result<()> {
    let mut doc = with_split_tree()?;
    let restored = xml_at(&doc, &["t"]);
    doc.update(|root| {
        root.remove("t");
        Ok(())
    })?;
    doc.history_mut().undo()?;

    let replica = replicate(&doc)?;
    assert_eq!(restored, xml_at(&doc, &["t"]));
    assert_eq!(restored, xml_at(&replica, &["t"]));
    assert_eq!(Vec::<String>::new(), split_links(tree_at(&doc, &["t"])));
    assert_eq!(Vec::<String>::new(), split_links(tree_at(&replica, &["t"])));
    Ok(())
}

/// The payload a replica decodes carries no links even when the sender kept
/// them, as JS `fromElementSimple` drops them on every Set/Add/ArraySet.
#[test]
fn a_decoded_set_payload_drops_links_from_a_nested_tree() -> Result<()> {
    let doc = with_split_tree()?;
    let tree = tree_at(&doc, &["t"]).deepcopy();
    assert!(!split_links(&tree).is_empty());

    let mut container = CrdtObject::create(TimeTicket::new(9, 0, "x"));
    container.set("t", CrdtElement::tree(tree), TimeTicket::new(9, 0, "x"));
    let op = Operation::Set(SetOperation::create(
        "c",
        CrdtElement::object(container),
        TimeTicket::initial(),
        Some(TimeTicket::new(10, 0, "x")),
    ));
    let decoded = Operation::try_from(WireOperation::try_from(&op)?)?;

    let Operation::Set(set) = decoded else {
        panic!("expected a set, got {decoded:?}");
    };
    let CrdtElement::Object(object) = set.value() else {
        panic!("expected an object");
    };
    let Some(CrdtElement::Tree(tree)) = object.get("t") else {
        panic!("expected a tree at t");
    };
    assert_eq!(Vec::<String>::new(), split_links(tree));
    Ok(())
}

/// The undo restores a container holding a split tree, then both replicas
/// split the same boundary concurrently. The replica that ran the undo and
/// the one that decoded it have to agree on the links, or the splits land
/// differently.
///
/// The expected tree is what the JS SDK (8629993e) produces for the same
/// steps, checked with a throwaway vitest run: with the links gone, the
/// position after `abc` resolves past `de`, the split sibling of the text
/// it was split from.
#[test]
fn converges_on_a_same_boundary_split_after_undo_restores_a_container() -> Result<()> {
    let mut d1 = replica(1);
    let mut d2 = replica(2);
    let (mut sent1, mut sent2) = (0, 0);

    d1.update(|root| {
        let mut container = JsonObject::new();
        container.set_tree("t", split_tree_input())?;
        root.set("c", container).map(|_| ())
    })?;
    d1.update(|root| {
        root.get_object_mut("c")?.get_tree_mut("t")?.edit_by_path(
            &[0, 0, 3],
            &[0, 0, 3],
            Vec::new(),
            1,
        )
    })?;
    deliver(&d1, &mut sent1, &mut d2)?;

    d1.update(|root| {
        root.remove("c");
        Ok(())
    })?;
    d1.history_mut().undo()?;
    deliver(&d1, &mut sent1, &mut d2)?;
    assert_eq!(xml_at(&d1, &["c", "t"]), xml_at(&d2, &["c", "t"]));
    assert_eq!(
        split_links(tree_at(&d1, &["c", "t"])),
        split_links(tree_at(&d2, &["c", "t"]))
    );

    for doc in [&mut d1, &mut d2] {
        doc.update(|root| {
            root.get_object_mut("c")?.get_tree_mut("t")?.edit_by_path(
                &[0, 0, 3],
                &[0, 0, 3],
                Vec::new(),
                1,
            )
        })?;
    }
    deliver(&d1, &mut sent1, &mut d2)?;
    deliver(&d2, &mut sent2, &mut d1)?;

    assert_eq!(
        "<doc><p><span>abc</span><span>de</span><span></span><span></span></p></doc>",
        xml_at(&d1, &["c", "t"])
    );
    assert_eq!(xml_at(&d1, &["c", "t"]), xml_at(&d2, &["c", "t"]));
    assert_eq!(d1.to_sorted_json(), d2.to_sorted_json());
    Ok(())
}
