//! An executable integration-test harness for the Cadenza platform (`design/cadenza-platform.md`).
//!
//! The platform runs opaque program blobs and reducers to completion. This crate observes that run
//! and lets a checker assert over what happened, making no assumption that a program is Cadenza, Rust,
//! or anything — inputs are opaque blobs and the checker is a wasm program, so the log and the checker
//! interface are language-neutral. That neutrality is the contract the harness builds.
//!
//! It is the design's optional *observation log* (§9) made concrete for tests: record every event a
//! reducer folds and every key-value (§7) and blob (§8) store call it makes — what, by whom, and when
//! — into one ordered log, then run a checker over that log. Recording never alters a run; it only
//! observes it.
//!
//! This slice lands the store-observation foundation:
//! - [`ObservationLog`] — the one ordered, cheaply-clonable log of [`Record`]s (who / what / when).
//! - [`RecordingKvStore`] and [`RecordingBlobStore`] — decorators that log every store call and defer
//!   to the wrapped backend, so a run can be observed by swapping in a recording store (the stores are
//!   swappable trait objects by design, §7/§8).
//!
//! Still to come, in later slices: the event tap at the kernel's delivery choke point (recording every
//! delivered and emitted event with its emitter), the driver that runs a reducer set to quiescence, and
//! the checker ABI (a wasm program that reads the log and asserts).

mod log;
mod store;

pub use log::{BlobOp, Entry, KvOp, ObservationLog, Record};
pub use store::{RecordingBlobStore, RecordingKvStore};

#[cfg(test)]
mod tests {
    use super::{BlobOp, Entry, KvOp, ObservationLog, RecordingBlobStore, RecordingKvStore};
    use bytes::Bytes;
    use cdz_platform::{
        BlobStore, HostId, InMemoryBlobStore, InMemoryKvStore, KvStore, Origin, ReducerId,
    };
    use std::ops::Bound;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A test clock: a strictly increasing counter, so records carry distinct, ordered timestamps
    /// without depending on wall-clock time. A free `fn` with no captures coerces to `fn() -> u64`,
    /// the clock the recording stores take.
    static CLOCK: AtomicU64 = AtomicU64::new(0);
    fn tick_clock() -> u64 {
        CLOCK.fetch_add(1, Ordering::SeqCst)
    }

    fn origin(reducer: &[u8]) -> Origin {
        Origin {
            reducer: ReducerId::of(reducer),
            host: HostId::of(b"test-node"),
        }
    }

    fn whole_store() -> cdz_platform::KeyRange {
        (Bound::Unbounded, Bound::Unbounded)
    }

    #[tokio::test]
    async fn kv_calls_are_recorded_with_who_what_and_order_and_pass_through_unchanged() {
        let who = origin(b"agent-1");
        let log = ObservationLog::new();
        let mut kv = RecordingKvStore::new(InMemoryKvStore::new(), who, log.clone(), tick_clock);

        // The wrapped store returns exactly what the backend would — a miss, then a hit after a put,
        // then a delete reporting the entry existed. Recording does not alter behavior (§9).
        assert_eq!(kv.get(b"k").await, None);
        kv.put(Bytes::from_static(b"k"), Bytes::from_static(b"v"))
            .await;
        assert_eq!(kv.get(b"k").await, Some(Bytes::from_static(b"v")));
        assert!(kv.delete(b"k").await);
        let _ = kv.scan(whole_store());

        let records = log.snapshot();
        assert_eq!(records.len(), 5, "one record per call, in call order");
        // seq is the dense global order 0..n; every record is attributed to the acting reducer; and the
        // timestamps strictly increase with the injected clock.
        for (i, r) in records.iter().enumerate() {
            assert_eq!(r.seq, i as u64);
            assert_eq!(r.source, who);
        }
        assert!(records.windows(2).all(|w| w[0].time_ns < w[1].time_ns));
        // Each recorded op carries the operation and its observed outcome.
        assert_eq!(
            records[0].entry,
            Entry::Kv(KvOp::Get {
                key: Bytes::from_static(b"k"),
                hit: false
            })
        );
        assert_eq!(
            records[1].entry,
            Entry::Kv(KvOp::Put {
                key: Bytes::from_static(b"k"),
                value: Bytes::from_static(b"v"),
            })
        );
        assert_eq!(
            records[2].entry,
            Entry::Kv(KvOp::Get {
                key: Bytes::from_static(b"k"),
                hit: true
            })
        );
        assert_eq!(
            records[3].entry,
            Entry::Kv(KvOp::Delete {
                key: Bytes::from_static(b"k"),
                existed: true
            })
        );
        assert_eq!(
            records[4].entry,
            Entry::Kv(KvOp::Scan {
                lower: Bound::Unbounded,
                upper: Bound::Unbounded,
                keys_only: false,
            })
        );
    }

