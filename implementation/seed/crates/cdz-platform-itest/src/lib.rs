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
//! Landed so far:
//! - [`ObservationLog`] — the one ordered, cheaply-clonable log of [`Record`]s (who / what / when).
//! - [`RecordingKvStore`] and [`RecordingBlobStore`] — decorators that log every store call and defer
//!   to the wrapped backend, so a run can be observed by swapping in a recording store (the stores are
//!   swappable trait objects by design, §7/§8).
//! - [`RecordingProgramStore`] and [`RecordingReducer`] — the event tap: wrap the program store handed
//!   to the kernel, and every reducer it instantiates records the events it folds, the requests it
//!   emits, and its close (§3/§4/§10) — capturing the whole system's event flow with no kernel change.
//!
//! Still to come: the driver that runs a reducer set to quiescence, and the checker ABI (a wasm program
//! that reads the log and asserts).

mod event;
mod log;
mod store;

pub use event::{RecordingProgramStore, RecordingReducer};
pub use log::{BlobOp, Entry, EventKind, EventOp, KvOp, ObservationLog, Record};
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

#[cfg(test)]
mod event_tap_tests {
    use super::{
        Entry, EventKind, EventOp, ObservationLog, RecordingProgramStore, RecordingReducer,
    };
    use bytes::Bytes;
    use cdz_platform::{
        ContractId, HostId, Message, Notification, Origin, Outcome, ProgramHash, Reducer,
        ReducerId, Request, Response, Spawned, spawned_contract,
    };

    /// A fixed clock — the event-tap tests assert on record order and content, not timestamps, so a
    /// constant time is enough (seq gives the order regardless).
    fn clock0() -> u64 {
        0
    }

