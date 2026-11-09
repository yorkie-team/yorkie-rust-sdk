//! Tests for the Text and Tree position-range conversions.
//!
//! JS has no dedicated Text test: the Text cases check the round trip
//! through JS `Text.indexRangeToPosRange` (`CRDTText.indexRangeToPosRange`,
//! `RGATreeSplit.indexToPos`) and `Text.posRangeToIndexRange`
//! (`RGATreeSplit.findIndexesFromRange`, `posToIndex`). The Tree cases port
//! `test/unit/document/gc_split_leak_test.ts` and add the same round trips;
//! the single-document cases of `test/integration/tree_test.ts` live in
//! `crates/yorkie/tests/integration_tree.rs`.

use crate::change::{Change, ChangePack, Checkpoint};
use crate::crdt::root::CrdtRoot;
use crate::wire::WireChange;
use crate::{
    Document, ErrorCode, JsonText, JsonTree, JsonValue, Result, TextPosStructRange, TreeNodeInput,
    TreePosStructRange, VersionVector, YorkieError,
};

const A1: &str = "000000000000000000000001";
const A2: &str = "000000000000000000000002";

fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

fn text_of(doc: &Document) -> &JsonText {
    match doc.get_root().get("k") {
        Some(JsonValue::Text(text)) => text,
        other => panic!("expected text, got {other:?}"),
    }
}

fn tree_of(doc: &Document) -> JsonTree {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.clone(),
        other => panic!("expected tree, got {other:?}"),
    }
}

fn text_doc(edits: &[(usize, usize, &str)]) -> Result<Document> {
    let mut doc = Document::new("test-doc");
    doc.update(|root| {
        let text = root.set_text("k")?;
        for &(from, to, content) in edits {
            text.edit(from, to, content, None)?;
        }
        Ok(())
    })?;
    Ok(doc)
}

/// Checks `pos_range_to_index_range(index_range_to_pos_range(r)) == r` for
/// every range of `text`.
fn assert_text_round_trips(text: &JsonText) -> Result<()> {
    let len = text.len();
    for from in 0..=len {
        for to in from..=len {
            let range = text.index_range_to_pos_range((from, to))?;
            assert_eq!(
                (from, to),
                text.pos_range_to_index_range(&range)?,
                "range {from}..{to} of {text}"
            );
        }
    }
    Ok(())
}

#[test]
fn text_round_trips_at_both_ends() -> Result<()> {
    let doc = text_doc(&[(0, 0, "hello")])?;
    let text = text_of(&doc);

    for range in [(0, 0), (5, 5), (0, 5)] {
        let pos = text.index_range_to_pos_range(range)?;
        assert_eq!(range, text.pos_range_to_index_range(&pos)?);
    }
    assert_text_round_trips(text)
}

/// A `from` at the end of a split piece resolves to the right, to the start
/// of the next piece of the same insert, so it skips content inserted
/// between the pieces. JS does the same (`RGATreeSplit.posToIndex` with
/// `preferToLeft` false; checked against JS at `8629993e`), so the round
/// trip is not lossless here: `1..t` comes back as `4..t`, `2..t` as `3..t`.
#[test]
fn text_round_trips_inside_a_split_node() -> Result<()> {
    // `ABCD` with `BC` replaced by `12`, then `x` inside `12`: the nodes are
    // A | 1 | x | 2 | {BC} | D, with `1` and `2` split off one insert.
    let doc = text_doc(&[(0, 0, "ABCD"), (1, 3, "12"), (2, 2, "x")])?;
    let text = text_of(&doc);
    assert_eq!("A1x2D", text.to_string());

    // An index inside a node keeps the node and an offset into it.
    let (from, _) = text.index_range_to_pos_range((1, 1))?;
    assert_eq!(1, from.relative_offset);

    let len = text.len();
    for from in 0..=len {
        for to in from..=len {
            let range = text.index_range_to_pos_range((from, to))?;
            let expected_from = match from {
                1 => 4,
                2 => 3,
                from => from,
            };
            assert_eq!(
                (expected_from, to),
                text.pos_range_to_index_range(&range)?,
                "range {from}..{to}"
            );
        }
    }
    Ok(())
}

