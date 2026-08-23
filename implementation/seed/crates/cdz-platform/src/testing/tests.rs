//! Tests for the observation log, the recording decorators, the event tap, and the harness driver —
//! exercising the public `testing` surface (`design/cadenza-platform.md` §3/§4/§7/§8/§9).

use super::{
    BlobOp, CheckOutcome, Entry, EventKind, EventOp, Harness, KvOp, ObservationLog,
    RecordingBlobStore, RecordingKvStore, RecordingProgramStore, RecordingReducer, Run, SpawnSpec,
    check_contract, check_message, decode_check, deserialize_log, encode_verdict, render,
    verdict_contract, verdict_in,
};
use crate::{
    BachRuntime, BlobStore, Bytes, ContractId, Delivered, FireAfter, Fired, Hash, HashTag, HostId,
    InMemoryBlobStore, InMemoryEventRegistry, InMemoryKvStore, InMemoryReducerGraph, KeyRange,
    KvStore, Links, Message, Notification, Origin, Outcome, ProgramHash, Reducer, ReducerId,
    ReducerKind, Request, Response, Runtime, Spawn, Spawned, Str, System, TaskSystem,
    spawned_contract, timer_contract,
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

/// Run a fresh copy of the same scenario — an emitter task that performs one effect and closes, plus the
/// system reducer that shepherds it. The caller NAMES a program blob and a task running it and delivers by
/// task name; the harness derives the hashes/ids. The native store factory (which ignores the seeded CAS)
/// registers the reducers under the same content hashes the blobs resolve to (`ProgramHash::of(bytes)`).
/// Returns the `Run` so it can be run more than once.
fn emitter_run() -> Run {
    let http = ContractId::of(b"http.get");
    Harness::new("sys")
        .host(HostId::of(b"node"))
        .blob("emitter", Bytes::from_static(b"emitter"))
        .blob("sys", Bytes::from_static(b"sys"))
        .spawn(SpawnSpec::new("emitter", "emitter"))
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
        .run(move |_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"emitter"), move || {
                Box::new(EmitAndClose { contract: http })
            });
            store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));
            store
        })
}

#[test]
fn the_driver_runs_a_reducer_set_to_quiescence_and_returns_the_event_log() {
    let http = ContractId::of(b"http.get");
    let run = emitter_run();
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
    let run = emitter_run();
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
    let first = emitter_run();
    let second = emitter_run();
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

    let run = emitter_run();
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
    let run = emitter_run();
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

// ---- richer harness scenarios: child lineage, timers (§3/§6) ----

/// A reducer that arms a fire-after timer on its first message, then closes when it is woken (the timer's
/// `Fired` response). Exercises the harness driving a timer to fire and the tap recording the round-trip.
struct ArmThenCloseOnFire {
    duration_ns: u64,
}
#[async_trait::async_trait]
impl Reducer for ArmThenCloseOnFire {
    async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
        let arm = FireAfter {
            duration: self.duration_ns,
        };
        (
            vec![arm.into_request(Bytes::from_static(b"wake"))],
            Outcome::Continue,
        )
    }
    async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
        if r.id == timer_contract() && matches!(&r.payload, Ok(b) if Fired::decode(b).is_some()) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: ContractId::of(b"woke"),
                    reason: Bytes::new(),
                },
            )
        } else {
            (Vec::new(), Outcome::Continue)
        }
    }
    async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }
}

#[test]
fn a_child_spawn_resolves_its_parent_by_name_and_gets_a_distinct_lineage_bearing_id() {
    // A run names a parent task and a child of it; the harness derives each id from its genesis and resolves
    // the child's parent by name — no reducer hash written by hand (§3).
    let run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("parent", Bytes::from_static(b"parent"))
        .blob("child", Bytes::from_static(b"child"))
        .spawn(SpawnSpec::new("parent", "parent"))
        .spawn(SpawnSpec::new("child", "child").child_of("parent"))
        .run(|_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"parent"), || Box::new(JustClose));
            store.register(ProgramHash::of(b"child"), || Box::new(JustClose));
            store
        });

    let parent = run.ids["parent"];
    let child = run.ids["child"];
    assert_ne!(
        parent, child,
        "parent and child get distinct genesis-derived ids"
    );

    // The child's spawn record names the parent's ASSIGNED id — lineage resolved from the name.
    let child_parent = run
        .records
        .iter()
        .find_map(|r| match &r.entry {
            Entry::Spawn(info) if info.name == "child" => Some(info.parent),
            _ => None,
        })
        .expect("child spawn recorded");
    assert_eq!(
        child_parent, parent,
        "the child's parent is the parent's assigned id"
    );

    // A root is its own parent: the parent's spawn record's parent equals its own id (the record source).
    let (parent_id_in_log, parent_of_parent) = run
        .records
        .iter()
        .find_map(|r| match &r.entry {
            Entry::Spawn(info) if info.name == "parent" => Some((r.source.reducer, info.parent)),
            _ => None,
        })
        .expect("parent spawn recorded");
    assert_eq!(
        parent_of_parent, parent_id_in_log,
        "a root is its own parent"
    );
}