    /// A reducer that, on its first message, emits one effect against a contract and closes.
    struct EmitAndClose {
        contract: ContractId,
    }
    #[async_trait::async_trait]
    impl Reducer for EmitAndClose {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            let request = Request {
                id: self.contract,
                payload: Bytes::from_static(b"effect"),
                continuation_token: Bytes::from_static(b"k"),
                deadline: None,
            };
            (
                vec![request],
                Outcome::Break {
                    schema: ContractId::of(b"done"),
                    reason: Bytes::from_static(b"emitted"),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A reducer that closes on its first message — a stand-in system reducer for the routed effect.
    struct JustClose;
    #[async_trait::async_trait]
    impl Reducer for JustClose {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: ContractId::of(b"shepherded"),
                    reason: Bytes::new(),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn a_recording_reducer_logs_birth_delivered_emitted_and_close_attributed_to_its_learned_id()
     {
        let program = ProgramHash::of(b"p");
        let me = ReducerId::of(b"self");
        let parent = ReducerId::of(b"parent");
        let host = HostId::of(b"node");
        let log = ObservationLog::new();
        let mut r = RecordingReducer::new(
            Box::new(EmitAndClose {
                contract: ContractId::of(b"eff"),
            }),
            program,
            host,
            log.clone(),
            clock0,
        );

        // Birth first — the reducer learns its own id from it (§3), so even this record is attributed to
        // the learned id, not the program.
        r.on_notification(Spawned { id: me, parent }.into_notification())
            .await;
        // Then a message that makes it emit an effect and close.
        let (_reqs, outcome) = r
            .on_message(Message {
                id: ContractId::of(b"go"),
                payload: Bytes::from_static(b"x"),
                from: Origin {
                    reducer: ReducerId::of(b"caller"),
                    host: HostId::of(b"other-node"),
                },
                continuation_token: Bytes::from_static(b"t"),
            })
            .await;
        assert!(matches!(outcome, Outcome::Break { .. }));

        let records = log.snapshot();
        assert_eq!(records.len(), 4);
        let expected_source = Origin { reducer: me, host };
        assert!(
            records.iter().all(|r| r.source == expected_source),
            "every record attributed to the learned id on this host"
        );
        // 1) the birth notification.
        assert!(matches!(
            &records[0].entry,
            Entry::Event(EventOp::Delivered { kind: EventKind::Notification, contract, .. })
                if *contract == spawned_contract()
        ));
        // 2) the delivered message, carrying its emitter Origin.
        assert!(matches!(
            &records[1].entry,
            Entry::Event(EventOp::Delivered { kind: EventKind::Message, from: Some(o), .. })
                if o.reducer == ReducerId::of(b"caller")
        ));
        // 3) the emitted effect.
        assert!(matches!(
            &records[2].entry,
            Entry::Event(EventOp::Emitted { contract, .. }) if *contract == ContractId::of(b"eff")
        ));
        // 4) the close, carrying the typed reason.
        assert!(matches!(
            &records[3].entry,
            Entry::Event(EventOp::Closed { schema, .. }) if *schema == ContractId::of(b"done")
        ));
    }

    #[test]
    fn the_program_store_seam_captures_the_whole_systems_event_flow_under_bach() {
        use bach::ext::*;
        use cdz_platform::{
            BachRuntime, InMemoryEventRegistry, InMemoryReducerGraph, Links, ReducerKind, Runtime,
            Spawn, System, TaskSystem,
        };
        use std::sync::Arc;

        bach::sim(|| {
            async {
                let log = ObservationLog::new();
                let host = HostId::of(b"node");
                let clock = BachRuntime::now as fn() -> u64;
                let emitter = ReducerId::of(b"emitter");
                let http = ContractId::of(b"http.get");

                let mut store = cdz_platform::testing::program::Store::new();
                store.register(ProgramHash::of(b"emitter"), move || {
                    Box::new(EmitAndClose { contract: http })
                });
                store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));

                // Wrap the program store: every reducer the kernel instantiates through it — the emitter and
                // the per-event system reducer it spawns to route the effect — is recorded.
                let recording = RecordingProgramStore::new(store, host, log.clone(), clock);
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(recording),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(ProgramHash::of(b"sys"))),
                    host,
                );
                system
                    .spawn(Spawn {
                        id: emitter,
                        program: ProgramHash::of(b"emitter"),
                        nonce: Bytes::from_static(b"nonce"),
                        parent: emitter,
                        kind: ReducerKind::Ordinary,
                        links: Links::NONE,
                    })
                    .await
                    .unwrap();
                system
                    .deliver(
                        emitter,
                        cdz_platform::Delivered::Message(Message {
                            id: ContractId::of(b"go"),
                            payload: Bytes::from_static(b"x"),
                            from: Origin {
                                reducer: ReducerId::of(b"caller"),
                                host,
                            },
                            continuation_token: Bytes::from_static(b"t"),
                        }),
                    )
                    .await
                    .unwrap();
                // Let the simulator route everything to quiescence (deterministic; no timers here).
                bach::time::sleep(core::time::Duration::from_millis(1)).await;

                let records = log.snapshot();
                // The emitter recorded emitting the http.get effect and then closing.
                assert!(
                    records.iter().any(|r| matches!(&r.entry,
                        Entry::Event(EventOp::Emitted { contract, .. }) if *contract == http)
                        && r.source.reducer == emitter),
                    "the emitter's effect was recorded"
                );
                assert!(
                    records
                        .iter()
                        .any(|r| matches!(&r.entry, Entry::Event(EventOp::Closed { .. }))
                            && r.source.reducer == emitter),
                    "the emitter's close was recorded"
                );
                // The system reducer — spawned by the kernel through the SAME recording store — recorded
                // receiving the routed effect as a message from the emitter. This proves the seam captures
                // kernel-spawned reducers, not only the ones the harness spawns directly.
                assert!(
                    records.iter().any(|r| matches!(&r.entry,
                        Entry::Event(EventOp::Delivered {
                            kind: EventKind::Message, contract, from: Some(o), ..
                        }) if *contract == http && o.reducer == emitter)),
                    "the kernel-spawned system reducer's receipt of the routed effect was recorded"
                );
            }
            .group("itest")
            .primary()
            .spawn();
        });
    }
}