#[test]
fn text_round_trips_across_a_deleted_range() -> Result<()> {
    let mut doc = text_doc(&[(0, 0, "hello world")])?;
    let before: TextPosStructRange = text_of(&doc).index_range_to_pos_range((3, 6))?;

    doc.update(|root| root.get_text_mut("k")?.delete(2, 8).map(|_| ()))?;
    let text = text_of(&doc);
    assert_eq!("herld", text.to_string());
    assert_text_round_trips(text)?;

    // A range captured inside the deleted text collapses to where it was.
    assert_eq!((2, 2), text.pos_range_to_index_range(&before)?);
    Ok(())
}

#[test]
fn text_gives_the_same_position_for_equal_indexes() -> Result<()> {
    let doc = text_doc(&[(0, 0, "hello")])?;
    let (from, to) = text_of(&doc).index_range_to_pos_range((3, 3))?;
    assert_eq!(from, to);
    Ok(())
}

#[test]
fn text_position_is_plain_data() -> Result<()> {
    let mut doc = Document::new("test-doc");
    doc.set_actor(A1);
    doc.update(|root| root.set_text("k")?.edit(0, 0, "hello", None).map(|_| ()))?;
    let (_, to) = text_of(&doc).index_range_to_pos_range((0, 4))?;

    // The node of the insert, 4 code units in; the lamport is the decimal
    // string of the ticket.
    assert_eq!(A1, to.id.created_at.actor_id.as_str());
    assert!(to.id.created_at.lamport.parse::<i64>().is_ok());
    assert_eq!(0, to.id.offset);
    assert_eq!(4, to.relative_offset);
    Ok(())
}

#[test]
fn text_rejects_an_unknown_node_id() -> Result<()> {
    let doc = text_doc(&[(0, 0, "hello")])?;
    let text = text_of(&doc);
    let (mut from, to) = text.index_range_to_pos_range((1, 2))?;
    from.id.created_at.actor_id = A2.into();

    let err = text
        .pos_range_to_index_range(&(from, to))
        .expect_err("an unknown node ID");
    assert_eq!(ErrorCode::ErrInvalidArgument, err.code());
    Ok(())
}

#[test]
fn text_out_of_range_index_is_invalid_argument() -> Result<()> {
    let doc = text_doc(&[(0, 0, "hello")])?;
    let err = text_of(&doc)
        .index_range_to_pos_range((0, 6))
        .expect_err("past the end");
    assert_eq!(ErrorCode::ErrInvalidArgument, err.code());
    Ok(())
}

#[test]
fn text_not_in_a_document_is_not_initialized() -> Result<()> {
    let doc = text_doc(&[(0, 0, "hello")])?;
    let range = text_of(&doc).index_range_to_pos_range((0, 1))?;
    let detached = JsonText::new();
    let not_initialized = YorkieError::NotInitialized("Text is not initialized yet".to_owned());

    assert_eq!(
        Err(not_initialized.clone()),
        detached.index_range_to_pos_range((0, 0))
    );
    assert_eq!(
        Err(not_initialized),
        detached.pos_range_to_index_range(&range)
    );
    Ok(())
}

// --- Tree ---

fn tree_doc(root: TreeNodeInput) -> Result<Document> {
    let mut doc = Document::new("test-doc");
    doc.set_actor(A1);
    doc.update(|r| r.set_tree("t", root).map(|_| ()))?;
    Ok(doc)
}

/// Checks the index and path round trips for every range of `tree`.
fn assert_tree_round_trips(tree: &mut JsonTree) -> Result<()> {
    let size = tree.size()?;
    for from in 0..=size {
        for to in from..=size {
            let range = tree.index_range_to_pos_range((from, to))?;
            assert_eq!(
                (from, to),
                tree.pos_range_to_index_range(&range)?,
                "index range {from}..{to}"
            );

            let paths = (tree.index_to_path(from)?, tree.index_to_path(to)?);
            let range = tree.path_range_to_pos_range((&paths.0, &paths.1))?;
            assert_eq!(
                paths,
                tree.pos_range_to_path_range(&range)?,
                "path range {from}..{to}"
            );
        }
    }
    Ok(())
}

