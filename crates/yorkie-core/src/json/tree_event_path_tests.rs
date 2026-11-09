//! The paths Tree change events report in a parent that holds both text and
//! element children.
//!
//! JS builds an event's `fromPath`/`toPath` from tree positions with
//! `CRDTTree.toPath(parent, left)` (`src/document/crdt/tree.ts` at
//! `8629993e`: `styleChangeRange`, `edit`, `makeDeletionChanges`), not from
//! the index with `indexToPath`. In a parent that is not text-only the two
//! differ: right after an element, `toPath` counts children while
//! `indexToPath` resolves into the text that follows and counts sizes. No JS
//! test pins these values; they are pinned to a scratch run of JS at
//! `8629993e` that subscribed to the local and the remote replica.

use crate::change::{Change, ChangePack, Checkpoint};
use crate::wire::WireChange;
use crate::{
    DocEvent, Document, JsonTree, JsonValue, OperationInfo, Result, TreeNodeInput, VersionVector,
};
use std::sync::{Arc, Mutex};

const A1: &str = "000000000000000000000001";
const A2: &str = "000000000000000000000002";

fn element(node_type: &str, children: Vec<TreeNodeInput>) -> TreeNodeInput {
    TreeNodeInput::element(node_type, children)
}

fn text(value: &str) -> TreeNodeInput {
    TreeNodeInput::text(value)
}

fn x() -> Vec<TreeNodeInput> {
    vec![text("x")]
}

fn kv() -> std::collections::BTreeMap<String, serde_json::Value> {
    [("k".to_owned(), serde_json::Value::from("v"))].into()
}

/// `<doc><p>ab<b>c</b>d</p></doc>`.
fn m1() -> TreeNodeInput {
    element(
        "doc",
        vec![element(
            "p",
            vec![text("ab"), element("b", vec![text("c")]), text("d")],
        )],
    )
}

/// `<doc><p>a<b>c</b><i>e</i>d</p></doc>`.
fn m2() -> TreeNodeInput {
    element(
        "doc",
        vec![element(
            "p",
            vec![
                text("a"),
                element("b", vec![text("c")]),
                element("i", vec![text("e")]),
                text("d"),
            ],
        )],
    )
}

