//! Sidecar logic — the pure, transport-agnostic brain of the bridge's OUTBOUND (fleet → Slack) path.
//!
//! Per `DESIGN.md`, the bridge is a SIDE-CAR to the concierge: it WATCHES the concierge's inbox (it does
//! NOT consume it — the concierge still drains it in tmux, dual-channel) and mirrors each newly-arrived
//! `ask`/`backlog` to a Slack thread. It records `{ slack_thread_ts → asker }` so that when the operator
//! replies in that thread, the INBOUND path (a later slice) can route a fleet `answer` back to the asker.
//!
//! This module is the decision + bookkeeping layer, with no Slack SDK and no async — so it's fully
//! unit-tested. The transport bin feeds it drained messages + the posted `thread_ts` and persists the map.
//!
//! Two pure pieces:
//!   - [`ThreadMap`] — the `thread_ts ↔ mirrored ask` bookkeeping, (de)serialized to a gitignored
//!     `slack-threads.json` under the fleet dir.
//!   - [`select_to_mirror`] — given the concierge's currently-pending inbox messages and the set already
//!     mirrored, decide which to post to Slack (new `ask`/`backlog`, never re-posting one already seen).

use crate::inbox::{Drained, Message};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

/// The kinds the sidecar mirrors OUTBOUND to Slack. An `ask` needs a human decision (the whole point); a
/// `backlog` is worth surfacing so the operator sees what's queuing. Other kinds (notes between agents,
/// merge-requests, etc.) are fleet-internal chatter and are NOT mirrored — they'd drown the channel.
pub const MIRRORED_KINDS: &[&str] = &["ask", "backlog"];

pub fn is_mirrored_kind(kind: &str) -> bool {
    MIRRORED_KINDS.contains(&kind)
}

/// Upper bound on how many mirrored threads the map retains. The bridge is a LONG-LIVED daemon (the
/// fleet's reliability backbone — it runs for weeks), and `record` is called for every mirrored
/// `ask`/`backlog`, so without a bound `slack-threads.json` grows forever: an unbounded memory + disk
/// leak, and — because the outbound loop `load`s (parse) the WHOLE map every tick (~couple of seconds)
/// and `save`s (serialize) it after every successful mirror post — a per-tick cost that grows with
/// history. Capping keeps both O(1)-ish.
///
/// Pruning the OLDEST threads is safe: `by_thread` is keyed by the Slack `thread_ts` (a `<epoch>.<frac>`
/// string that sorts chronologically), so the first keys are the oldest, and any still-pending concierge
/// ask is by definition recent. The two degraded outcomes are both benign — an operator reply in a
/// pruned (ancient) thread falls through to a free-form `note` instead of an `answer`, and a pruned
/// `by_key` entry can at worst re-mirror an ask that is STILL sitting unprocessed in the concierge inbox
/// after this many newer asks arrived, which is a harmless idempotent duplicate (`DESIGN.md`: asks are
/// idempotent by ask-id, whichever channel answers first wins). 1000 is generous (weeks of asks) yet
/// keeps the file small (~150 KB) so the per-tick load (and the on-post save) stays cheap.
pub const MAX_THREADS: usize = 1000;

/// A stable identity for a concierge inbox message, so we mirror it exactly once even though the concierge
/// file lingers (we don't consume it). The inbox filename is unique per delivery (`<seq>-<pid>-<kind>`), so
/// we key on it. `select_to_mirror` compares against the set of keys already in the [`ThreadMap`].
pub fn mirror_key(entry: &Drained) -> String {
    entry
        .file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One mirrored ask: what we posted and who asked, so an operator's thread reply routes back. Persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirroredAsk {
    /// The asking agent — the recipient of the eventual `answer`.
    pub asker: String,
    /// The message kind we mirrored (`ask`/`backlog`) — for context on the inbound side.
    pub kind: String,
    /// The subject we posted (for logging / dedup sanity).
    pub subject: String,
}