#[test]
fn tree_round_trips_every_range() -> Result<()> {
    let doc = tree_doc(element(
        "root",
        vec![element(
            "p",
            vec![element("b", vec![element("i", vec![text("ab")])])],
        )],
    ))?;
    assert_tree_round_trips(&mut tree_of(&doc))
}

#[test]
fn tree_round_trips_across_split_and_removed_nodes() -> Result<()> {
    let mut doc = tree_doc(element(
        "doc",
        vec![
            element("p", vec![text("hello")]),
            element("p", vec![text("world")]),
        ],
    ))?;
    doc.update(|root| {
        let tree = root.get_tree_mut("t")?;
        tree.edit(3, 3, vec![text("XY")], 0)?;
        tree.edit(4, 6, Vec::new(), 0)?;
        tree.edit(9, 11, Vec::new(), 0)
    })?;
    let mut tree = tree_of(&doc);
    assert_eq!("<doc><p>heXlo</p><p>wld</p></doc>", tree.to_xml()?);
    assert_tree_round_trips(&mut tree)
}

#[test]
fn tree_gives_the_same_position_for_equal_indexes() -> Result<()> {
    let doc = tree_doc(element("doc", vec![element("p", vec![text("ab")])]))?;
    let (from, to) = tree_of(&doc).index_range_to_pos_range((2, 2))?;
    assert_eq!(from, to);
    Ok(())
}

#[test]
fn tree_rejects_an_unknown_node_id() -> Result<()> {
    let doc = tree_doc(element("doc", vec![element("p", vec![text("ab")])]))?;
    let mut tree = tree_of(&doc);
    let (mut from, to) = tree.index_range_to_pos_range((1, 2))?;
    from.left_sibling_id.created_at.actor_id = A2.into();
    from.parent_id.created_at.actor_id = A2.into();
    let range: TreePosStructRange = (from, to);

    let err = tree
        .pos_range_to_index_range(&range)
        .expect_err("an unknown node ID");
    assert_eq!(ErrorCode::ErrInvalidArgument, err.code());
    let err = tree
        .pos_range_to_path_range(&range)
        .expect_err("an unknown node ID");
    assert_eq!(ErrorCode::ErrInvalidArgument, err.code());
    Ok(())
}

#[test]
fn tree_not_in_a_document_is_not_initialized() -> Result<()> {
    let doc = tree_doc(element("doc", vec![element("p", vec![text("ab")])]))?;
    let range = tree_of(&doc).index_range_to_pos_range((0, 1))?;
    let mut detached = JsonTree::new(element("doc", Vec::new()));
    let not_initialized = YorkieError::NotInitialized("Tree is not initialized yet".to_owned());

    assert_eq!(
        Err(not_initialized.clone()),
        detached.index_range_to_pos_range((0, 0))
    );
    assert_eq!(
        Err(not_initialized.clone()),
        detached.path_range_to_pos_range((&[0], &[0]))
    );
    assert_eq!(
        Err(not_initialized.clone()),
        detached.pos_range_to_index_range(&range)
    );
    assert_eq!(
        Err(not_initialized),
        detached.pos_range_to_path_range(&range)
    );
    Ok(())
}

// --- Read-path splits (JS `gc_split_leak_test.ts`) ---

/// Applies `changes` to `d`, as a pull from the server would.
fn pull(d: &mut Document, changes: Vec<Change>) -> Result<()> {
    let pack = ChangePack::create(
        d.key(),
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    );
    d.apply_change_pack(&pack)
}

