//! The push/pull model of JS `test/helper/adv_sync.ts` (`84068f70`), itself
//! the model of Go's `pkg/document/adv_sync_test.go`, for the randomized
//! harnesses that drive replicas through a model server.
//!
//! `yorkie-core` cannot reach protobuf, so where JS passes a change or a
//! snapshot through protobuf, these helpers pass it through the `wire`
//! projection `yorkie-protocol` encodes from: `ChangePack -> WireChangePack
//! -> ChangePack`. The protobuf leg is covered by the converter tests.

use super::Document;
use crate::change::{Change, ChangePack, Checkpoint};
use crate::wire::WireChangePack;
use crate::{Result, VersionVector};
use std::collections::BTreeMap;

/// JS `mulberry32`: a small seeded PRNG, so a seed replays the same history.
/// It yields the same sequence as the JS function for the same seed.
pub(super) fn mulberry32(seed: u32) -> impl FnMut() -> f64 {
    let mut a = seed;
    move || {
        a = a.wrapping_add(0x6d2b_79f5);
        let mut t = a;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// JS `minVersionVector`, Go's `time.MinVersionVector`: element-wise min,
/// and 0 for an actor any vector does not carry.
pub(super) fn min_version_vector<'a>(
    vectors: impl IntoIterator<Item = &'a VersionVector> + Clone,
) -> VersionVector {
    let mut out = VersionVector::new();
    for vector in vectors.clone() {
        for (actor, _) in vector.iter() {
            let min = vectors
                .clone()
                .into_iter()
                .map(|v| v.get(actor.as_str()).unwrap_or(0))
                .min()
                .unwrap_or(0);
            out.set(actor.clone(), min);
        }
    }
    out
}

/// Passes `pack` through the wire projection, as a server would receive or
/// send it.
pub(super) fn through_wire(pack: &ChangePack) -> Result<ChangePack> {
    ChangePack::try_from(WireChangePack::try_from(pack)?)
}

/// A snapshot pack of `src`'s root, through the wire projection: what a
/// client that attaches late receives (JS `converter.snapshotToBytes` and
/// back). Its vector is `src`'s, and the checkpoint moves the server seq.
pub(super) fn snapshot_pack(key: &str, src: &Document) -> Result<ChangePack> {
    through_wire(&ChangePack::create_with_snapshot_root(
        key,
        Checkpoint::new(1, 0),
        false,
        Vec::new(),
        Some(src.version_vector().clone()),
        None,
        Some(src.crdt_root.object().clone()),
    ))
}

/// JS `AdvClient`: a replica attached to an `AdvServer`.
pub(super) struct AdvClient {
    pub(super) doc: Document,
    pub(super) id: String,
    pub(super) cursor: usize,
}

/// JS `newAdvClients`: `n` replicas of one document under the actors 1..n.
pub(super) fn new_adv_clients(n: usize) -> Vec<AdvClient> {
    (0..n)
        .map(|i| {
            let id = format!("{:024}", i + 1);
            let mut doc = Document::new("adv-doc");
            doc.set_actor(id.as_str());
            AdvClient { doc, id, cursor: 0 }
        })
        .collect()
}

/// JS `AdvServer`: the server's pushpull, with one ordered change log, one
/// stored version vector row per client (the vector its push carried), and
/// minVV = element-wise min over the stored rows. A client only ever sees
/// minVV as the version vector of a pull response, and `apply_change_pack`
/// is what runs GC with it, so GC can never run at a moment the protocol
/// would not produce.
#[derive(Default)]
pub(super) struct AdvServer {
    log: Vec<Change>,
    rows: BTreeMap<String, VersionVector>,
}

impl AdvServer {
    /// JS `sync`: one PushPull round trip. It pushes local changes, stores
    /// the request's vector, pulls everything the server holds that this
    /// client has not seen, and hands the response (changes + minVV) to
    /// `apply_change_pack`, which applies then collects.
    ///
    /// The push goes through the wire projection on its way into the log,
    /// so every change a client pulls has crossed it (JS passes the log
    /// through protobuf after each sync step).
    pub(super) fn sync(&mut self, c: &mut AdvClient, gc: bool) -> Result<()> {
        let pack = through_wire(&c.doc.create_change_pack())?;
        let pushed = pack.changes();
        self.log.extend(pushed.iter().cloned());
        self.rows.insert(
            c.id.clone(),
            pack.version_vector().cloned().unwrap_or_default(),
        );
        let last_seq = pushed.last().map_or(0, |change| change.id().client_seq());

        let pulled = self.log[c.cursor..]
            .iter()
            .filter(|change| change.id().actor_id().as_str() != c.id)
            .cloned()
            .collect();
        c.cursor = self.log.len();

        let vector = if gc {
            min_version_vector(self.rows.values())
        } else {
            VersionVector::new()
        };
        c.doc.apply_change_pack(&ChangePack::create(
            c.doc.key(),
            Checkpoint::new(0, last_seq),
            false,
            pulled,
            Some(vector),
            None,
        ))
    }
}

/// The sequence matches JS `mulberry32` for the same seeds (values scaled
/// by 2^32, taken from Node).
#[test]
fn mulberry32_matches_js() {
    for (seed, want) in [
        (0, [1_144_304_738_u32, 1_416_247, 958_946_056]),
        (7, [50_271_532, 266_108_690, 4_195_786_334]),
        (999, [4_165_713_787, 2_726_356_805, 1_328_570_424]),
    ] {
        let mut rnd = mulberry32(seed);
        let got = [0; 3].map(|_| (rnd() * 4_294_967_296.0) as u32);
        assert_eq!(want, got, "seed {seed}");
    }
}
