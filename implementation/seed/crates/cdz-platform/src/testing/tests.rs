//! Tests for the observation log, the recording decorators, the event tap, and the harness driver —
//! exercising the public `testing` surface (`design/cadenza-platform.md` §3/§4/§7/§8/§9).

use super::{
    BlobOp, CheckOutcome, Entry, EventKind, EventOp, Harness, KvOp, ObservationLog,
    RecordingBlobStore, RecordingKvStore, RecordingProgramStore, RecordingReducer, Run, SpawnSpec,
};
use crate::{
    BachRuntime, BlobStore, Bytes, ContractId, Delivered, Hash, HashTag, HostId, InMemoryBlobStore,
    InMemoryEventRegistry, InMemoryKvStore, InMemoryReducerGraph, KeyRange, KvStore, Links,
    Message, Notification, Origin, Outcome, ProgramHash, Reducer, ReducerId, ReducerKind, Request,
    Response, Runtime, Spawn, Spawned, System, TaskSystem, spawned_contract,
};
use std::ops::Bound;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A test clock: a strictly increasing counter, so records carry distinct, ordered timestamps
/// without depending on wall-clock time. A free `fn` with no captures coerces to `fn() -> u64`,
/// the clock the recording decorators take.
static CLOCK: AtomicU64 = AtomicU64::new(0);
fn tick_clock() -> u64 {
    CLOCK.fetch_add(1, Ordering::SeqCst)
}

/// A fixed clock — the event-tap tests assert on record order and content, not timestamps, so a
/// constant time is enough (seq gives the order regardless).
fn clock0() -> u64 {
    0
}

fn origin(reducer: &[u8]) -> Origin {
    Origin {
        reducer: ReducerId::of(reducer),
        host: HostId::of(b"test-node"),
    }
}

fn whole_store() -> KeyRange {
    (Bound::Unbounded, Bound::Unbounded)
}