/// `(kind, from, to, from_path, to_path)` of a Tree operation.
type PathInfo = (&'static str, usize, usize, Vec<usize>, Vec<usize>);

fn path_info(op: &OperationInfo) -> Option<PathInfo> {
    match op {
        OperationInfo::TreeEdit {
            from,
            to,
            from_path,
            to_path,
            ..
        } => Some(("edit", *from, *to, from_path.clone(), to_path.clone())),
        OperationInfo::TreeStyle {
            from,
            to,
            from_path,
            to_path,
            ..
        } => Some(("style", *from, *to, from_path.clone(), to_path.clone())),
        _ => None,
    }
}

fn collect(doc: &mut Document, remote: bool) -> Arc<Mutex<Vec<PathInfo>>> {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let out = sink.clone();
    doc.subscribe(move |event| {
        let info = match event {
            DocEvent::LocalChange(info) if !remote => info,
            DocEvent::RemoteChange(info) if remote => info,
            _ => return,
        };
        out.lock()
            .unwrap()
            .extend(info.operations.iter().filter_map(path_info));
    });
    sink
}

/// Takes `doc`'s pending changes through the wire shape and acks them.
fn push(doc: &mut Document) -> Result<Vec<Change>> {
    let pack = doc.create_change_pack();
    let changes = pack
        .changes()
        .iter()
        .map(|change| Change::try_from(WireChange::try_from(change)?))
        .collect::<Result<Vec<_>>>()?;
    let last_seq = pack.changes().last().map_or(0, |c| c.id().client_seq());
    doc.apply_change_pack(&ChangePack::create(
        doc.key(),
        Checkpoint::new(0, last_seq),
        false,
        Vec::new(),
        Some(VersionVector::new()),
        None,
    ))?;
    Ok(changes)
}

fn pull(doc: &mut Document, changes: Vec<Change>) -> Result<()> {
    doc.apply_change_pack(&ChangePack::create(
        doc.key(),
        Checkpoint::new(0, 0),
        false,
        changes,
        Some(VersionVector::new()),
        None,
    ))
}

fn xml(doc: &Document) -> String {
    match doc.get_root().get("t") {
        Some(JsonValue::Tree(tree)) => tree.to_xml().expect("a document tree"),
        other => panic!("expected tree, got {other:?}"),
    }
}

type TreeEdit = fn(&mut JsonTree) -> Result<()>;

/// Runs `edit` on a fresh `root` after `pre`, and checks the paths of the
/// change events on the editing replica and on a peer that applies the
/// change.
fn assert_paths(
    label: &str,
    root: TreeNodeInput,
    pre: Option<TreeEdit>,
    edit: TreeEdit,
    expected_xml: &str,
    expected: &[PathInfo],
) -> Result<()> {
    let mut d1 = Document::new("paths");
    d1.set_actor(A1);
    d1.update(|r| r.set_tree("t", root).map(|_| ()))?;
    if let Some(pre) = pre {
        d1.update(|r| pre(r.get_tree_mut("t")?))?;
    }
    let mut d2 = Document::new("paths");
    d2.set_actor(A2);
    pull(&mut d2, push(&mut d1)?)?;

    let local = collect(&mut d1, false);
    let remote = collect(&mut d2, true);
    d1.update(|r| edit(r.get_tree_mut("t")?))?;
    pull(&mut d2, push(&mut d1)?)?;

    assert_eq!(expected_xml, xml(&d1), "{label}");
    assert_eq!(expected_xml, xml(&d2), "{label}");
    assert_eq!(expected, &local.lock().unwrap()[..], "{label} local");
    assert_eq!(expected, &remote.lock().unwrap()[..], "{label} remote");
    Ok(())
}

fn edit(from: usize, to: usize, from_path: &[usize], to_path: &[usize]) -> PathInfo {
    ("edit", from, to, from_path.to_vec(), to_path.to_vec())
}

fn style(from: usize, to: usize, from_path: &[usize], to_path: &[usize]) -> PathInfo {
    ("style", from, to, from_path.to_vec(), to_path.to_vec())
}

#[test]
fn reports_edit_paths_from_tree_positions() -> Result<()> {
    let cases: [(&str, TreeNodeInput, TreeEdit, &str, PathInfo); 18] = [
        (
            "M1 ins6",
            m1(),
            |t| t.edit(6, 6, x(), 0),
            "<doc><p>ab<b>c</b>xd</p></doc>",
            edit(6, 6, &[0, 2], &[0, 2]),
        ),
        (
            "M1 ins3",
            m1(),
            |t| t.edit(3, 3, x(), 0),
            "<doc><p>abx<b>c</b>d</p></doc>",
            edit(3, 3, &[0, 2], &[0, 2]),
        ),
        (
            "M1 del67",
            m1(),
            |t| t.edit(6, 7, Vec::new(), 0),
            "<doc><p>ab<b>c</b></p></doc>",
            edit(6, 7, &[0, 2], &[0, 6]),
        ),
        (
            "M1 del36",
            m1(),
            |t| t.edit(3, 6, Vec::new(), 0),
            "<doc><p>abd</p></doc>",
            edit(3, 6, &[0, 2], &[0, 2]),
        ),
        (
            "M1 del16",
            m1(),
            |t| t.edit(1, 6, Vec::new(), 0),
            "<doc><p>d</p></doc>",
            edit(1, 6, &[0, 0], &[0, 2]),
        ),
        (
            "M1 del27",
            m1(),
            |t| t.edit(2, 7, Vec::new(), 0),
            "<doc><p>a</p></doc>",
            edit(2, 7, &[0, 1], &[0, 6]),
        ),
        (
            "M1 rep67",
            m1(),
            |t| t.edit(6, 7, x(), 0),
            "<doc><p>ab<b>c</b>x</p></doc>",
            edit(6, 7, &[0, 2], &[0, 6]),
        ),
        (
            "M1 split6",
            m1(),
            |t| t.edit(6, 6, Vec::new(), 1),
            "<doc><p>ab<b>c</b></p><p>d</p></doc>",
            edit(6, 6, &[0, 2], &[0, 2]),
        ),
        (
            "M1 split3",
            m1(),
            |t| t.edit(3, 3, Vec::new(), 1),
            "<doc><p>ab</p><p><b>c</b>d</p></doc>",
            edit(3, 3, &[0, 2], &[0, 2]),
        ),
        (
            "M1 byPath02",
            m1(),
            |t| t.edit_by_path(&[0, 2], &[0, 2], x(), 0),
            "<doc><p>ab<b>c</b>xd</p></doc>",
            edit(6, 6, &[0, 2], &[0, 2]),
        ),
        (
            "M2 ins5",
            m2(),
            |t| t.edit(5, 5, x(), 0),
            "<doc><p>a<b>c</b>x<i>e</i>d</p></doc>",
            edit(5, 5, &[0, 2], &[0, 2]),
        ),
        (
            "M2 ins8",
            m2(),
            |t| t.edit(8, 8, x(), 0),
            "<doc><p>a<b>c</b><i>e</i>xd</p></doc>",
            edit(8, 8, &[0, 3], &[0, 3]),
        ),
        (
            "M2 del89",
            m2(),
            |t| t.edit(8, 9, Vec::new(), 0),
            "<doc><p>a<b>c</b><i>e</i></p></doc>",
            edit(8, 9, &[0, 3], &[0, 8]),
        ),
        (
            "M2 del58",
            m2(),
            |t| t.edit(5, 8, Vec::new(), 0),
            "<doc><p>a<b>c</b>d</p></doc>",
            edit(5, 8, &[0, 2], &[0, 3]),
        ),
        (
            "M2 del28",
            m2(),
            |t| t.edit(2, 8, Vec::new(), 0),
            "<doc><p>ad</p></doc>",
            edit(2, 8, &[0, 1], &[0, 3]),
        ),
        (
            "M2 del29",
            m2(),
            |t| t.edit(2, 9, Vec::new(), 0),
            "<doc><p>a</p></doc>",
            edit(2, 9, &[0, 1], &[0, 8]),
        ),
        (
            "M2 split5",
            m2(),
            |t| t.edit(5, 5, Vec::new(), 1),
            "<doc><p>a<b>c</b></p><p><i>e</i>d</p></doc>",
            edit(5, 5, &[0, 2], &[0, 2]),
        ),
        (
            "M2 split8",
            m2(),
            |t| t.edit(8, 8, Vec::new(), 1),
            "<doc><p>a<b>c</b><i>e</i></p><p>d</p></doc>",
            edit(8, 8, &[0, 3], &[0, 3]),
        ),
    ];
    for (label, root, f, expected_xml, expected) in cases {
        assert_paths(label, root, None, f, expected_xml, &[expected])?;
    }
    Ok(())
}

#[test]
fn reports_style_paths_from_tree_positions() -> Result<()> {
    let cases: [(&str, TreeNodeInput, TreeEdit, &str, Vec<PathInfo>); 5] = [
        (
            "M1 style34",
            m1(),
            |t| t.style(3, 4, kv()),
            r#"<doc><p>ab<b k="v">c</b>d</p></doc>"#,
            vec![style(3, 4, &[0, 2], &[0, 1, 0])],
        ),
        (
            "M1 style01",
            m1(),
            |t| t.style(0, 1, kv()),
            r#"<doc><p k="v">ab<b>c</b>d</p></doc>"#,
            vec![style(0, 1, &[0], &[0, 0])],
        ),
        (
            "M1 style08",
            m1(),
            |t| t.style(0, 8, kv()),
            r#"<doc><p k="v">ab<b k="v">c</b>d</p></doc>"#,
            vec![style(0, 1, &[0], &[0, 0]), style(3, 4, &[0, 2], &[0, 1, 0])],
        ),
        (
            "M2 style56",
            m2(),
            |t| t.style(5, 6, kv()),
            r#"<doc><p>a<b>c</b><i k="v">e</i>d</p></doc>"#,
            vec![style(5, 6, &[0, 2], &[0, 2, 0])],
        ),
        (
            "M2 style28",
            m2(),
            |t| t.style(2, 8, kv()),
            r#"<doc><p>a<b k="v">c</b><i k="v">e</i>d</p></doc>"#,
            vec![
                style(2, 3, &[0, 1], &[0, 1, 0]),
                style(5, 6, &[0, 2], &[0, 2, 0]),
            ],
        ),
    ];
    for (label, root, f, expected_xml, expected) in cases {
        assert_paths(label, root, None, f, expected_xml, &expected)?;
    }

    assert_paths(
        "M1 rmstyle34",
        m1(),
        Some(|t| t.style(3, 4, kv())),
        |t| t.remove_style(3, 4, ["k"]),
        "<doc><p>ab<b>c</b>d</p></doc>",
        &[style(3, 4, &[0, 2], &[0, 1, 0])],
    )
}

#[test]
fn reports_deletion_paths_around_runs() -> Result<()> {
    // `<doc><p>a<i></i>d</p></doc>`.
    let m3 = || {
        element(
            "doc",
            vec![element(
                "p",
                vec![text("a"), element("i", Vec::new()), text("d")],
            )],
        )
    };
    let cases: [(&str, TreeNodeInput, TreeEdit, &str, PathInfo); 4] = [
        (
            "M3 del24",
            m3(),
            |t| t.edit(2, 4, Vec::new(), 0),
            "<doc><p>ad</p></doc>",
            edit(2, 4, &[0, 1], &[0, 2]),
        ),
        (
            "M1 del45",
            m1(),
            |t| t.edit(4, 5, Vec::new(), 0),
            "<doc><p>ab<b></b>d</p></doc>",
            edit(4, 5, &[0, 1, 0], &[0, 1, 1]),
        ),
        (
            "M1 del08",
            m1(),
            |t| t.edit(0, 8, Vec::new(), 0),
            "<doc></doc>",
            edit(0, 8, &[0], &[1]),
        ),
        (
            "M1 del26",
            m1(),
            |t| t.edit(2, 6, Vec::new(), 0),
            "<doc><p>ad</p></doc>",
            edit(2, 6, &[0, 1], &[0, 3]),
        ),
    ];
    for (label, root, f, expected_xml, expected) in cases {
        assert_paths(label, root, None, f, expected_xml, &[expected])?;
    }
    Ok(())
}

/// Next to a removed node: a deletion still takes it as the left token (JS
/// `findLeftToken` walks `allChildren`) and `toPath` places it where it was;
/// an insertion and a style change skip it (JS `prevSibling` counts visible
/// siblings only).
#[test]
fn reports_paths_next_to_removed_nodes() -> Result<()> {
    assert_paths(
        "M1 tomb del34",
        m1(),
        Some(|t| t.edit(3, 6, Vec::new(), 0)),
        |t| t.edit(3, 4, Vec::new(), 0),
        "<doc><p>ab</p></doc>",
        &[edit(3, 4, &[0, 2], &[0, 3])],
    )?;
    assert_paths(
        "M1 tomb ins3",
        m1(),
        Some(|t| t.edit(3, 6, Vec::new(), 0)),
        |t| t.edit(3, 3, x(), 0),
        "<doc><p>abxd</p></doc>",
        &[edit(3, 3, &[0, 2], &[0, 2])],
    )?;
    assert_paths(
        "M2 tomb style",
        m2(),
        Some(|t| t.edit(2, 5, Vec::new(), 0)),
        |t| t.style(4, 5, kv()),
        r#"<doc><p>a<i k="v">e</i>d</p></doc>"#,
        &[style(2, 3, &[0, 1], &[0, 1, 0])],
    )
}
