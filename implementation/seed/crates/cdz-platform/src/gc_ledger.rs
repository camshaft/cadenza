//! The GC ledger's bookkeeping — pins, reference edges, the liveness computation over them, and the
//! `collect` sweep (`design/DESIGN-cas-pinning-gc.md` §5/§6/§7, increments 2–3).
//!
//! The content-addressed store (§8) holds opaque bytes and must not parse them, so it cannot know the DAG's
//! shape. That knowledge lives HERE, in a ledger backed by an ordinary key-value store ([`KvStore`], §7) —
//! "storage is a reducer", so the GC's own state is just KV like any reducer's. This module is the ledger's
//! STATE, its liveness LOGIC, and the pure `collect` sweep ([`GcLedger::collect`]) that deletes the dead
//! blobs from a store. What is NOT here yet: wiring it as a reducer that folds `cas-pin`/`cas-unpin` host
//! calls and lifecycle events, and gathering the live root set / choosing WHEN to collect — those drive
//! behavior through the platform (§10 increments 4–6) and their coverage belongs in the conformance suite.
//!
//! **Two sets, keyed for the scans the liveness walk needs.**
//!  - **edges** — `edge:<parent><child>` keys. A put declares a blob's outbound refs ([`BlobStore::put`]);
//!    each `(parent, child)` edge is one key. Scanning the `edge:<parent>` prefix streams a blob's children
//!    for the reachability walk (the store's canonical key order makes the scan replay-deterministic, §7).
//!  - **pins** — `pin:<hash><session>` keys. A live session pins a hash to retain it against the collector;
//!    each `(hash, session)` pin is one key. Scanning the `pin:<hash>` prefix answers "who pins this hash",
//!    so `pinned(h)` and `|pins(h)|` are prefix scans.
//!
//! Both hashes and session ids are fixed-width ([`Hash::LEN`] bytes each — a session id is a [`ReducerId`],
//! itself a hash), so the composite keys need no delimiter between fields: a field is read at a fixed offset,
//! and a prefix of whole fixed-width fields (e.g. `edge:<parent>`) cannot straddle a field boundary. Pins and
//! edges are SETS, not counters — re-pinning by the same session, or re-putting identical bytes (§8's
//! idempotent-by-content put), simply re-writes the same key, so the effective count is DERIVED from the set,
//! never incremented in place (sidestepping the classic refcount double-count/underflow bug, §3).
//!
//! **Liveness.** A hash is `retained` iff it is pinned, named by a supplied root (the kernel's implicit
//! live-session roots and compaction-governed history roots — passed in, since they are not the ledger's own
//! state, §6), or reachable from a retained blob along a declared edge:
//! ```text
//! retained(h) ⟺ pinned(h) ∨ root(h) ∨ (∃ retained b : h ∈ refs(b))
//! ```
//! Content-addressing precludes cycles, so this least-fixed-point is a plain reachability walk from the root
//! set (roots ∪ pinned) over the edges — no tracer, and it always terminates. `collectable(h) ⟺ ¬retained(h)`
//! is what the increment-3 sweep will act on.

use crate::kv::prefix_range;
use crate::{Bytes, Hash, KvStore, ReducerId};
use futures_util::StreamExt;
use std::collections::HashSet;

/// The key prefix for a reference edge; followed by the parent hash then the child hash (each [`Hash::LEN`]).
const EDGE_PREFIX: &[u8] = b"edge:";
/// The key prefix for a pin; followed by the pinned hash then the pinning session id (each [`Hash::LEN`]).
const PIN_PREFIX: &[u8] = b"pin:";

/// The GC ledger: the pin and edge sets, backed by a [`KvStore`], plus the liveness computation over them
/// (`design/DESIGN-cas-pinning-gc.md` §6). Generic over the store so the same logic runs over the in-memory
/// backend in tests and over a session's real KV when this becomes a reducer (a later increment).
pub struct GcLedger<K: KvStore> {
    kv: K,
}