/// Hands `d`'s pending changes to the server, through the wire shape, and
/// acks them at `d`.
fn push(d: &mut Document) -> Result<Vec<Change>> {
    let pack = d.create_change_pack();
    let changes = pack
        .changes()
        .iter()
        .map(|change| Change::try_from(WireChange::try_from(change)?))
        .collect::<Result<Vec<_>>>()?;
    let last_seq = pack.changes().last().map_or(0, |c| c.id().client_seq());
    d.apply_change_pack(&ChangePack::create(
        d.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))?;
    Ok(changes)
}

/// JS `crossSync`.
fn cross_sync(d1: &mut Document, d2: &mut Document) -> Result<()> {
    let c1 = push(d1)?;
    let c2 = push(d2)?;
    pull(d2, c1)?;
    pull(d1, c2)
}

fn max_vector_of(actors: &[&str]) -> VersionVector {
    let mut out = VersionVector::new();
    for actor in actors {
        out.set(*actor, i64::MAX);
    }
    out
}

/// A root rebuilt from `doc`'s, as a client loading a snapshot builds it.
fn rebuilt(doc: &Document) -> CrdtRoot {
    CrdtRoot::new(doc.crdt_root_for_test().object().deepcopy())
}

/// JS `registers GC pairs split during read-path range conversion` (both
/// conversions). JS splits the tombstoned text of the live clone root and
/// must register the born-removed pieces with it. A Rust `JsonTree` is a
/// copy, so the split never reaches the document: its garbage and size do
/// not move, and its garbage stays that of a rebuilt root, before and after
/// collection. The running size is compared with itself, not with the
/// rebuild: it disagrees with a rebuild for removed tree nodes with or
/// without a conversion (see the `doc_size` entry of the porting gaps).
fn assert_read_path_split_leaves_the_root(
    convert: fn(&mut JsonTree, &TreePosStructRange) -> Result<()>,
) -> Result<()> {
    let mut d1 = Document::new("test-doc");
    let mut d2 = Document::new("test-doc");
    d1.set_actor(A1);
    d2.set_actor(A2);

    d1.update(|root| {
        root.set_tree("t", element("doc", vec![element("p", vec![text("hello")])]))
            .map(|_| ())
    })?;
    cross_sync(&mut d1, &mut d2)?;

    // A selection into the middle of "hello", as a stored cursor would be.
    let selection = tree_of(&d1).index_range_to_pos_range((2, 4))?;

    // A peer deletes the whole <p>, tombstoning "hello" on d1.
    d2.update(|root| root.get_tree_mut("t")?.edit(0, 7, Vec::new(), 0))?;
    cross_sync(&mut d1, &mut d2)?;

    let garbage = d1.garbage_len();
    let size = d1.doc_size();
    assert!(garbage > 0);

    // Resolving the selection lands inside the tombstoned text and splits it
    // on the facade copy, inside an update and outside one.
    d1.update(|root| convert(root.get_tree_mut("t")?, &selection))?;
    convert(&mut tree_of(&d1), &selection)?;

    assert_eq!(garbage, d1.garbage_len());
    assert_eq!(size, d1.doc_size());
    let rebuilt_root = rebuilt(&d1);
    assert_eq!(rebuilt_root.get_garbage_len(), d1.garbage_len());

    d1.garbage_collect(&max_vector_of(&[A1, A2]));
    assert_eq!(0, d1.garbage_len());
    assert_eq!(0, rebuilt(&d1).get_garbage_len());
    Ok(())
}

#[test]
fn read_path_split_of_pos_range_to_index_range_leaves_the_root() -> Result<()> {
    assert_read_path_split_leaves_the_root(|tree, range| {
        assert_eq!((0, 0), tree.pos_range_to_index_range(range)?);
        Ok(())
    })
}

#[test]
fn read_path_split_of_pos_range_to_path_range_leaves_the_root() -> Result<()> {
    assert_read_path_split_leaves_the_root(|tree, range| {
        assert_eq!((vec![0], vec![0]), tree.pos_range_to_path_range(range)?);
        Ok(())
    })
}