#[test]
fn a_fire_after_timer_is_driven_to_fire_and_the_round_trip_is_recorded() {
    // The armer arms a 10ms timer on the go message; the run advances simulated time (run_for defaults to a
    // virtual hour) so the timer fires and the armer is woken. bach makes this deterministic.
    let run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("armer", Bytes::from_static(b"armer"))
        .spawn(SpawnSpec::new("armer", "armer"))
        .deliver(
            "armer",
            Delivered::Message(Message {
                id: ContractId::of(b"go"),
                payload: Bytes::from_static(b"x"),
                from: Origin {
                    reducer: ReducerId::of(b"caller"),
                    host: HostId::of(b"ingress"),
                },
                continuation_token: Bytes::from_static(b"t"),
            }),
        )
        .run(|_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"armer"), || {
                Box::new(ArmThenCloseOnFire {
                    duration_ns: 10_000_000, // 10ms of simulated time
                })
            });
            store
        });

    // The arm was emitted against the timer contract...
    assert!(
        run.records_from("armer").any(|r| matches!(&r.entry,
            Entry::Event(EventOp::Emitted { contract, .. }) if *contract == timer_contract())),
        "the fire-after arm was recorded"
    );
    // ...the run drove time forward so it fired, delivered back as a response on the timer contract...
    assert!(
        run.records_from("armer").any(|r| matches!(&r.entry,
            Entry::Event(EventOp::Delivered { kind: EventKind::Response, contract, .. })
                if *contract == timer_contract())),
        "the timer Fired response was recorded — the run drove the timer to fire"
    );
    // ...and the armer closed on the fire, so the run reached quiescence.
    assert!(
        run.records_from("armer")
            .any(|r| matches!(&r.entry, Entry::Event(EventOp::Closed { .. }))),
        "the armer closed after the timer fired"
    );
}

// ---- uncontrolled fold failure (§3/§10 fold-failed) ----

/// A reducer whose fold panics — an uncontrolled failure, distinct from a controlled `Break`.
struct Panicker;
#[async_trait::async_trait]
impl Reducer for Panicker {
    async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
        panic!("boom");
    }
    async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }
    async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }
}

#[test]
fn a_fold_panic_is_recorded_as_fold_failed() {
    // Delivering the go message makes the panicker's fold panic. The runtime catches the crash (it does
    // not take down the run), and the tap records it as fold-failed first — the reducer's terminal event.
    let run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("panicker", Bytes::from_static(b"panicker"))
        .spawn(SpawnSpec::new("panicker", "panicker"))
        .deliver(
            "panicker",
            Delivered::Message(Message {
                id: ContractId::of(b"go"),
                payload: Bytes::from_static(b"x"),
                from: Origin {
                    reducer: ReducerId::of(b"caller"),
                    host: HostId::of(b"ingress"),
                },
                continuation_token: Bytes::from_static(b"t"),
            }),
        )
        .run(|_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"panicker"), || Box::new(Panicker));
            store
        });

    let panicker = run.ids["panicker"];
    assert!(
        run.records.iter().any(|r| matches!(&r.entry,
            Entry::Event(EventOp::Failed { during: EventKind::Message, contract, reason })
                if *contract == ContractId::of(b"go") && reason.as_str().contains("boom"))
            && r.source.reducer == panicker),
        "the uncontrolled fold panic was recorded as fold-failed, naming the event and the reason"
    );
}

/// A reducer that, on any message, writes one key-value pair and stores one blob through the recording
/// stores it was given, then continues — a stand-in for a reducer whose fold touches its direct-access
/// stores (§7/§8). It holds the recording decorators so its KV and blob calls are observed.
struct Storer {
    kv: RecordingKvStore<InMemoryKvStore>,
    blobs: RecordingBlobStore<InMemoryBlobStore>,
}