impl<K: KvStore> GcLedger<K> {
    /// A ledger over `kv` (empty when `kv` is empty).
    pub fn new(kv: K) -> Self {
        Self { kv }
    }

    /// Borrow the backing store (introspection; the reducer wiring owns its KV in a later increment).
    pub fn kv(&self) -> &K {
        &self.kv
    }

    // --- edges ---

    /// Record `parent`'s outbound reference edges — one `edge:<parent><child>` key per child. Set semantics:
    /// recording the same edge twice is a no-op re-write. A leaf blob (`refs` empty) records nothing.
    pub async fn record_edges(&mut self, parent: Hash, refs: &[Hash]) {
        for child in refs {
            self.kv.put(edge_key(parent, *child), Bytes::new()).await;
        }
    }

    /// The children of `parent` — the hashes it points at, read from the `edge:<parent>` prefix. Deduplicated
    /// (the keys are already a set) and in the store's canonical key order.
    pub async fn children(&self, parent: Hash) -> Vec<Hash> {
        let prefix = prefix_of(EDGE_PREFIX, parent);
        let mut keys = self.kv.scan_keys(prefix_range(prefix));
        let mut out = Vec::new();
        while let Some(key) = keys.next().await {
            // The child is the second fixed-width field, after the prefix and the parent.
            if let Some(child) = hash_at(&key, EDGE_PREFIX.len() + Hash::LEN) {
                out.push(child);
            }
        }
        out
    }

    // --- pins ---

    /// Pin `hash` for `session` — write the `pin:<hash><session>` key. Idempotent (set semantics): pinning a
    /// hash this session already pins re-writes the same key.
    pub async fn pin(&mut self, hash: Hash, session: ReducerId) {
        self.kv.put(pin_key(hash, session), Bytes::new()).await;
    }

    /// Release `session`'s pin on `hash`; `true` if a pin was present. Idempotent: unpinning a hash this
    /// session does not pin is a no-op returning `false`.
    pub async fn unpin(&mut self, hash: Hash, session: ReducerId) -> bool {
        self.kv.delete(&pin_key(hash, session)).await
    }

    /// The number of live sessions pinning `hash` — `|pins(h)|`, counted from the `pin:<hash>` prefix.
    pub async fn pin_count(&self, hash: Hash) -> usize {
        let prefix = prefix_of(PIN_PREFIX, hash);
        let mut keys = self.kv.scan_keys(prefix_range(prefix));
        let mut n = 0usize;
        while keys.next().await.is_some() {
            n += 1;
        }
        n
    }

    /// Whether any session pins `hash` — `pinned(h)`.
    pub async fn is_pinned(&self, hash: Hash) -> bool {
        let prefix = prefix_of(PIN_PREFIX, hash);
        self.kv
            .scan_keys(prefix_range(prefix))
            .next()
            .await
            .is_some()
    }

    /// Every distinct hash pinned by some session — the pinned root contribution to liveness, read from the
    /// whole `pin:` prefix (the hash is the first fixed-width field after the prefix).
    pub async fn pinned_hashes(&self) -> HashSet<Hash> {
        let mut keys = self
            .kv
            .scan_keys(prefix_range(Bytes::from_static(PIN_PREFIX)));
        let mut out = HashSet::new();
        while let Some(key) = keys.next().await {
            if let Some(hash) = hash_at(&key, PIN_PREFIX.len()) {
                out.insert(hash);
            }
        }
        out
    }

    // --- liveness ---

    /// The set of `retained` hashes given the supplied `roots` (the kernel's implicit live-session roots and
    /// compaction-governed history roots — not the ledger's own state, so passed in, §6). A reachability walk
    /// from `roots ∪ pinned_hashes` over the declared edges; content-addressing precludes cycles, so it
    /// terminates. Every reachable hash is retained; a hash NOT in the returned set is collectable.
    pub async fn retained(&self, roots: &[Hash]) -> HashSet<Hash> {
        let mut retained: HashSet<Hash> = HashSet::new();
        // Seed the frontier with the roots and every pinned hash.
        let mut frontier: Vec<Hash> = roots.to_vec();
        frontier.extend(self.pinned_hashes().await);
        while let Some(h) = frontier.pop() {
            if !retained.insert(h) {
                continue; // already visited
            }
            for child in self.children(h).await {
                if !retained.contains(&child) {
                    frontier.push(child);
                }
            }
        }
        retained
    }

