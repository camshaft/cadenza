//! Property test for the v0.1 milestone (§15b: "prove replay determinism") and the §16c-S3 invariant:
//! **replaying a session's own log reconstructs a bit-identical KV** (same root hash), for arbitrary
//! event sequences, within one kernel version.
//!
//! Strategy: a reducer that performs varied, content-dependent KV mutations AND emits effects (so the
//! log grows the full trigger→dispatch→result chain), driven by many deterministically-generated
//! inbound sequences. For each: run to quiescence, snapshot the KV root, replay the resulting log into
//! a fresh Session, and assert the replayed KV root equals the original. If replay ever diverged
//! (nondeterministic fold, order-dependent KV, encoding drift), the root hashes would differ.
//!
//! Deterministic generation (a tiny seeded LCG) keeps the test reproducible — no external rng dep, no
//! wall-clock/entropy (which would themselves violate the determinism the test checks).

use crate::authz::Authorizer;
use crate::effect::{
    Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use crate::event::{ContentType, Event, EventBody};
use crate::executor::RecordingExecutor;
use crate::hash::Hash;
use crate::kernel::Session;
use crate::kv::Kv;
use crate::reducer::{FoldOutput, Reducer};

/// A reducer that exercises varied KV shapes and the effect chain. On an inbound message it writes a
/// per-key counter, appends to a running list under a scanned prefix, and (for some inputs) emits an
/// Http effect; when a result comes back it bumps a "completed" counter and sometimes deletes a key.
/// The behavior is a pure function of the event + current KV — exactly what replay must reproduce.
struct BusyReducer;

#[async_trait::async_trait(?Send)]
impl Reducer for BusyReducer {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { payload, .. } => {
                let byte = match payload {
                    Payload::Inline(b) if !b.is_empty() => b[0],
                    _ => 0,
                };
                // Per-value counter.
                let key = format!("count/{byte}");
                let n = kv
                    .get(key.as_bytes())
                    .map(|b| b[0])
                    .unwrap_or(0)
                    .wrapping_add(1);
                kv.put(key.into_bytes(), vec![n]);

                // Append into a prefix-scanned collection (exercises prefix_scan ordering determinism).
                let seq_key = format!("item/{:08}", kv.prefix_scan(b"item/").len());
                kv.put(seq_key.into_bytes(), vec![byte]);

                // Occasionally delete an old key (exercises delete in the fold).
                if byte % 5 == 0 {
                    kv.delete(b"count/0");
                }

                // Every third input emits an effect, so the log grows the dispatch/result chain.
                if byte % 3 == 0 {
                    FoldOutput::with(vec![EffectRequest::new(
                        EffectKind::Http,
                        "https://ok.host/x",
                        None,
                        Timeliness::Interactive,
                    )])
                } else {
                    FoldOutput::none()
                }
            }
            EventBody::EffectResult { .. } => {
                let n = kv
                    .get(b"completed")
                    .map(|b| b[0])
                    .unwrap_or(0)
                    .wrapping_add(1);
                kv.put(b"completed".to_vec(), vec![n]);
                FoldOutput::none()
            }
            _ => FoldOutput::none(),
        }
    }
}

/// A tiny deterministic LCG (Numerical Recipes constants) — reproducible pseudo-randomness with no
/// external dep and no entropy source.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn byte(&mut self) -> u8 {
        (self.next() >> 33) as u8
    }
}

fn cap() -> Authorizer {
    Authorizer::new(vec![Capability {
        kind: EffectKind::Http,
        predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
    }])
}

/// Run one generated sequence to quiescence and return the resulting session.
// Returns the built session AND the durable log captured through a recording sink (log-decouple I5:
// replay-equivalence reads the log from the SOURCE, not the resident Vec). Callers that only need the
// session's derived state ignore the second element.
async fn run_sequence(seed: u64, len: usize) -> (Session, crate::test_log_source::CapturedLog) {
    let mut reducer = BusyReducer;
    let authz = cap();
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"busy-v1"), Hash::of(b"test-spawn-nonce"));
    let captured = crate::test_log_source::attach_recording_sink(&mut session);
    let mut rng = Lcg(seed);
    for _ in 0..len {
        let byte = rng.byte();
        let body = EventBody::Inbound {
            content_type: ContentType {
                family: "m".into(),
                version: 1,
            },
            payload: Payload::Inline(vec![byte].into()),
        };
        session
            .deliver(body, None, &mut reducer, &authz, &mut exec)
            .await
            .unwrap();
    }
    (session, captured)
}