#[async_trait::async_trait]
impl Reducer for Storer {
    async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
        self.kv
            .put(Bytes::from_static(b"k"), Bytes::from_static(b"v"))
            .await;
        self.blobs.put(Bytes::from_static(b"blob-payload")).await;
        (Vec::new(), Outcome::Continue)
    }

    async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }

    async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }
}

#[test]
fn a_shared_log_records_a_reducers_kv_and_blob_calls_alongside_its_events() {
    // A caller shares one observation log with the harness (via `.log`) AND with the recording stores it
    // wires into the reducer through `make_store`. So when the reducer folds the delivered message and makes
    // KV and blob calls, those calls land in the SAME ordered log as the event it folded — the harness's
    // core promise (§9), the KV/blob half of it that the event tap alone does not cover. Without `.log`, the
    // stores would record into a log the run never returns.
    let log = ObservationLog::new();
    let store_log = log.clone();
    let owner = Origin {
        reducer: ReducerId::of(b"storer-backend"),
        host: HostId::of(b"h"),
    };

    let run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("storer", Bytes::from_static(b"storer"))
        .spawn(SpawnSpec::new("storer", "storer"))
        .deliver(
            "storer",
            Delivered::Message(Message {
                id: ContractId::of(b"go"),
                payload: Bytes::from_static(b"x"),
                from: Origin {
                    reducer: ReducerId::of(b"caller"),
                    host: HostId::of(b"ingress"),
                },
                continuation_token: Bytes::from_static(b"t"),
            }),
        )
        .log(log)
        .run(move |_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"storer"), move || {
                Box::new(Storer {
                    kv: RecordingKvStore::new(
                        InMemoryKvStore::new(),
                        owner,
                        store_log.clone(),
                        tick_clock as fn() -> u64,
                    ),
                    blobs: RecordingBlobStore::new(
                        InMemoryBlobStore::new(),
                        owner,
                        store_log.clone(),
                        tick_clock as fn() -> u64,
                    ),
                })
            });
            store
        });

    // The one returned log carries the reducer's KV put, its blob put, and the message it folded.
    assert!(
        run.records
            .iter()
            .any(|r| matches!(&r.entry, Entry::Kv(KvOp::Put { .. }))),
        "the reducer's KV put is in the shared log:\n{}",
        crate::testing::render(&run.records)
    );
    assert!(
        run.records
            .iter()
            .any(|r| matches!(&r.entry, Entry::Blob(BlobOp::Put { .. }))),
        "the reducer's blob put is in the shared log:\n{}",
        crate::testing::render(&run.records)
    );
    assert!(
        run.records.iter().any(|r| matches!(
            &r.entry,
            Entry::Event(EventOp::Delivered {
                kind: EventKind::Message,
                ..
            })
        )),
        "the delivered message is in the same log — events and store calls interleaved"
    );
}

#[test]
fn a_real_runs_log_round_trips_through_the_structured_serializer() {
    // The structured observation log (`serialize_log` / `deserialize_log`) is the language-neutral form a
    // checker reads (§9). Its own tests round-trip hand-built records of every variant; this pins that the
    // records a REAL harness run actually produces — a spawn (name→id), a delivered message, and a reducer's
    // KV and blob calls — round-trip byte-for-byte too. It is the cross-module guard between `observation`
    // (what a run records) and `log_value` (how the log serializes): a new `Entry` a run emits but the
    // serializer does not handle would break this, catching the omission at the gate.
    let log = ObservationLog::new();
    let store_log = log.clone();
    let owner = Origin {
        reducer: ReducerId::of(b"storer-backend"),
        host: HostId::of(b"h"),
    };

    let run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("storer", Bytes::from_static(b"storer"))
        .spawn(SpawnSpec::new("storer", "storer"))
        .deliver(
            "storer",
            Delivered::Message(Message {
                id: ContractId::of(b"go"),
                payload: Bytes::from_static(b"x"),
                from: Origin {
                    reducer: ReducerId::of(b"caller"),
                    host: HostId::of(b"ingress"),
                },
                continuation_token: Bytes::from_static(b"t"),
            }),
        )
        .log(log)
        .run(move |_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"storer"), move || {
                Box::new(Storer {
                    kv: RecordingKvStore::new(
                        InMemoryKvStore::new(),
                        owner,
                        store_log.clone(),
                        tick_clock as fn() -> u64,
                    ),
                    blobs: RecordingBlobStore::new(
                        InMemoryBlobStore::new(),
                        owner,
                        store_log.clone(),
                        tick_clock as fn() -> u64,
                    ),
                })
            });
            store
        });

    // A rich log: at least a spawn, a delivered event, and the KV + blob store calls.
    assert!(
        run.records.len() >= 4,
        "the scenario produces a rich log:\n{}",
        crate::testing::render(&run.records)
    );
    // The whole real log round-trips through the Cadenza-value serializer, byte-for-byte.
    let bytes = crate::testing::serialize_log(&run.records);
    assert_eq!(
        crate::testing::deserialize_log(&bytes),
        Some(run.records.clone()),
        "a real run's observation log must survive serialize_log → deserialize_log intact"
    );
}