    /// Whether `hash` is retained under `roots` (see [`retained`](Self::retained)). Convenience for a single
    /// query; computes the full retained set (the collector wants the whole set anyway).
    pub async fn is_retained(&self, hash: Hash, roots: &[Hash]) -> bool {
        self.retained(roots).await.contains(&hash)
    }

    /// The reference count of `hash` under `roots` — `|pins(h)| + |roots naming h| + |{retained b : h ∈
    /// refs(b)}|` (§5). `count(h) == 0 ⟺ ¬retained(h)`, so a collector may equivalently sweep `count == 0`
    /// blobs; exposed because the design states liveness as this decomposition. Computes the retained set to
    /// find the referencing retained parents.
    pub async fn count(&self, hash: Hash, roots: &[Hash]) -> usize {
        let mut n = self.pin_count(hash).await;
        n += roots.iter().filter(|&&r| r == hash).count();
        for parent in self.retained(roots).await {
            if self.children(parent).await.contains(&hash) {
                n += 1;
            }
        }
        n
    }

    // --- collection (increment 3) ---

    /// Every hash the ledger knows about — the union of both endpoints of every edge and every pinned hash.
    /// This is the candidate universe a `collect` evaluates: a blob is either pinned, reachable from a root
    /// via an edge, or a root, and §9's `cas-put`-self-pins rule keeps a freshly-put blob pinned until it is
    /// rooted, so every live blob is edge- or pin-known here. Read from the `edge:` and `pin:` prefixes.
    pub async fn known_hashes(&self) -> HashSet<Hash> {
        let mut out: HashSet<Hash> = self.pinned_hashes().await;
        let mut keys = self
            .kv
            .scan_keys(prefix_range(Bytes::from_static(EDGE_PREFIX)));
        while let Some(key) = keys.next().await {
            // An edge key carries BOTH endpoints: the parent, then the child.
            if let Some(parent) = hash_at(&key, EDGE_PREFIX.len()) {
                out.insert(parent);
            }
            if let Some(child) = hash_at(&key, EDGE_PREFIX.len() + Hash::LEN) {
                out.insert(child);
            }
        }
        out
    }

    /// Remove `parent`'s outbound reference edges (the `edge:<parent>` prefix) — the ledger-side of collecting
    /// a blob, so the DAG shrinks with the store. (Its incoming edges come from other collected blobs, whose
    /// own outbound edges this removes when they are processed — a retained blob never points at a collected
    /// one, §5.)
    async fn remove_blob_edges(&mut self, parent: Hash) {
        let prefix = prefix_of(EDGE_PREFIX, parent);
        let keys: Vec<Bytes> = self
            .kv
            .scan_keys(prefix_range(prefix))
            .collect::<Vec<_>>()
            .await;
        for key in keys {
            self.kv.delete(&key).await;
        }
    }

    /// Collect the dead blobs: delete every known-but-not-`retained` hash from `store` and drop its edges,
    /// under the supplied `roots` (§6/§7/§8). A single global `retained` computation over the DAG already
    /// accounts for the cascade — a blob reachable ONLY through a collected parent is itself not retained, so
    /// it is collected in the same pass; a blob also reachable through a retained parent survives — so no
    /// iterative re-evaluation is needed and the DAG guarantees termination. Returns the set collected.
    /// Content-safe: a delete is re-`put`-able (same bytes → same hash), so premature collection would be a
    /// liveness bug, not a correctness one — but §9 keeps a fold from ever referencing a collectable hash.
    /// This is the pure sweep; WHEN it runs (a quiescent point between folds, batched, deliberately
    /// triggered — never an eager background sweep, §8) and how `roots` are gathered are later increments.
    pub async fn collect<S: crate::BlobStore>(
        &mut self,
        store: &mut S,
        roots: &[Hash],
    ) -> HashSet<Hash> {
        let retained = self.retained(roots).await;
        let collectable: Vec<Hash> = self
            .known_hashes()
            .await
            .into_iter()
            .filter(|h| !retained.contains(h))
            .collect();
        let mut collected = HashSet::new();
        for h in collectable {
            store.delete(h).await;
            self.remove_blob_edges(h).await;
            collected.insert(h);
        }
        collected
    }
}