/// The persisted `slack-threads.json`: a map from Slack `thread_ts` → the ask it mirrors, plus the reverse
/// index (mirror-key → thread_ts) so we never double-post the same concierge message. Stored sorted
/// (`BTreeMap`) for a stable, diff-friendly on-disk form.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadMap {
    /// `slack_thread_ts` → the mirrored ask. This is what the inbound path looks up on a thread reply.
    pub by_thread: BTreeMap<String, MirroredAsk>,
    /// `mirror_key` (concierge inbox filename) → `slack_thread_ts`. Guards against re-posting.
    pub by_key: BTreeMap<String, String>,
}

impl ThreadMap {
    /// Has this concierge message already been mirrored?
    pub fn already_mirrored(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    /// Record a freshly-posted mirror: the Slack `thread_ts`, the concierge message key, and the ask.
    /// Prunes the oldest threads past [`MAX_THREADS`] so a long-lived daemon's map can't grow unbounded.
    pub fn record(
        &mut self,
        thread_ts: impl Into<String>,
        key: impl Into<String>,
        ask: MirroredAsk,
    ) {
        let ts = thread_ts.into();
        self.by_key.insert(key.into(), ts.clone());
        self.by_thread.insert(ts, ask);
        self.prune();
    }

    /// Evict the oldest threads until at most [`MAX_THREADS`] remain. `by_thread` is keyed by the
    /// chronologically-sortable Slack `thread_ts`, so its first keys are the oldest; we drop those and the
    /// `by_key` entries that point at them, keeping the two indices consistent (a leftover `by_key` row
    /// pointing at an evicted thread would merely miss the dedup — a harmless re-mirror — but we keep them
    /// in lockstep so the map stays a faithful bijection). Cheap and a no-op under the cap.
    ///
    /// Collect the oldest-excess `thread_ts` into a `HashSet` once, then do a SINGLE `retain` pass over
    /// each index against expected-O(1) set membership — expected O(n + k) total, not O(k·n) (a full
    /// `by_key` scan per evicted thread). A `HashSet` (not a `BTreeSet`) so `contains` is expected/amortized
    /// O(1) (worst-case degrades), keeping the retain passes expected-linear rather than O(n·log k). Matters
    /// if a large map is loaded and pruned at once.
    fn prune(&mut self) {
        let len = self.by_thread.len();
        if len <= MAX_THREADS {
            return; // common case: nothing to evict.
        }
        // The first `len - MAX_THREADS` keys are the oldest thread_ts (BTreeMap iterates sorted).
        let evict: HashSet<String> = self
            .by_thread
            .keys()
            .take(len - MAX_THREADS)
            .cloned()
            .collect();
        self.by_thread.retain(|ts, _| !evict.contains(ts));
        self.by_key.retain(|_, ts| !evict.contains(ts));
    }

    /// Mark a concierge message as UN-MIRRORABLE — the outbound mirror gave up posting it to Slack after
    /// exhausting retries (a deterministically un-postable `ask`, the mirror-side analogue of the relay's
    /// dead-letter). Recording it stops [`select_to_mirror`] from re-selecting it every tick (which would
    /// otherwise let one poison ask head-of-line-block mirroring of every later ask). We reuse [`record`]
    /// with a NON-NUMERIC sentinel `thread_ts` (`unpostable.<key>`): a real operator thread reply always
    /// carries a numeric Slack `<epoch>.<frac>` ts, so it can never match this sentinel in
    /// [`asker_for_thread`] — the operator still sees the ask in the concierge's tmux (dual-channel) and the
    /// concierge can answer it there. Bounded by the same [`prune`](Self::prune) as any recorded thread.
    pub fn mark_unmirrorable(&mut self, key: impl Into<String>, ask: MirroredAsk) {
        let key = key.into();
        self.record(format!("unpostable.{key}"), key, ask);
    }

    /// The asker to route an `answer` to, given a Slack thread the operator replied in.
    pub fn asker_for_thread(&self, thread_ts: &str) -> Option<&MirroredAsk> {
        self.by_thread.get(thread_ts)
    }

    /// The path of the persisted map under `fleet_dir` (gitignored, alongside the inboxes).
    pub fn path(fleet_dir: &Path) -> PathBuf {
        fleet_dir.join("slack-threads.json")
    }

    /// Load the map from `slack-threads.json`, or a fresh empty map if absent/malformed (fail-soft — a
    /// corrupt map should not crash the bridge; worst case it re-posts, which the operator can ignore).
    pub fn load(fleet_dir: &Path) -> Self {
        let path = Self::path(fleet_dir);
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!("slack-bridge: ignoring malformed {}: {e}", path.display());
                ThreadMap::default()
            }),
            Err(_) => ThreadMap::default(),
        }
    }

    /// Persist the map atomically (temp + rename), so a crash mid-write never leaves a torn file.
    pub fn save(&self, fleet_dir: &Path) -> io::Result<()> {
        let path = Self::path(fleet_dir);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// A concierge inbox message the sidecar has decided to mirror to Slack: the key (to record after posting)
/// and the message itself (to render the Slack post).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToMirror {
    pub key: String,
    pub msg: Message,
}