/// A checker reducer: on the delivered `check` message it decodes the observation log and passes iff the log
/// contains a spawn record, emitting its judgement on the verdict contract and closing. A stand-in for the
/// reducer-shaped wasm checker the operator's design calls for (§9) — exercised natively here.
struct SpawnPresenceChecker;
#[async_trait::async_trait]
impl Reducer for SpawnPresenceChecker {
    async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
        let verdict = if m.id == check_contract() {
            match decode_check(&m.payload).and_then(|log| deserialize_log(&log)) {
                Some(records) if records.iter().any(|r| matches!(r.entry, Entry::Spawn(_))) => {
                    encode_verdict(true, &[])
                }
                Some(_) => encode_verdict(false, &[Str::from("log has no spawn record")]),
                None => encode_verdict(false, &[Str::from("log did not decode")]),
            }
        } else {
            encode_verdict(false, &[Str::from("unexpected contract")])
        };
        (
            vec![Request {
                id: verdict_contract(),
                payload: verdict,
                continuation_token: Bytes::new(),
                deadline: None,
            }],
            Outcome::Break {
                schema: ContractId::of(b"checked"),
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

#[test]
fn a_checker_reducer_folds_the_delivered_log_and_emits_a_verdict_the_harness_reads() {
    // The end-to-end checker path (§9), natively: a first run produces an observation log; a checker reducer
    // is spawned, delivered that whole log as a `check` message, folds it, and emits a `verdict` request the
    // harness reads back with `verdict_in`. This proves the operator's design — a reducer-shaped checker over
    // the reducer interface — end to end, minus the wasm instantiation.
    fn checker_run(log_to_check: &[super::Record]) -> Run {
        Harness::new("sys")
            .blob("sys", Bytes::from_static(b"sys"))
            .blob("checker", Bytes::from_static(b"checker"))
            .spawn(SpawnSpec::new("checker", "checker"))
            .deliver("checker", check_message(log_to_check))
            .run(|_cas| {
                let mut store = crate::program::testing::Store::new();
                // The checker under test, plus a stand-in system reducer to absorb the routed verdict effect.
                store.register(ProgramHash::of(b"checker"), || {
                    Box::new(SpawnPresenceChecker)
                });
                store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));
                store
            })
    }

    // A real run's log contains spawn records, so the checker passes.
    let main_run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("worker", Bytes::from_static(b"worker"))
        .spawn(SpawnSpec::new("worker", "worker"))
        .run(|_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"worker"), || Box::new(JustClose));
            store
        });
    assert!(
        main_run
            .records
            .iter()
            .any(|r| matches!(r.entry, Entry::Spawn(_))),
        "the main run records a spawn"
    );
    let pass = checker_run(&main_run.records);
    assert_eq!(
        verdict_in(&pass.records),
        Some(CheckOutcome::Pass),
        "the checker passes on a log that contains a spawn:\n{}",
        render(&pass.records)
    );

    // An empty log has no spawn, so the same checker fails, carrying its reason back to the harness.
    let fail = checker_run(&[]);
    assert_eq!(
        verdict_in(&fail.records),
        Some(CheckOutcome::Fail {
            reasons: vec!["log has no spawn record".to_string()],
        }),
        "the checker fails on an empty log:\n{}",
        render(&fail.records)
    );
}

/// A reducer that echoes: on a message it emits one request on the SAME contract carrying the SAME payload,
/// then continues. A native stand-in for the reducer-echo guest, so a checker can assert the echo (§9).
struct Echoer;
#[async_trait::async_trait]
impl Reducer for Echoer {
    async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
        (
            vec![Request {
                id: m.id,
                payload: m.payload.clone(),
                continuation_token: m.continuation_token.clone(),
                deadline: None,
            }],
            Outcome::Continue,
        )
    }
    async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }
    async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }
}