// --- key layout (fixed-width fields, no delimiter needed — see the module docs) ---

/// The `edge:<parent><child>` key.
fn edge_key(parent: Hash, child: Hash) -> Bytes {
    let mut k = Vec::with_capacity(EDGE_PREFIX.len() + 2 * Hash::LEN);
    k.extend_from_slice(EDGE_PREFIX);
    k.extend_from_slice(parent.as_bytes());
    k.extend_from_slice(child.as_bytes());
    Bytes::from(k)
}

/// The `pin:<hash><session>` key.
fn pin_key(hash: Hash, session: ReducerId) -> Bytes {
    let mut k = Vec::with_capacity(PIN_PREFIX.len() + 2 * Hash::LEN);
    k.extend_from_slice(PIN_PREFIX);
    k.extend_from_slice(hash.as_bytes());
    k.extend_from_slice(session.hash().as_bytes());
    Bytes::from(k)
}

/// A scan prefix `<prefix><hash>` — every key for that hash under `prefix` (its children, or its pinners).
fn prefix_of(prefix: &[u8], hash: Hash) -> Bytes {
    let mut k = Vec::with_capacity(prefix.len() + Hash::LEN);
    k.extend_from_slice(prefix);
    k.extend_from_slice(hash.as_bytes());
    Bytes::from(k)
}

/// The [`Hash`] at fixed byte `offset` in `key`, or `None` if the key is too short (a malformed key is
/// skipped, never a panic — the scans only ever see keys this module writes, so this is defensive).
fn hash_at(key: &[u8], offset: usize) -> Option<Hash> {
    let end = offset.checked_add(Hash::LEN)?;
    let slice = key.get(offset..end)?;
    <[u8; Hash::LEN]>::try_from(slice)
        .ok()
        .map(Hash::from_bytes)
}

#[cfg(test)]
mod tests {
    use super::GcLedger;
    use crate::{BlobStore, Bytes, Hash, HashTag, InMemoryBlobStore, InMemoryKvStore, ReducerId};
    use std::collections::HashSet;

    fn h(tag: &[u8]) -> Hash {
        Hash::of(HashTag::Blob, tag)
    }
    fn sess(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }
    fn ledger() -> GcLedger<InMemoryKvStore> {
        GcLedger::new(InMemoryKvStore::new())
    }

    #[tokio::test]
    async fn edges_record_and_read_back_as_a_set() {
        let mut g = ledger();
        let (p, a, b) = (h(b"parent"), h(b"a"), h(b"b"));
        g.record_edges(p, &[a, b]).await;
        // Recorded once, read back (order is the store's canonical key order, so compare as a set).
        let kids: HashSet<Hash> = g.children(p).await.into_iter().collect();
        assert_eq!(kids, HashSet::from([a, b]));
        // Set semantics: re-recording an existing edge does not duplicate it.
        g.record_edges(p, &[a]).await;
        assert_eq!(g.children(p).await.len(), 2);
        // A parent with no recorded edges has no children.
        assert!(g.children(h(b"leaf")).await.is_empty());
    }