/// `<doc><p>ab<b>c</b>d</p></doc>`: a parent with both text and element
/// children.
fn mixed_root() -> TreeNodeInput {
    element(
        "doc",
        vec![element(
            "p",
            vec![text("ab"), element("b", vec![text("c")]), text("d")],
        )],
    )
}

/// JS `posRangeToPathRange` goes through `toPath` (`treePosToPath` of
/// `toTreePos`). The values are pinned to JS at `8629993e`, converting each
/// index of the tree in turn inside one update, as JS did.
///
/// The conversion of index 2 splits `ab` in two, so the later paths count
/// one more child: index 4 is `[0, 2, 0]` here and `[0, 1, 0]` on a fresh
/// tree. Index 6, right after `</b>`, is an element position in a parent
/// that is not text-only, so JS counts children there (its `hasTextChild`
/// needs every child to be text) and gives `[0, 3]`.
#[test]
fn tree_converts_positions_in_a_mixed_parent_to_js_paths() -> Result<()> {
    let mut doc = tree_doc(mixed_root())?;
    let expected: [(usize, &[usize]); 9] = [
        (0, &[0]),
        (1, &[0, 0]),
        (2, &[0, 1]),
        (3, &[0, 2]),
        (4, &[0, 2, 0]),
        (5, &[0, 2, 1]),
        (6, &[0, 3]),
        (7, &[0, 6]),
        (8, &[1]),
    ];
    doc.update(|root| {
        let tree = root.get_tree_mut("t")?;
        for (index, path) in expected {
            let range = tree.index_range_to_pos_range((index, index))?;
            let (from, to) = tree.pos_range_to_path_range(&range)?;
            assert_eq!((path, path), (&from[..], &to[..]), "index {index}");
        }
        Ok(())
    })
}

/// The same conversions, each on a fresh tree, next to `index_to_path`
/// (pinned to JS at `8629993e`). Only index 6 differs between the two
/// routes: `index_to_path` resolves it into the text `d`, which counts
/// sizes, while the position anchors on `<b>`, an element position that
/// counts children.
#[test]
fn tree_converts_fresh_positions_in_a_mixed_parent_to_js_paths() -> Result<()> {
    let expected: [(usize, &[usize], &[usize]); 9] = [
        (0, &[0], &[0]),
        (1, &[0, 0], &[0, 0]),
        (2, &[0, 1], &[0, 1]),
        (3, &[0, 2], &[0, 2]),
        (4, &[0, 1, 0], &[0, 1, 0]),
        (5, &[0, 1, 1], &[0, 1, 1]),
        (6, &[0, 2], &[0, 5]),
        (7, &[0, 6], &[0, 6]),
        (8, &[1], &[1]),
    ];
    for (index, via_pos, via_index) in expected {
        let mut doc = tree_doc(mixed_root())?;
        doc.update(|root| {
            let tree = root.get_tree_mut("t")?;
            let range = tree.index_range_to_pos_range((index, index))?;
            let (from, to) = tree.pos_range_to_path_range(&range)?;
            assert_eq!((via_pos, via_pos), (&from[..], &to[..]), "index {index}");
            assert_eq!(via_index, &tree.index_to_path(index)?[..], "index {index}");
            Ok(())
        })?;
    }
    Ok(())
}

