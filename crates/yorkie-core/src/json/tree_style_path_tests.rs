//! Tests for `JsonTree::style_by_path`, `style_by_path_range`,
//! `remove_style_by_path` and the small tree accessors.
//!
//! The bounds cases port `test/unit/document/tree_style_path_bounds_test.ts`
//! of JS at `f70e9f16` ("Reject styleByPath(path) for a path that names no
//! node"). Its negative path elements (the cases at lines 116 and 177, and
//! the whole "Tree path APIs with a negative path element" block at 209-280)
//! have no port: Rust paths are `&[usize]`, so a negative element cannot be
//! passed. The single-document cases of `test/integration/tree_test.ts` live
//! in `crates/yorkie/tests/integration_tree.rs`.

use crate::{
    ActorId, DocEvent, Document, ErrorCode, JsonTree, JsonValue, OperationInfo, Result, TimeTicket,
    TreeNodeInput, YorkieError,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

fn element_with(node_type: &str, attrs: Value, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element_with_attributes(node_type, attributes(attrs), children)
}

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

fn attributes(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).expect("attributes are an object")
}

fn bold() -> BTreeMap<String, Value> {
    attributes(json!({"bold": true}))
}

fn tree_of(doc: &Document) -> &JsonTree {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree,
        other => panic!("expected tree, got {other:?}"),
    }
}

fn xml(doc: &Document) -> String {
    tree_of(doc)
        .to_xml()
        .expect("a document tree is initialized")
}

fn tree_doc(root: TreeNodeInput) -> Result<Document> {
    let mut doc = Document::new("tree-style-path");
    doc.update(|root_object| root_object.set_tree("t", root).map(|_| ()))?;
    Ok(doc)
}

fn update_tree(doc: &mut Document, f: impl FnOnce(&mut JsonTree) -> Result<()>) -> Result<()> {
    doc.update(|root| f(root.get_tree_mut("t")?))
}

/// Asserts `result` failed with `ErrInvalidArgument` (JS
/// `assertInvalidArgument`).
fn assert_invalid_argument(result: Result<()>) {
    match result {
        Err(err) => assert_eq!(ErrorCode::ErrInvalidArgument, err.code(), "{err}"),
        Ok(()) => panic!("expected ErrInvalidArgument"),
    }
}

const INITIAL_XML: &str = "<root><p><t>ab</t></p><p></p></root>";

/// `<root><p><t>ab</t></p><p></p></root>` (JS `setup`).
fn setup() -> Result<Document> {
    let doc = tree_doc(element(
        "root",
        vec![
            element("p", vec![element("t", vec![text("ab")])]),
            element("p", vec![]),
        ],
    ))?;
    assert_eq!(INITIAL_XML, xml(&doc));
    Ok(doc)
}

/// JS bounds test "styles the node at an existing path".
#[test]
fn styles_the_node_at_an_existing_path() -> Result<()> {
    let mut doc = setup()?;
    update_tree(&mut doc, |tree| tree.style_by_path(&[0, 0], bold()))?;
    assert_eq!(
        r#"<root><p><t bold="true">ab</t></p><p></p></root>"#,
        xml(&doc)
    );
    Ok(())
}

/// JS bounds test "styles the element at a top-level path".
#[test]
fn styles_the_element_at_a_top_level_path() -> Result<()> {
    let mut doc = setup()?;
    update_tree(&mut doc, |tree| tree.style_by_path(&[1], bold()))?;
    assert_eq!(
        r#"<root><p><t>ab</t></p><p bold="true"></p></root>"#,
        xml(&doc)
    );
    Ok(())
}