    #[tokio::test]
    async fn pins_are_a_set_over_hash_and_session() {
        let mut g = ledger();
        let x = h(b"x");
        let (s1, s2) = (sess(b"s1"), sess(b"s2"));
        assert!(!g.is_pinned(x).await);
        g.pin(x, s1).await;
        g.pin(x, s2).await;
        // Two distinct sessions pin x.
        assert!(g.is_pinned(x).await);
        assert_eq!(g.pin_count(x).await, 2);
        // Idempotent: re-pinning by the same session does not add a pin.
        g.pin(x, s1).await;
        assert_eq!(g.pin_count(x).await, 2);
        // Unpin reports presence and is idempotent.
        assert!(g.unpin(x, s1).await);
        assert!(!g.unpin(x, s1).await);
        assert_eq!(g.pin_count(x).await, 1);
        assert!(g.is_pinned(x).await);
        g.unpin(x, s2).await;
        assert!(!g.is_pinned(x).await);
        // pinned_hashes reflects the distinct pinned hashes.
        let y = h(b"y");
        g.pin(x, s1).await;
        g.pin(y, s1).await;
        g.pin(y, s2).await;
        assert_eq!(g.pinned_hashes().await, HashSet::from([x, y]));
    }

    #[tokio::test]
    async fn retained_is_reachability_from_roots_and_pins() {
        // root -> mid -> leaf; and an orphan chain o1 -> o2 with no root/pin.
        let mut g = ledger();
        let (root, mid, leaf) = (h(b"root"), h(b"mid"), h(b"leaf"));
        let (o1, o2) = (h(b"o1"), h(b"o2"));
        g.record_edges(root, &[mid]).await;
        g.record_edges(mid, &[leaf]).await;
        g.record_edges(o1, &[o2]).await;
        // With `root` as the sole root: root, mid, leaf are retained; the orphan chain is not.
        let r = g.retained(&[root]).await;
        assert_eq!(r, HashSet::from([root, mid, leaf]));
        assert!(g.is_retained(leaf, &[root]).await);
        assert!(!g.is_retained(o1, &[root]).await);
        // A PIN roots its subtree too: pinning o1 retains o1 and o2.
        g.pin(o1, sess(b"s")).await;
        let r2 = g.retained(&[root]).await;
        assert_eq!(r2, HashSet::from([root, mid, leaf, o1, o2]));
        // count(h) == 0 exactly when not retained; > 0 when retained.
        assert_eq!(g.count(h(b"absent"), &[root]).await, 0);
        assert!(g.count(leaf, &[root]).await > 0);
        assert!(g.count(o2, &[root]).await > 0); // reachable via the pinned o1
    }

    /// Liveness matches a brute-force reachability oracle on random DAGs (`design/DESIGN-cas-pinning-gc.md`
    /// §10 increment 2). A deterministic PRNG builds many random acyclic graphs (edges only i -> j with
    /// i < j, so no cycles), random pins, and random roots; the ledger's `retained` must equal a plain
    /// in-memory BFS from `roots ∪ pinned` over the same edges.
    #[tokio::test]
    async fn collect_sweeps_unretained_blobs_cascades_and_survives_the_retained() {
        // Store holds a chain a -> b -> c (rooted at a) and an unreferenced subtree d -> e. Ledger mirrors
        // the edges the puts declared (what cas-put will wire in a later increment).
        let mut store = InMemoryBlobStore::new();
        let mut g = ledger();
        let a = store.put(Bytes::from_static(b"a"), &[]).await;
        let b = store.put(Bytes::from_static(b"b"), &[]).await;
        let c = store.put(Bytes::from_static(b"c"), &[]).await;
        let d = store.put(Bytes::from_static(b"d"), &[]).await;
        let e = store.put(Bytes::from_static(b"e"), &[]).await;
        g.record_edges(a, &[b]).await;
        g.record_edges(b, &[c]).await;
        g.record_edges(d, &[e]).await;

        // Rooted at `a`: a,b,c retained; the whole d->e subtree is dead → collected (cascade in one pass).
        let collected = g.collect(&mut store, &[a]).await;
        assert_eq!(collected, HashSet::from([d, e]));
        for h in [a, b, c] {
            assert!(store.has(h).await, "retained blob survives");
        }
        for h in [d, e] {
            assert!(!store.has(h).await, "unreferenced blob collected");
        }
        // The collected blobs' edges are gone from the ledger too (the DAG shrank with the store).
        assert!(g.children(d).await.is_empty());
        // Content-safe: re-putting a collected blob restores it under the same hash.
        let d2 = store.put(Bytes::from_static(b"d"), &[]).await;
        assert_eq!(d2, d);
        assert!(store.has(d).await);
    }