    #[tokio::test]
    async fn blob_calls_are_recorded_with_hash_length_and_outcome() {
        let who = origin(b"agent-2");
        let log = ObservationLog::new();
        let mut blobs =
            RecordingBlobStore::new(InMemoryBlobStore::new(), who, log.clone(), tick_clock);

        let bytes = Bytes::from_static(b"hello observation log");
        let hash = blobs.put(bytes.clone()).await;
        assert_eq!(
            blobs.get(hash).await,
            Some(bytes.clone()),
            "put/get round-trips"
        );
        let absent = cdz_platform::Hash::of(cdz_platform::HashTag::Blob, b"never stored");
        assert_eq!(blobs.get(absent).await, None);
        assert!(blobs.has(hash).await);
        assert!(!blobs.has(absent).await);

        let records = log.snapshot();
        assert_eq!(records.len(), 5);
        assert!(records.iter().all(|r| r.source == who));
        assert_eq!(
            records[0].entry,
            Entry::Blob(BlobOp::Put {
                hash,
                len: bytes.len()
            })
        );
        assert_eq!(
            records[1].entry,
            Entry::Blob(BlobOp::Get { hash, hit: true })
        );
        assert_eq!(
            records[2].entry,
            Entry::Blob(BlobOp::Get {
                hash: absent,
                hit: false
            })
        );
        assert_eq!(
            records[3].entry,
            Entry::Blob(BlobOp::Has {
                hash,
                present: true
            })
        );
        assert_eq!(
            records[4].entry,
            Entry::Blob(BlobOp::Has {
                hash: absent,
                present: false
            })
        );
    }

    /// The log is one global order across reducers, and it drives under the bach simulator with the
    /// runtime's deterministic clock — the seam that proves observation is runtime-agnostic and
    /// deterministic. Two reducers share one log; their interleaved store calls land in a single `seq`
    /// order, each attributed to its own reducer, and every record carries simulated time.
    #[test]
    fn one_log_orders_two_reducers_deterministically_under_bach() {
        use bach::ext::*;
        use cdz_platform::{BachRuntime, Runtime};

        bach::sim(|| {
            async {
                let clock = BachRuntime::now as fn() -> u64;
                let log = ObservationLog::new();
                let alice = origin(b"alice");
                let bob = origin(b"bob");
                let mut a =
                    RecordingKvStore::new(InMemoryKvStore::new(), alice, log.clone(), clock);
                let mut b =
                    RecordingBlobStore::new(InMemoryBlobStore::new(), bob, log.clone(), clock);

                // Interleave the two reducers' store calls against the one log.
                a.put(Bytes::from_static(b"x"), Bytes::from_static(b"1"))
                    .await;
                let h = b.put(Bytes::from_static(b"payload")).await;
                assert_eq!(a.get(b"x").await, Some(Bytes::from_static(b"1")));
                assert!(b.has(h).await);

                let records = log.snapshot();
                assert_eq!(records.len(), 4);
                // The single global order interleaves both reducers, each record attributed correctly.
                assert_eq!(records[0].source, alice);
                assert_eq!(records[1].source, bob);
                assert_eq!(records[2].source, alice);
                assert_eq!(records[3].source, bob);
                assert!(matches!(records[0].entry, Entry::Kv(KvOp::Put { .. })));
                assert!(matches!(records[1].entry, Entry::Blob(BlobOp::Put { .. })));
                // seq is dense and monotonic; time is the simulator's clock (non-decreasing).
                for (i, r) in records.iter().enumerate() {
                    assert_eq!(r.seq, i as u64);
                }
                assert!(records.windows(2).all(|w| w[0].time_ns <= w[1].time_ns));
            }
            .group("observation-log")
            .primary()
            .spawn();
        });
    }
}
