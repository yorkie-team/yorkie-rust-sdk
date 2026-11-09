//! JS `test/unit/document/text_undo_fuzz_test.ts` (`84068f70`), a port of
//! Go's `pkg/document/text_undo_fuzz_test.go` (yorkie#2310). It drives Text
//! undo and redo against concurrent deletions over the push/pull model of
//! `adv_sync`, every change through the wire projection. The default run
//! takes a few seeds per required setting; more seeds, and the setting that
//! is only reported, run with
//!
//! ```sh
//! TEXT_UNDO_FUZZ_SEEDS=1000 TEXT_UNDO_FUZZ_FULL=1 \
//!   cargo test -p yorkie-core --lib text_undo_fuzz -- --nocapture
//! ```
//!
//! It requires convergence with GC off, and with GC on when nothing is
//! inserted, and that every replica's removal ledger survives a snapshot.
//! With GC on and inserts, a recreated piece can still land after text
//! inserted between the gap and the following piece of its insertion (a
//! known limit JS and Go share), so that setting is only reported. The PRNG
//! is JS's `mulberry32`, so a seed here replays the JS seed's history.

use super::adv_sync::{new_adv_clients, AdvServer};
use crate::change::{ChangePack, Checkpoint};
use crate::crdt::element::CrdtElement;
use crate::crdt::text::CrdtText;
use crate::wire::WireChangePack;
use crate::{Document, Result};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// JS `seeds`: `TEXT_UNDO_FUZZ_SEEDS`, 10 by default.
fn seeds() -> u32 {
    std::env::var("TEXT_UNDO_FUZZ_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10)
}

/// JS `full`: whether `TEXT_UNDO_FUZZ_FULL` is set.
fn full() -> bool {
    std::env::var_os("TEXT_UNDO_FUZZ_FULL").is_some_and(|value| !value.is_empty())
}

/// JS `ledger`: the removal ledger of every node as a value, its
/// `removed_at`, the removals and the cancel pairs.
fn ledger(text: &CrdtText) -> String {
    let mut out = Vec::new();
    for node in text.nodes() {
        let (ledgered, dels, cancels) = node.ledger();
        let mut ds = Vec::new();
        let mut cs = Vec::new();
        if ledgered {
            ds = dels.iter().map(|d| d.to_test_string()).collect();
            cs = cancels
                .iter()
                .map(|c| {
                    format!(
                        "{}>{}",
                        c.deletion.to_test_string(),
                        c.cancelled_at.to_test_string()
                    )
                })
                .collect();
        } else if let Some(removed_at) = node.removed_at() {
            ds = vec![removed_at.to_test_string()];
        }
        ds.sort();
        cs.sort();
        let ra = node
            .removed_at()
            .map_or_else(|| "-".to_owned(), |t| t.to_test_string());
        out.push(format!(
            "{} r={ra} d={} c={}",
            node.id().to_test_string(),
            ds.join(","),
            cs.join(",")
        ));
    }
    out.join("|")
}

fn text_of(root: &crate::crdt::object::CrdtObject) -> Option<&CrdtText> {
    match root.get("t") {
        Some(CrdtElement::Text(text)) => Some(text),
        _ => None,
    }
}

/// JS `snapshotMismatch`: reports a ledger that does not survive a
/// snapshot, re-encoded through the wire root projection.
fn snapshot_mismatch(d: &Document) -> String {
    let pack = ChangePack::create_with_snapshot_root(
        d.key(),
        Checkpoint::new(1, 0),
        false,
        Vec::new(),
        None,
        None,
        Some(d.crdt_root.object().clone()),
    );
    let decoded = match WireChangePack::try_from(&pack).and_then(ChangePack::try_from) {
        Ok(decoded) => decoded,
        Err(err) => return format!("decode: {err}"),
    };
    let Some(got) = decoded.snapshot_root().and_then(text_of) else {
        return "decode: no text at t".to_owned();
    };
    let want = text_of(d.crdt_root.object())
        .map(ledger)
        .unwrap_or_default();
    let have = ledger(got);
    if want == have {
        String::new()
    } else {
        format!("ledger\n {want}\n {have}")
    }
}

fn text(d: &Document) -> String {
    match d.get_root().get("t") {
        Some(crate::JsonValue::Text(text)) => text.to_string(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

fn text_len(d: &Document) -> usize {
    match d.get_root().get("t") {
        Some(crate::JsonValue::Text(text)) => text.len(),
        other => panic!("expected a text at t, got {other:?}"),
    }
}

/// JS `runSeed`: runs one seed and returns an empty string on convergence,
/// or the failure.
fn run_seed(seed: u32, gc: bool, insert_pct: usize, clients: usize, steps: usize) -> String {
    let mut rnd = super::adv_sync::mulberry32(seed);
    let mut pick = move |n: usize| (rnd() * n as f64).floor() as usize;
    let mut s = AdvServer::default();
    let mut cs = new_adv_clients(clients);

    let setup = (|| -> Result<()> {
        cs[0].doc.update(|root| {
            root.set_text("t")?
                .edit(0, 0, "abcdefghij", None)
                .map(|_| ())
        })?;
        for c in cs.iter_mut() {
            s.sync(c, gc)?;
        }
        s.sync(&mut cs[0], gc)?;
        cs[0].doc.history.clear_undo();
        cs[0].doc.history.clear_redo();
        Ok(())
    })();
    if let Err(err) = setup {
        return format!("setup: {err}");
    }

    for step in 0..steps {
        let i = pick(cs.len());
        let c = &mut cs[i];
        let mut k = pick(10);
        if k < 2 && pick(100) >= insert_pct {
            k = 2;
        }
        let result = if k < 2 {
            let pos = pick(text_len(&c.doc) + 1);
            let ch = char::from(b'A' + pick(26) as u8).to_string();
            c.doc.update(|root| {
                root.get_text_mut("t")?
                    .edit(pos, pos, &ch, None)
                    .map(|_| ())
            })
        } else if k < 5 {
            let l = text_len(&c.doc);
            if l == 0 {
                continue;
            }
            let from = pick(l);
            let to = from + 1 + pick(3.min(l - from));
            c.doc
                .update(|root| root.get_text_mut("t")?.edit(from, to, "", None).map(|_| ()))
        } else if k < 7 {
            c.doc.history_mut().undo()
        } else if k < 8 {
            c.doc.history_mut().redo()
        } else {
            s.sync(c, gc)
        };
        if let Err(err) = result {
            return format!("step {step}: {err}");
        }
        for x in &cs {
            let why = snapshot_mismatch(&x.doc);
            if !why.is_empty() {
                return format!("step {step}: {why}");
            }
        }
    }

    for _ in 0..3 {
        for c in cs.iter_mut() {
            if let Err(err) = s.sync(c, gc) {
                return format!("final sync: {err}");
            }
        }
    }
    let want = text(&cs[0].doc);
    for c in &cs[1..] {
        let got = text(&c.doc);
        if got != want {
            return format!("diverged {want:?} vs {got:?}");
        }
    }
    String::new()
}

/// JS `it(name)` for one setting: runs every seed, prints the count (and,
/// on a full sweep, every failing seed) to stderr, and fails a required
/// setting on its first three failures. A panic in a seed is a failure of
/// that seed, so the seed is reported with it.
fn run_setting(gc: bool, insert_pct: usize, clients: usize, required: bool) {
    let full = full();
    if !required && !full {
        return;
    }
    let seeds = seeds();
    let name = format!("gc={gc} insert={insert_pct}% clients={clients}");
    let mut failures = Vec::new();
    for seed in 0..seeds {
        let why = catch_unwind(AssertUnwindSafe(|| {
            run_seed(seed, gc, insert_pct, clients, 40)
        }))
        .unwrap_or_else(|_| "panicked".to_owned());
        if !why.is_empty() {
            failures.push(format!("seed {seed}: {why}"));
        }
    }

    if full || !failures.is_empty() {
        eprintln!("{name}: {} of {seeds} seeds failed", failures.len());
    }
    if full {
        for failure in &failures {
            eprintln!("  {}", failure.lines().next().unwrap_or_default());
        }
    }
    if required {
        let first: Vec<_> = failures.iter().take(3).collect();
        assert!(first.is_empty(), "{name}: {first:#?}");
    }
}

#[test]
fn text_undo_fuzz_gc_off_no_insert_3_clients() {
    run_setting(false, 0, 3, true);
}

#[test]
fn text_undo_fuzz_gc_off_insert_3_clients() {
    run_setting(false, 100, 3, true);
}

#[test]
fn text_undo_fuzz_gc_off_insert_4_clients() {
    run_setting(false, 100, 4, true);
}

#[test]
fn text_undo_fuzz_gc_on_no_insert_3_clients() {
    run_setting(true, 0, 3, true);
}

/// Report-only: GC on with inserts hits the known recreate-anchor limit.
#[test]
fn text_undo_fuzz_gc_on_insert_3_clients() {
    run_setting(true, 100, 3, false);
}