/// Given the concierge's currently-pending inbox (`drained`, oldest-first) and the `map` of what's already
/// been mirrored, return the messages to post to Slack now — the new `ask`/`backlog` entries not yet in the
/// map, preserving order. Pure: it does NOT post or mutate the map (the caller posts, gets a `thread_ts`,
/// then calls [`ThreadMap::record`]) — so a post failure leaves the message un-recorded and it retries next
/// tick rather than being lost.
pub fn select_to_mirror(drained: &[Drained], map: &ThreadMap) -> Vec<ToMirror> {
    let mut out = Vec::new();
    for entry in drained {
        if !is_mirrored_kind(&entry.msg.kind) {
            continue;
        }
        let key = mirror_key(entry);
        if key.is_empty() || map.already_mirrored(&key) {
            continue;
        }
        out.push(ToMirror {
            key,
            msg: entry.msg.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp_dir(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "slack-sidecar-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Build a `Drained` with a synthetic inbox filename `key` and a message of `kind`.
    fn drained(key: &str, from: &str, kind: &str, subject: &str) -> Drained {
        Drained {
            file: PathBuf::from(format!("/tmp/inbox/concierge/{key}")),
            msg: Message::new(from, "concierge", kind, subject),
        }
    }

    #[test]
    fn mirrors_only_ask_and_backlog() {
        let map = ThreadMap::default();
        let pending = vec![
            drained("000000000001-1-ask.json", "v-x", "ask", "decide A"),
            drained("000000000002-1-note.json", "v-y", "note", "fyi"),
            drained("000000000003-1-backlog.json", "v-z", "backlog", "idea"),
            drained(
                "000000000004-1-merge-request.json",
                "v-w",
                "merge-request",
                "land me",
            ),
        ];
        let sel = select_to_mirror(&pending, &map);
        let kinds: Vec<&str> = sel.iter().map(|m| m.msg.kind.as_str()).collect();
        assert_eq!(kinds, ["ask", "backlog"], "only ask + backlog, in order");
    }

    #[test]
    fn does_not_remirror_already_seen() {
        let mut map = ThreadMap::default();
        let a = drained("000000000001-1-ask.json", "v-x", "ask", "decide A");
        // First pass: it's new.
        let sel1 = select_to_mirror(std::slice::from_ref(&a), &map);
        assert_eq!(sel1.len(), 1);
        // Record it as posted (as the transport would after a successful Slack post).
        map.record(
            "1700000000.0001",
            sel1[0].key.clone(),
            MirroredAsk {
                asker: a.msg.from.clone(),
                kind: a.msg.kind.clone(),
                subject: a.msg.subject.clone(),
            },
        );
        // Second pass (concierge file still present — we don't consume it): NOT re-mirrored.
        let sel2 = select_to_mirror(std::slice::from_ref(&a), &map);
        assert!(
            sel2.is_empty(),
            "an already-mirrored ask is not posted again"
        );
    }

    #[test]
    fn mark_unmirrorable_stops_reselection_without_bogus_reply_routing() {
        // A deterministically un-postable ask must (a) stop being re-selected for mirroring every tick (else
        // one poison ask head-of-line-blocks mirroring of every later ask), and (b) NOT create a thread a
        // real operator reply could route to (the sentinel ts is non-numeric; real Slack ts is numeric).
        let mut map = ThreadMap::default();
        let a = drained("000000000009-1-ask.json", "v-poison", "ask", "un-postable");
        assert_eq!(
            select_to_mirror(std::slice::from_ref(&a), &map).len(),
            1,
            "new ask selected"
        );
        map.mark_unmirrorable(
            "000000000009-1-ask.json",
            MirroredAsk {
                asker: "v-poison".into(),
                kind: "ask".into(),
                subject: "un-postable".into(),
            },
        );
        assert!(
            select_to_mirror(std::slice::from_ref(&a), &map).is_empty(),
            "an un-mirrorable ask is not re-selected"
        );
        // A real numeric Slack thread reply never matches the sentinel ts.
        assert!(map.asker_for_thread("1700000000.0009").is_none());
        assert!(
            map.asker_for_thread("unpostable.000000000009-1-ask.json")
                .is_some(),
            "the sentinel itself is recorded (bounds via prune like any thread)"
        );
    }

    #[test]
    fn thread_map_routes_reply_to_asker() {
        let mut map = ThreadMap::default();
        map.record(
            "1700000000.9999",
            "000000000007-1-ask.json",
            MirroredAsk {
                asker: "v-inference".into(),
                kind: "ask".into(),
                subject: "which unifier?".into(),
            },
        );
        let ask = map
            .asker_for_thread("1700000000.9999")
            .expect("known thread");
        assert_eq!(ask.asker, "v-inference");
        assert!(map.asker_for_thread("nope").is_none());
    }

    #[test]
    fn thread_map_roundtrips_through_disk() {
        let dir = tmp_dir("persist");
        let mut map = ThreadMap::default();
        map.record(
            "1700000000.0001",
            "000000000001-1-ask.json",
            MirroredAsk {
                asker: "v-x".into(),
                kind: "ask".into(),
                subject: "s".into(),
            },
        );
        map.save(&dir).unwrap();
        let loaded = ThreadMap::load(&dir);
        assert_eq!(loaded, map, "map survives save→load");
        assert_eq!(
            loaded.asker_for_thread("1700000000.0001").unwrap().asker,
            "v-x"
        );
    }

    #[test]
    fn load_missing_map_is_empty() {
        let dir = tmp_dir("missing");
        assert_eq!(ThreadMap::load(&dir), ThreadMap::default());
    }

    #[test]
    fn load_malformed_map_is_empty_not_fatal() {
        let dir = tmp_dir("bad");
        std::fs::write(ThreadMap::path(&dir), "{not valid json").unwrap();
        assert_eq!(ThreadMap::load(&dir), ThreadMap::default());
    }

    #[test]
    fn record_bounds_the_map_to_max_threads() {
        // A long-lived daemon records one entry per mirrored ask forever; the map MUST stay bounded so
        // slack-threads.json can't leak memory/disk and the per-tick load (and on-post save) stays cheap.
        let mut map = ThreadMap::default();
        let over = MAX_THREADS + 250;
        for i in 0..over {
            // thread_ts as a chronologically-sortable "<epoch>.<frac>"; monotonically increasing i keeps
            // insertion == chronological order, so the oldest are the first-inserted.
            map.record(
                format!("17000{:06}.0001", i),
                format!("{:012}-1-ask.json", i),
                MirroredAsk {
                    asker: format!("v-{i}"),
                    kind: "ask".into(),
                    subject: "s".into(),
                },
            );
        }
        assert_eq!(map.by_thread.len(), MAX_THREADS, "by_thread capped");
        assert_eq!(map.by_key.len(), MAX_THREADS, "by_key stays in lockstep");
        // The OLDEST (first-inserted) were evicted; the NEWEST are retained.
        assert!(
            map.asker_for_thread("17000000000.0001").is_none(),
            "oldest thread pruned"
        );
        assert!(
            map.asker_for_thread(&format!("17000{:06}.0001", over - 1))
                .is_some(),
            "newest thread retained"
        );
    }

    #[test]
    fn prune_keeps_indices_consistent_and_survives_disk() {
        // After pruning, every by_key entry must still point at a live by_thread entry (a faithful
        // bijection), and the bounded map must round-trip through save/load unchanged.
        let dir = tmp_dir("prune-persist");
        let mut map = ThreadMap::default();
        for i in 0..(MAX_THREADS + 10) {
            map.record(
                format!("17000{:06}.0001", i),
                format!("{:012}-1-ask.json", i),
                MirroredAsk {
                    asker: format!("v-{i}"),
                    kind: "ask".into(),
                    subject: "s".into(),
                },
            );
        }
        for ts in map.by_key.values() {
            assert!(
                map.by_thread.contains_key(ts),
                "by_key points at a live thread {ts}"
            );
        }
        map.save(&dir).unwrap();
        let loaded = ThreadMap::load(&dir);
        assert_eq!(loaded, map, "bounded map round-trips through disk");
        assert_eq!(loaded.by_thread.len(), MAX_THREADS);
    }

    #[test]
    fn bulk_over_cap_prune_evicts_exactly_the_oldest_and_keeps_bijection() {
        // Behavioral contract of the single-retain-pass prune: a big over-cap insert must evict EXACTLY
        // the oldest excess threads, retain exactly the newest MAX_THREADS, and keep by_key a faithful
        // bijection with by_thread (no dangling rows). (A test can't assert big-O; this pins the result.)
        let mut map = ThreadMap::default();
        let excess = 500;
        let total = MAX_THREADS + excess;
        for i in 0..total {
            map.record(
                format!("17000{:06}.0001", i),
                format!("{:012}-1-ask.json", i),
                MirroredAsk {
                    asker: format!("v-{i}"),
                    kind: "ask".into(),
                    subject: "s".into(),
                },
            );
        }
        assert_eq!(map.by_thread.len(), MAX_THREADS);
        assert_eq!(map.by_key.len(), MAX_THREADS, "indices stay in lockstep");
        // Exactly the first `excess` thread_ts were evicted, and everything from `excess` on is retained.
        for i in 0..excess {
            assert!(
                map.asker_for_thread(&format!("17000{:06}.0001", i))
                    .is_none(),
                "oldest #{i} evicted"
            );
        }
        for i in excess..total {
            assert!(
                map.asker_for_thread(&format!("17000{:06}.0001", i))
                    .is_some(),
                "newest #{i} retained"
            );
        }
        // Every surviving by_key row still points at a live by_thread entry (bijection).
        for ts in map.by_key.values() {
            assert!(map.by_thread.contains_key(ts), "dangling by_key → {ts}");
        }
    }

    #[test]
    fn under_the_cap_nothing_is_pruned() {
        let mut map = ThreadMap::default();
        for i in 0..5 {
            map.record(
                format!("17000{:06}.0001", i),
                format!("{:012}-1-ask.json", i),
                MirroredAsk {
                    asker: format!("v-{i}"),
                    kind: "ask".into(),
                    subject: "s".into(),
                },
            );
        }
        assert_eq!(map.by_thread.len(), 5, "no eviction below the cap");
        assert!(map.asker_for_thread("17000000000.0001").is_some());
    }

    #[test]
    fn save_is_atomic_no_tmp_left() {
        let dir = tmp_dir("atomic");
        ThreadMap::default().save(&dir).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no .json.tmp left behind");
    }
}