/// The path APIs in a parent that is not text-only, pinned to JS at
/// `8629993e`: the last path element counts children there, not sizes, and
/// `pathToPos` resolves the path through its index, so `[0, 1]`, the
/// position of `<b>`, lands at the end of `ab`.
#[test]
fn tree_path_apis_count_children_in_a_mixed_parent() -> Result<()> {
    let doc = tree_doc(mixed_root())?;
    let tree = tree_of(&doc);
    for (path, index) in [
        (&[0, 0][..], 1),
        (&[0, 1], 3),
        (&[0, 2], 6),
        (&[0, 3], 7),
        (&[0, 1, 0], 4),
        (&[0, 1, 1], 5),
    ] {
        assert_eq!(index, tree.path_to_index(path)?, "path {path:?}");
    }
    for path in [&[0, 4][..], &[0, 5], &[0, 1, 2]] {
        let err = tree.path_to_index(path).unwrap_err();
        assert_eq!(ErrorCode::ErrInvalidArgument, err.code(), "path {path:?}");
    }

    let edited = |f: &dyn Fn(&mut JsonTree) -> Result<()>| -> Result<String> {
        let mut doc = tree_doc(mixed_root())?;
        doc.update(|root| f(root.get_tree_mut("t")?))?;
        tree_of(&doc).to_xml()
    };
    let x = || vec![text("X")];
    for (path, xml) in [
        (&[0, 0][..], "<doc><p>Xab<b>c</b>d</p></doc>"),
        (&[0, 1], "<doc><p>abX<b>c</b>d</p></doc>"),
        (&[0, 2], "<doc><p>ab<b>c</b>Xd</p></doc>"),
        (&[0, 3], "<doc><p>ab<b>c</b>dX</p></doc>"),
    ] {
        let got = edited(&|tree| tree.edit_by_path(path, path, x(), 0))?;
        assert_eq!(xml, got, "insert at {path:?}");
    }
    for (from, to, xml) in [
        (&[0, 0][..], &[0, 1][..], "<doc><p><b>c</b>d</p></doc>"),
        (&[0, 1], &[0, 2], "<doc><p>abd</p></doc>"),
        (&[0, 2], &[0, 3], "<doc><p>ab<b>c</b></p></doc>"),
        (&[0, 0], &[0, 3], "<doc><p></p></doc>"),
    ] {
        let got = edited(&|tree| tree.edit_by_path(from, to, Vec::new(), 0))?;
        assert_eq!(xml, got, "delete {from:?}..{to:?}");
    }

    let kv = || [("k".to_owned(), serde_json::Value::from("v"))].into();
    // `[0, 0]` and `[0, 2]` name the texts `ab` and `d`, which take no
    // style; `[0, 1]` names `<b>`.
    for (path, xml) in [
        (&[0, 0][..], "<doc><p>ab<b>c</b>d</p></doc>"),
        (&[0, 1], r#"<doc><p>ab<b k="v">c</b>d</p></doc>"#),
        (&[0, 2], "<doc><p>ab<b>c</b>d</p></doc>"),
        (&[0, 1, 0], "<doc><p>ab<b>c</b>d</p></doc>"),
    ] {
        let got = edited(&|tree| tree.style_by_path(path, kv()))?;
        assert_eq!(xml, got, "style {path:?}");
    }
    let got = edited(&|tree| tree.style_by_path_range(&[0, 0], &[0, 2], kv()))?;
    assert_eq!(r#"<doc><p>ab<b k="v">c</b>d</p></doc>"#, got);

    for (path, xml) in [
        (&[0, 1][..], "<doc><p>ab</p><p><b>c</b>d</p></doc>"),
        (&[0, 2], "<doc><p>ab<b>c</b></p><p>d</p></doc>"),
    ] {
        let got = edited(&|tree| tree.split_by_path(path))?;
        assert_eq!(xml, got, "split {path:?}");
    }
    Ok(())
}

/// Positions taken in a parent that is then removed resolve to where the
/// parent was, as in JS at `8629993e`.
#[test]
fn tree_converts_positions_in_a_removed_mixed_parent_to_js_paths() -> Result<()> {
    let mut doc = tree_doc(mixed_root())?;
    let size = tree_of(&doc).size()?;
    let ranges = (0..=size)
        .map(|index| tree_of(&doc).index_range_to_pos_range((index, index)))
        .collect::<Result<Vec<_>>>()?;
    doc.update(|root| root.get_tree_mut("t")?.edit(0, 8, Vec::new(), 0))?;
    assert_eq!("<doc></doc>", tree_of(&doc).to_xml()?);

    let mut tree = tree_of(&doc);
    for (index, range) in ranges.iter().enumerate() {
        assert_eq!(
            (vec![0], vec![0]),
            tree.pos_range_to_path_range(range)?,
            "index {index}"
        );
    }
    Ok(())
}