/// A checker reducer that asserts an ECHO in the delivered log: some message delivered on a contract with a
/// payload was answered by a request the same reducer emitted on the same contract with the same payload.
/// This is the shape of the reducer-echo v2 assertion (contract + payload echo), authored natively as a
/// reference; the wasm checker guest asserts the same over a serialized log.
struct EchoAssertChecker;
#[async_trait::async_trait]
impl Reducer for EchoAssertChecker {
    async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
        let verdict = if m.id == check_contract() {
            match decode_check(&m.payload).and_then(|log| deserialize_log(&log)) {
                Some(records) => {
                    let delivered: Vec<(ContractId, Bytes)> = records
                        .iter()
                        .filter_map(|r| match &r.entry {
                            Entry::Event(EventOp::Delivered {
                                kind: EventKind::Message,
                                contract,
                                payload,
                                ..
                            }) => Some((*contract, payload.clone())),
                            _ => None,
                        })
                        .collect();
                    let echoed = |c: &ContractId, p: &Bytes| {
                        records.iter().any(|r| {
                            matches!(&r.entry, Entry::Event(EventOp::Emitted { contract, payload, .. })
                                if contract == c && payload == p)
                        })
                    };
                    if delivered.iter().any(|(c, p)| echoed(c, p)) {
                        encode_verdict(true, &[])
                    } else {
                        encode_verdict(false, &[Str::from("no delivered message was echoed back")])
                    }
                }
                None => encode_verdict(false, &[Str::from("log did not decode")]),
            }
        } else {
            encode_verdict(false, &[Str::from("unexpected contract")])
        };
        (
            vec![Request {
                id: verdict_contract(),
                payload: verdict,
                continuation_token: Bytes::new(),
                deadline: None,
            }],
            Outcome::Break {
                schema: ContractId::of(b"checked"),
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

#[test]
fn a_checker_asserts_an_echo_relationship_in_the_log() {
    // The realistic checker shape (reducer-echo v2): a checker reads the whole log and asserts a delivered
    // message was echoed back as an emitted request on the same contract with the same payload. It passes
    // over an echoer's run and fails over a run with no echo — proving a checker can relate two events in
    // the log, the pattern the wasm checker guest will implement.
    fn checker_run(log_to_check: &[super::Record]) -> Run {
        Harness::new("sys")
            .blob("sys", Bytes::from_static(b"sys"))
            .blob("checker", Bytes::from_static(b"checker"))
            .spawn(SpawnSpec::new("checker", "checker"))
            .deliver("checker", check_message(log_to_check))
            .run(|_cas| {
                let mut store = crate::program::testing::Store::new();
                store.register(ProgramHash::of(b"checker"), || Box::new(EchoAssertChecker));
                store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));
                store
            })
    }

    let deliver_go = || {
        Delivered::Message(Message {
            id: ContractId::of(b"go"),
            payload: Bytes::from_static(b"ping"),
            from: Origin {
                reducer: ReducerId::of(b"caller"),
                host: HostId::of(b"ingress"),
            },
            continuation_token: Bytes::from_static(b"t"),
        })
    };

    // An echoer's run: the delivered (go, "ping") is echoed back as an emitted (go, "ping") → the checker passes.
    let echo_run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("echoer", Bytes::from_static(b"echoer"))
        .spawn(SpawnSpec::new("echoer", "echoer"))
        .deliver("echoer", deliver_go())
        .run(|_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"echoer"), || Box::new(Echoer));
            store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));
            store
        });
    let pass = checker_run(&echo_run.records);
    assert_eq!(
        verdict_in(&pass.records),
        Some(CheckOutcome::Pass),
        "the checker passes when the delivered message was echoed:\n{}",
        render(&echo_run.records)
    );

    // A run that only closes: the delivered message is never echoed → the checker fails with its reason.
    let no_echo_run = Harness::new("sys")
        .blob("sys", Bytes::from_static(b"sys"))
        .blob("mute", Bytes::from_static(b"mute"))
        .spawn(SpawnSpec::new("mute", "mute"))
        .deliver("mute", deliver_go())
        .run(|_cas| {
            let mut store = crate::program::testing::Store::new();
            store.register(ProgramHash::of(b"mute"), || Box::new(JustClose));
            store.register(ProgramHash::of(b"sys"), || Box::new(JustClose));
            store
        });
    let fail = checker_run(&no_echo_run.records);
    assert_eq!(
        verdict_in(&fail.records),
        Some(CheckOutcome::Fail {
            reasons: vec!["no delivered message was echoed back".to_string()],
        }),
        "the checker fails when nothing echoed:\n{}",
        render(&no_echo_run.records)
    );
}