/// A reducer that, on its first message, emits one effect against a contract and closes.
struct EmitAndClose {
    contract: ContractId,
}
#[async_trait::async_trait]
impl Reducer for EmitAndClose {
    async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
        (
            vec![Request {
                id: self.contract,
                payload: Bytes::from_static(b"effect"),
                continuation_token: Bytes::from_static(b"k"),
                deadline: None,
            }],
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

/// A stand-in system reducer: closes on the routed effect (or message) it receives.
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

// ---- observation log + recording store decorators (§7/§8) ----

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
    let mut blobs = RecordingBlobStore::new(InMemoryBlobStore::new(), who, log.clone(), tick_clock);

    let bytes = Bytes::from_static(b"hello observation log");
    let hash = blobs.put(bytes.clone()).await;
    assert_eq!(
        blobs.get(hash).await,
        Some(bytes.clone()),
        "put/get round-trips"
    );
    let absent = Hash::of(HashTag::Blob, b"never stored");
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

    bach::sim(|| {
        async {
            let clock = BachRuntime::now as fn() -> u64;
            let log = ObservationLog::new();
            let alice = origin(b"alice");
            let bob = origin(b"bob");
            let mut a = RecordingKvStore::new(InMemoryKvStore::new(), alice, log.clone(), clock);
            let mut b = RecordingBlobStore::new(InMemoryBlobStore::new(), bob, log.clone(), clock);

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

// ---- event tap (§3/§4/§10) ----

#[tokio::test]
async fn a_recording_reducer_logs_birth_delivered_emitted_and_close_attributed_to_its_learned_id() {
    let me = ReducerId::of(b"self");
    let parent = ReducerId::of(b"parent");
    let host = HostId::of(b"node");
    let log = ObservationLog::new();
    let mut r = RecordingReducer::new(
        Box::new(EmitAndClose {
            contract: ContractId::of(b"eff"),
        }),
        me,
        host,
        log.clone(),
        clock0,
    );

    // The reducer's id is known at construction (from the spawn context, §3), so every record — including
    // the birth notification it folds first — is attributed to it.
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

    bach::sim(|| {
        async {
            let log = ObservationLog::new();
            let host = HostId::of(b"node");
            let clock = BachRuntime::now as fn() -> u64;
            let emitter = ReducerId::of(b"emitter");
            let http = ContractId::of(b"http.get");

            let mut store = crate::program::testing::Store::new();
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
                    Delivered::Message(Message {
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

// ---- the run-to-quiescence harness driver (§3/§9) ----

/// Build a fresh harness for the same run — an emitter that performs one effect and closes, plus the
/// default system reducer that shepherds it — so it can be run more than once. The caller NAMES the
/// spawn ("emitter") and delivers by that name; the harness derives the reducer id from its genesis.
fn emitter_run() -> Harness<crate::program::testing::Store> {
    let http = ContractId::of(b"http.get");
    let mut store = crate::program::testing::Store::new();
    store.register(ProgramHash::of(b"emitter"), move || {
        Box::new(EmitAndClose { contract: http })
    });
    store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));

    Harness::new(store, ProgramHash::of(b"sys"))
        .host(HostId::of(b"node"))
        .spawn(SpawnSpec::new("emitter", ProgramHash::of(b"emitter")))
        .deliver(
            "emitter",
            Delivered::Message(Message {
                id: ContractId::of(b"go"),
                payload: Bytes::from_static(b"x"),
                from: Origin {
                    reducer: ReducerId::of(b"caller"),
                    host: HostId::of(b"node"),
                },
                continuation_token: Bytes::from_static(b"t"),
            }),
        )
}

#[test]
fn the_driver_runs_a_reducer_set_to_quiescence_and_returns_the_event_log() {
    let http = ContractId::of(b"http.get");
    let run = emitter_run().run();
    // The harness assigned the named spawn a genesis-derived id, which the checker reads back by name.
    let emitter = run.ids["emitter"];
    let records = &run.records;

    assert!(!records.is_empty(), "the run produced observations");
    // The emitter emitted its effect and then closed.
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
    // The kernel-spawned system reducer received the routed effect as a message from the emitter, so
    // the run drove past the initial delivery through the effect's dispatch to quiescence.
    assert!(
        records.iter().any(|r| matches!(&r.entry,
            Entry::Event(EventOp::Delivered {
                kind: EventKind::Message, contract, from: Some(o), ..
            }) if *contract == http && o.reducer == emitter)),
        "the routed effect reached the system reducer"
    );
}

#[test]
fn the_driver_resolves_a_delivery_to_the_named_spawns_derived_id() {
    // A delivery names its target; the harness resolves that name to the genesis-derived id it assigned
    // the spawn, and the delivered event is recorded at that reducer. The caller never writes the id.
    let run = emitter_run().run();
    let emitter = run.ids["emitter"];
    // The `go` message the run delivered by name was folded at the emitter (the assigned id).
    assert!(
        run.records.iter().any(|r| matches!(&r.entry,
            Entry::Event(EventOp::Delivered { kind: EventKind::Message, contract, .. })
                if *contract == ContractId::of(b"go"))
            && r.source.reducer == emitter),
        "the by-name delivery reached the reducer the harness assigned that name"
    );
}

#[test]
fn the_driver_is_deterministic_two_identical_runs_produce_the_same_log() {
    // Determinism via bach is part of the harness contract (operator directive): the same run, driven
    // under the deterministic simulator, produces byte-for-byte the same observation log AND the same
    // name→id assignment every time — same events, same order (seq), same simulated timestamps. This is
    // exactly what lets a checker assert over the log without flake.
    let first = emitter_run().run();
    let second = emitter_run().run();
    assert_eq!(first, second, "two identical runs yield an identical Run");
}

// ---- checkers over the run (§9) ----

#[test]
fn checkers_pass_on_a_matching_log_and_fail_with_reasons_otherwise() {
    // A checker asserting the emitter performed http.get and then closed — written by NAME, resolving the
    // name to the recorded id through the run's id map (records_from). This is the assertion side of the
    // harness: the run records what happened, the checker states what should have.
    fn emitter_did_http_and_closed(run: &Run) -> CheckOutcome {
        let http = ContractId::of(b"http.get");
        let from_emitter: Vec<_> = run.records_from("emitter").collect();
        let mut reasons = Vec::new();
        if !from_emitter.iter().any(|r| {
            matches!(&r.entry,
            Entry::Event(EventOp::Emitted { contract, .. }) if *contract == http)
        }) {
            reasons.push("emitter never performed http.get".to_string());
        }
        if !from_emitter
            .iter()
            .any(|r| matches!(&r.entry, Entry::Event(EventOp::Closed { .. })))
        {
            reasons.push("emitter never closed".to_string());
        }
        CheckOutcome::from_reasons(reasons)
    }

    // A checker asserting something the run does not do — it fails, carrying its reason.
    fn expects_a_kv_write(run: &Run) -> CheckOutcome {
        if run.records.iter().any(|r| matches!(r.entry, Entry::Kv(_))) {
            CheckOutcome::pass()
        } else {
            CheckOutcome::fail("expected a key-value write, but the run made none")
        }
    }

    let run = emitter_run().run();
    assert!(
        run.check(&emitter_did_http_and_closed).is_pass(),
        "the checker holds on the matching log"
    );
    let verdict = run.check(&expects_a_kv_write);
    assert!(
        !verdict.is_pass(),
        "the checker fails on an unmet assertion"
    );
    assert_eq!(
        verdict.reasons(),
        ["expected a key-value write, but the run made none"]
    );
}

#[test]
fn the_log_records_each_spawns_name_and_id_so_it_is_self_describing() {
    let run = emitter_run().run();
    // The log itself carries the name→id assignment: a Spawn record naming "emitter", whose source is the
    // id the harness derived — so anything reading the log (including a wasm checker over a serialized log)
    // derefs the name to its id with no out-of-band map.
    let (id_in_log, program) = run
        .records
        .iter()
        .find_map(|r| match &r.entry {
            Entry::Spawn(info) if info.name == "emitter" => Some((r.source.reducer, info.program)),
            _ => None,
        })
        .expect("the log records the emitter spawn by name");
    assert_eq!(
        id_in_log, run.ids["emitter"],
        "the recorded id matches the name→id assignment"
    );
    assert_eq!(program, ProgramHash::of(b"emitter"));
    // Spawn records lead the log — the run's setup, ahead of any reducer's birth or events.
    let first_spawn = run
        .records
        .iter()
        .find(|r| matches!(&r.entry, Entry::Spawn(_)))
        .expect("a spawn record");
    assert_eq!(first_spawn.seq, 0, "spawn records lead the log");
}