    #[tokio::test]
    async fn collect_keeps_a_pinned_blob_and_its_subtree_but_not_an_unrooted_parent() {
        // p -> q, with nothing rooting p. Pinning q retains q (and anything q reaches), but NOT p (unrooted,
        // unpinned, unreferenced) — a retained child does not retain its parent.
        let mut store = InMemoryBlobStore::new();
        let mut g = ledger();
        let p = store.put(Bytes::from_static(b"p"), &[]).await;
        let q = store.put(Bytes::from_static(b"q"), &[]).await;
        g.record_edges(p, &[q]).await;
        g.pin(q, sess(b"s")).await;
        let collected = g.collect(&mut store, &[]).await;
        assert_eq!(collected, HashSet::from([p]));
        assert!(store.has(q).await, "pinned blob survives");
        assert!(
            !store.has(p).await,
            "unrooted parent of a pinned child is still collected"
        );
    }

    // The random-DAG construction is index-keyed: the node index is the natural key for the oracle's
    // adjacency lists and the pinned/root index sets, so range-over-index is the clearest form here.
    #[allow(clippy::needless_range_loop)]
    #[tokio::test]
    async fn retained_matches_a_brute_force_reachability_oracle() {
        // A tiny deterministic xorshift PRNG (no external rand dep; seeded so the test is reproducible).
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            fn below(&mut self, n: usize) -> usize {
                (self.next() % (n as u64)) as usize
            }
            fn chance(&mut self, one_in: u64) -> bool {
                self.next().is_multiple_of(one_in)
            }
        }

        for seed in 1u64..=40 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let n = 3 + rng.below(10); // 3..=12 nodes
            let nodes: Vec<Hash> = (0..n)
                .map(|i| h(format!("n{seed}-{i}").as_bytes()))
                .collect();

            let mut g = ledger();
            // Adjacency oracle: only forward edges i -> j with i < j (acyclic).
            let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
            for i in 0..n {
                let mut refs = Vec::new();
                for j in (i + 1)..n {
                    if rng.chance(3) {
                        adj[i].push(j);
                        refs.push(nodes[j]);
                    }
                }
                g.record_edges(nodes[i], &refs).await;
            }
            // Random pins and roots.
            let mut pinned_idx: Vec<usize> = Vec::new();
            for i in 0..n {
                if rng.chance(4) {
                    g.pin(nodes[i], sess(format!("s{}", rng.below(3)).as_bytes()))
                        .await;
                    pinned_idx.push(i);
                }
            }
            let mut root_idx: Vec<usize> = Vec::new();
            let mut roots: Vec<Hash> = Vec::new();
            for i in 0..n {
                if rng.chance(4) {
                    root_idx.push(i);
                    roots.push(nodes[i]);
                }
            }

            // Brute-force oracle: BFS from roots ∪ pinned over adj.
            let mut expect: HashSet<usize> = HashSet::new();
            let mut stack: Vec<usize> = root_idx.iter().chain(pinned_idx.iter()).copied().collect();
            while let Some(i) = stack.pop() {
                if expect.insert(i) {
                    stack.extend(adj[i].iter().copied());
                }
            }
            let expect_hashes: HashSet<Hash> = expect.iter().map(|&i| nodes[i]).collect();

            let got = g.retained(&roots).await;
            assert_eq!(
                got, expect_hashes,
                "seed {seed}: ledger retained set must match the brute-force reachability oracle"
            );
        }
    }
}