/// JS bounds test "rejects the first child path of an empty element".
#[test]
fn rejects_the_first_child_path_of_an_empty_element() -> Result<()> {
    let mut doc = setup()?;
    let result = update_tree(&mut doc, |tree| tree.style_by_path(&[1, 0], bold()));
    assert_eq!(
        Err(YorkieError::InvalidArgument("unacceptable path".to_owned())),
        result
    );
    assert_eq!(INITIAL_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "rejects the path one past the last child".
#[test]
fn rejects_the_path_one_past_the_last_child() -> Result<()> {
    let mut doc = setup()?;
    let result = update_tree(&mut doc, |tree| tree.style_by_path(&[0, 1], bold()));
    assert_eq!(
        Err(YorkieError::InvalidArgument("unacceptable path".to_owned())),
        result
    );
    assert_eq!(INITIAL_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "rejects the top-level path one past the last element".
#[test]
fn rejects_the_top_level_path_one_past_the_last_element() -> Result<()> {
    let mut doc = setup()?;
    let result = update_tree(&mut doc, |tree| tree.style_by_path(&[2], bold()));
    assert_eq!(
        Err(YorkieError::InvalidArgument("unacceptable path".to_owned())),
        result
    );
    assert_eq!(INITIAL_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "keeps rejecting paths further out of range".
#[test]
fn keeps_rejecting_paths_further_out_of_range() -> Result<()> {
    let mut doc = setup()?;
    assert_invalid_argument(update_tree(&mut doc, |tree| {
        tree.style_by_path(&[1, 5], bold())
    }));
    assert_invalid_argument(update_tree(&mut doc, |tree| {
        tree.style_by_path(&[0, 3], bold())
    }));
    assert_eq!(INITIAL_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "leaves the range form alone": the end offset of a range
/// is an insertion point, so one past the last child is still accepted.
#[test]
fn leaves_the_range_form_alone() -> Result<()> {
    let mut doc = setup()?;
    update_tree(&mut doc, |tree| {
        tree.style_by_path_range(&[0], &[1], bold())
    })?;
    assert_eq!(
        r#"<root><p bold="true"><t bold="true">ab</t></p><p></p></root>"#,
        xml(&doc)
    );
    Ok(())
}

const INITIAL_TEXT_XML: &str = "<root><p>ab</p><p>cd</p></root>";

/// `<root><p>ab</p><p>cd</p></root>` (JS `setupText`).
fn setup_text() -> Result<Document> {
    let doc = tree_doc(element(
        "root",
        vec![
            element("p", vec![text("ab")]),
            element("p", vec![text("cd")]),
        ],
    ))?;
    assert_eq!(INITIAL_TEXT_XML, xml(&doc));
    Ok(doc)
}

/// JS bounds test "rejects the offset one past the last character".
#[test]
fn rejects_the_offset_one_past_the_last_character() -> Result<()> {
    let mut doc = setup_text()?;
    let result = update_tree(&mut doc, |tree| tree.style_by_path(&[0, 2], bold()));
    assert_eq!(
        Err(YorkieError::InvalidArgument("unacceptable path".to_owned())),
        result
    );
    assert_eq!(INITIAL_TEXT_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "rejects an offset further past the last character".
#[test]
fn rejects_an_offset_further_past_the_last_character() -> Result<()> {
    let mut doc = setup_text()?;
    assert_invalid_argument(update_tree(&mut doc, |tree| {
        tree.style_by_path(&[0, 3], bold())
    }));
    assert_eq!(INITIAL_TEXT_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "accepts an offset that names a character". Text nodes
/// take no attributes, so the tree is unchanged.
#[test]
fn accepts_an_offset_that_names_a_character() -> Result<()> {
    let mut doc = setup_text()?;
    update_tree(&mut doc, |tree| tree.style_by_path(&[0, 0], bold()))?;
    update_tree(&mut doc, |tree| tree.style_by_path(&[0, 1], bold()))?;
    assert_eq!(INITIAL_TEXT_XML, xml(&doc));
    Ok(())
}

/// JS bounds test "still styles the elements themselves".
#[test]
fn still_styles_the_elements_themselves() -> Result<()> {
    let mut doc = setup_text()?;
    update_tree(&mut doc, |tree| tree.style_by_path(&[1], bold()))?;
    assert_eq!(r#"<root><p>ab</p><p bold="true">cd</p></root>"#, xml(&doc));
    Ok(())
}

/// The end of one text node still names the first character of the next one
/// (JS `pathToPosRange`: past the end only once there is no next sibling).
#[test]
fn accepts_the_end_of_a_text_node_followed_by_another() -> Result<()> {
    let mut doc = setup_text()?;
    update_tree(&mut doc, |tree| tree.edit(3, 3, vec![text("x")], 0))?;
    assert_eq!("<root><p>abx</p><p>cd</p></root>", xml(&doc));

    // `ab` and `x` are two text nodes; offset 2 ends `ab` and names `x`.
    update_tree(&mut doc, |tree| tree.style_by_path(&[0, 2], bold()))?;
    assert_invalid_argument(update_tree(&mut doc, |tree| {
        tree.style_by_path(&[0, 3], bold())
    }));
    assert_eq!("<root><p>abx</p><p>cd</p></root>", xml(&doc));
    Ok(())
}

/// `<r><p>😀x</p></r>`: path `[0, 0]` is index 1, a whole-character
/// boundary, and the derived end, index 2, falls between the emoji's halves.
fn surrogate_doc() -> Result<Document> {
    tree_doc(element("r", vec![element("p", vec![text("😀x")])]))
}

/// JS `surrogate_index_test.ts` "styles a single path whose derived end
/// lands inside a pair": the single form derives its end as one past the
/// start, so an end between the halves of a pair must not refuse a request
/// that names a whole character.
///
/// JS also asserts `tree.findPos(2)` throws; Rust `find_pos` has no surrogate
/// check for trees yet, so that line has no port. The JS check that the emoji
/// survives is `keeps_the_pair_when_a_derived_end_lands_inside_it` below.
#[test]
fn styles_a_single_path_whose_derived_end_lands_inside_a_pair() -> Result<()> {
    let mut doc = surrogate_doc()?;
    update_tree(&mut doc, |tree| {
        assert_eq!(1, tree.path_to_index(&[0, 0])?);
        tree.style_by_path(&[0, 0], attributes(json!({"bold": "true"})))
    })?;
    Ok(())
}

/// The last assertion of the JS case above. The style splits the text node
/// at the derived end. JS strings keep each half, so the XML still shows the
/// emoji; Rust strings cannot hold a lone half, and the split decodes each
/// half as U+FFFD, as Go does (see "Rust strings cannot represent invalid
/// standalone UTF-16 surrogate halves" in `docs/current-porting-gaps.md`).
#[test]
#[ignore = "a Rust tree split inside a surrogate pair is lossy"]
fn keeps_the_pair_when_a_derived_end_lands_inside_it() -> Result<()> {
    let mut doc = surrogate_doc()?;
    update_tree(&mut doc, |tree| {
        tree.style_by_path(&[0, 0], attributes(json!({"bold": "true"})))
    })?;
    assert!(xml(&doc).contains('😀'), "{}", xml(&doc));
    Ok(())
}

/// The path forms check their arguments in JS order: lengths, then
/// emptiness, then order, each with JS's message.
#[test]
fn validates_paths_in_js_order() -> Result<()> {
    let mut doc = setup_text()?;
    let invalid = |message: &str| Err(YorkieError::InvalidArgument(message.to_owned()));

    for (from, to, message) in [
        (&[][..], &[0][..], "path length should be equal"),
        (&[], &[], "path should not be empty"),
        (&[1], &[0], "from should be less than or equal to to"),
        (&[1, 0], &[0, 2], "from should be less than or equal to to"),
    ] {
        assert_eq!(
            invalid(message),
            update_tree(&mut doc, |tree| tree.style_by_path_range(from, to, bold())),
            "style {from:?}..{to:?}"
        );
        assert_eq!(
            invalid(message),
            update_tree(&mut doc, |tree| tree.remove_style_by_path(
                from,
                to,
                ["bold"]
            )),
            "remove style {from:?}..{to:?}"
        );
    }
    assert_eq!(
        invalid("path should not be empty"),
        update_tree(&mut doc, |tree| tree.style_by_path(&[], bold()))
    );
    assert_eq!(INITIAL_TEXT_XML, xml(&doc));

    // Equal paths are an empty range, not an error.
    update_tree(&mut doc, |tree| {
        tree.style_by_path_range(&[1], &[1], bold())
    })?;
    update_tree(&mut doc, |tree| {
        tree.remove_style_by_path(&[1], &[1], ["bold"])
    })?;
    assert_eq!(INITIAL_TEXT_XML, xml(&doc));
    Ok(())
}

/// Collects the op infos of local changes under `$.t`.
fn collect_local_ops(doc: &mut Document) -> Arc<Mutex<Vec<OperationInfo>>> {
    let ops = Arc::new(Mutex::new(Vec::new()));
    let sink = ops.clone();
    doc.subscribe_path("$.t", move |event| {
        if let DocEvent::LocalChange(info) = event {
            sink.lock().unwrap().extend(info.operations.iter().cloned());
        }
    });
    ops
}

/// The path forms record the same operation as the index forms over the
/// matching indexes; only the position lookup differs.
#[test]
fn path_forms_record_the_same_operations_as_index_forms() -> Result<()> {
    let root = || {
        element(
            "root",
            vec![
                element_with("p", json!({"italic": true}), vec![text("ab")]),
                element_with("p", json!({"italic": true}), vec![text("cd")]),
            ],
        )
    };
    let run = |by_path: bool| -> Result<(Vec<OperationInfo>, String, String)> {
        let mut doc = tree_doc(root())?;
        let ops = collect_local_ops(&mut doc);
        let color = attributes(json!({"color": "red", "size": 12}));
        update_tree(&mut doc, |tree| {
            if by_path {
                tree.style_by_path(&[0], color.clone())?;
                tree.style_by_path_range(&[1], &[2], color.clone())?;
                tree.remove_style_by_path(&[0], &[2], ["italic"])
            } else {
                tree.style(0, 1, color.clone())?;
                tree.style(4, 8, color.clone())?;
                tree.remove_style(0, 8, ["italic"])
            }
        })?;
        let ops = ops.lock().unwrap().clone();
        Ok((ops, xml(&doc), doc.to_sorted_json()))
    };

    let (path_ops, path_xml, path_json) = run(true)?;
    let (index_ops, index_xml, index_json) = run(false)?;
    // The removal reports one op info per element it changes.
    assert_eq!(4, path_ops.len());
    assert_eq!(index_ops, path_ops);
    assert_eq!(index_xml, path_xml);
    assert_eq!(index_json, path_json);
    assert_eq!(
        r#"<root><p color="red" size="12">ab</p><p color="red" size="12">cd</p></root>"#,
        path_xml
    );
    Ok(())
}

/// Path styling needs `Document::update`, like the index forms.
#[test]
fn path_styles_need_a_document_update() -> Result<()> {
    let mut detached = JsonTree::new(element("root", vec![element("p", vec![])]));
    let not_initialized = || {
        Err(YorkieError::NotInitialized(
            "Tree is not initialized yet".to_owned(),
        ))
    };
    assert_eq!(not_initialized(), detached.style_by_path(&[0], bold()));
    assert_eq!(
        not_initialized(),
        detached.style_by_path_range(&[0], &[1], bold())
    );
    assert_eq!(
        not_initialized(),
        detached.remove_style_by_path(&[0], &[1], ["bold"])
    );

    let doc = setup()?;
    let mut read = tree_of(&doc).clone();
    assert_eq!(not_initialized(), read.style_by_path(&[0], bold()));
    Ok(())
}

/// JS `Tree.getID`, `getNodeSize` and `getRootTreeNode`; the values are
/// pinned to JS at `8629993e`. Removed nodes stay in the node map until
/// garbage collection, so deleting `cd` keeps the size.
#[test]
fn reports_id_node_size_and_root_tree_node() -> Result<()> {
    let detached = JsonTree::new(element("root", vec![]));
    let not_initialized = YorkieError::NotInitialized("Tree is not initialized yet".to_owned());
    assert_eq!(Err(not_initialized.clone()), detached.id().cloned());
    assert_eq!(Err(not_initialized.clone()), detached.node_size());
    assert_eq!(Err(not_initialized), detached.root_tree_node());

    let mut doc = tree_doc(element(
        "doc",
        vec![
            element_with("p", json!({"italic": "true"}), vec![text("ab")]),
            element("p", vec![text("cd")]),
        ],
    ))?;
    let tree = tree_of(&doc);
    assert_eq!(5, tree.node_size()?);
    // The tree is the first element the first change creates.
    let id = tree.id()?.clone();
    assert_eq!(TimeTicket::new(1, 1, ActorId::initial()), id);

    update_tree(&mut doc, |tree| tree.edit(3, 3, vec![text("x")], 0))?;
    assert_eq!(6, tree_of(&doc).node_size()?);
    update_tree(&mut doc, |tree| tree.edit(5, 9, vec![], 0))?;
    assert_eq!(r#"<doc><p italic="true">abx</p></doc>"#, xml(&doc));
    assert_eq!(6, tree_of(&doc).node_size()?);
    assert_eq!(
        element(
            "doc",
            vec![element_with(
                "p",
                json!({"italic": "true"}),
                vec![text("ab"), text("x")],
            )],
        ),
        tree_of(&doc).root_tree_node()?
    );
    assert_eq!(id, *tree_of(&doc).id()?);
    Ok(())
}