#[tokio::test(flavor = "current_thread")]
async fn replay_reconstructs_identical_kv_root_over_many_sequences() {
    let mut reducer = BusyReducer;
    // Many seeds × varied lengths — a broad sweep of event sequences.
    for seed in 0..200u64 {
        let len = (seed as usize % 40) + 1;
        let (session, captured) = run_sequence(seed.wrapping_mul(2654435761), len).await;

        let original_root = session.snapshot().kv_root;
        let log = crate::test_log_source::replay_input(&captured);

        // Replay the session's OWN log into a fresh Session and compare KV roots.
        let replayed = Session::replay(log.clone(), &mut reducer).await.unwrap();
        assert_eq!(
            replayed.snapshot().kv_root,
            original_root,
            "replay diverged for seed={seed} len={len}: KV root differs (§16c-S3)"
        );

        // Replay must also be idempotent: replaying the same durable log again matches (a replay of log L
        // yields a session whose log IS L, so the second replay's input is the same `log`).
        let twice = Session::replay(log.clone(), &mut reducer).await.unwrap();
        assert_eq!(
            twice.snapshot().kv_root,
            original_root,
            "second replay diverged for seed={seed}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn identical_sequences_produce_identical_roots() {
    // Determinism of the forward run itself: the same seed twice → identical KV root (no hidden
    // nondeterminism in the fold/drive path).
    for seed in 0..50u64 {
        let len = (seed as usize % 20) + 1;
        let a = run_sequence(seed, len).await.0.snapshot().kv_root;
        let b = run_sequence(seed, len).await.0.snapshot().kv_root;
        assert_eq!(
            a, b,
            "same sequence produced different roots for seed={seed}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn different_sequences_generally_produce_different_roots() {
    // Sanity that the root actually reflects contents (guards against a degenerate always-equal hash
    // that would make the determinism test vacuous). Not every pair must differ, but the set of roots
    // over many distinct sequences must have real variety.
    use std::collections::HashSet;
    let mut roots = HashSet::new();
    for seed in 0..100u64 {
        let root = run_sequence(seed.wrapping_mul(2654435761), (seed as usize % 30) + 3)
            .await
            .0
            .snapshot()
            .kv_root;
        roots.insert(root);
    }
    assert!(
        roots.len() > 50,
        "expected varied KV roots across distinct sequences, got only {} — root may not reflect contents",
        roots.len()
    );
}

/// A reducer whose KV writes deliberately depend on prefix_scan RESULTS, to stress that scan order is
/// a deterministic total order (§16c-S8): if scan order ever depended on insertion history, replaying
/// a log whose events arrived in a different internal order would diverge.
struct ScanOrderReducer;
#[async_trait::async_trait(?Send)]
impl Reducer for ScanOrderReducer {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        if let EventBody::Inbound { payload, .. } = &event.body {
            let byte = match payload {
                Payload::Inline(b) if !b.is_empty() => b[0],
                _ => 0,
            };
            kv.put(format!("k/{byte:03}").into_bytes(), vec![byte]);
            // Fold a digest of the CURRENT scan order into a running key. If scan order were
            // nondeterministic, this digest — and thus the KV root — would vary across replay.
            let mut digest = 0u8;
            for (k, _) in kv.prefix_scan(b"k/") {
                digest = digest.wrapping_add(*k.last().unwrap());
            }
            kv.put(b"scan-digest".to_vec(), vec![digest]);
        }
        FoldOutput::none()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn scan_order_dependent_reducer_still_replays_identically() {
    let mut reducer = ScanOrderReducer;
    let authz = Authorizer::deny_all(); // this reducer emits no effects
    for seed in 0..100u64 {
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"scan-v1"), Hash::of(b"test-spawn-nonce"));
        let captured = crate::test_log_source::attach_recording_sink(&mut session);
        let mut rng = Lcg(seed.wrapping_mul(11400714819323198485));
        for _ in 0..((seed as usize % 30) + 1) {
            let body = EventBody::Inbound {
                content_type: ContentType {
                    family: "m".into(),
                    version: 1,
                },
                payload: Payload::Inline(vec![rng.byte()].into()),
            };
            session
                .deliver(body, None, &mut reducer, &authz, &mut exec)
                .await
                .unwrap();
        }
        let original = session.snapshot().kv_root;
        let replayed = Session::replay(
            crate::test_log_source::replay_input(&captured),
            &mut reducer,
        )
        .await
        .unwrap();
        assert_eq!(
            replayed.snapshot().kv_root,
            original,
            "scan-order replay diverged for seed={seed}"
        );
    }
}
