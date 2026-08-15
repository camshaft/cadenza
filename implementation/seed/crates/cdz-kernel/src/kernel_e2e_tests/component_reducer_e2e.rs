//! End-to-end test of the component-model reducer path (§19b/§21c interim): load a REAL wasm-component
//! reducer (the wit-bindgen Rust guest fixture, concierge Option A), drive it through
//! `ComponentReducer::apply`, and assert the fold behaved — the effects it returned AND the KV it
//! mutated via the `kv` host import (§4b). This is the first time `apply` runs against a real guest
//! (not just compiles), proving the reducer INTERFACE + host wiring actually work through the component
//! boundary — the operator's §21c bar (a real Cadenza reducer is the eventual target; this Rust guest
//! is the interim bring-up that proves the machinery).
//!
//! The guest component is built from `tests/fixtures/reducer-guest/` (a wit-bindgen guest) — no longer a
//! committed binary (v-nix N2: nix `packages.reducer-guest` builds it content-addressed). These tests
//! read its bytes from the `REDUCER_GUEST_COMPONENT` env path (see `guest_bytes`); the cdz-kernel CI job
//! builds it from source, validates it's a component, and exports the path. When the env is UNSET (a bare
//! local `cargo test` with no wasm build) the tests OPTIONAL-SKIP cleanly. To run locally:
//!   cd tests/fixtures/reducer-guest && cargo build --target wasm32-unknown-unknown --release
//!   wasm-tools component new target/wasm32-unknown-unknown/release/reducer_guest.wasm -o /tmp/rg.wasm
//!   REDUCER_GUEST_COMPONENT=/tmp/rg.wasm cargo test  (or `nix build .#reducer-guest`)

use crate::event::ContentType;
use crate::kv::Kv;
use crate::wasm_host::{ComponentError, ComponentReducer};

/// The reducer-guest component bytes, read from the `REDUCER_GUEST_COMPONENT` env path (the nix-built
/// `packages.reducer-guest`; the cdz-kernel CI job exports it). Returns `None` when the env is UNSET so a
/// bare `cargo test -p cdz-kernel` (no wasm build) SKIPS these e2e tests cleanly instead of failing — the
/// same optional-skip contract as the Cedar guest (v-nix N2, agreed). A path that IS set but unreadable
/// PANICS (a broken path in CI must fail loud, not silently skip).
fn guest_bytes() -> Option<Vec<u8>> {
    let p = std::env::var("REDUCER_GUEST_COMPONENT").ok()?;
    Some(
        std::fs::read(&p)
            .unwrap_or_else(|e| panic!("REDUCER_GUEST_COMPONENT={p:?} is set but unreadable: {e}")),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn real_guest_component_folds_through_apply_end_to_end() {
    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP real_guest_component_folds_through_apply_end_to_end: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    let reducer = match ComponentReducer::from_component_bytes(&guest) {
        Ok(r) => r,
        Err(e) => panic!("the guest fixture must be a valid reducer component: {e:?}"),
    };
    // The Rust guest is dependency-free (no content-addressed component imports), so no §23 compose
    // needed — the kernel resolves declared deps generically by hash, and this guest declares none.
    assert!(
        reducer.deps().is_empty(),
        "the Rust guest declares no component dependencies"
    );

    // Fold an inbound "message" event through the REAL guest.
    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    let (effects, kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .expect("apply drives the guest without trapping");

    // The guest requested exactly one Http effect, with its own correlation token — proving the
    // effect-request round-trips across the component boundary intact.
    assert_eq!(effects.len(), 1);
    // Under the bytes boundary the guest's `kind` crosses as the family STRING (parse_effect_list →
    // new_with_family), and the kernel `Effect` is `{request, token}` — so assert the family/target/token
    // rather than a WIT `EffectKind` enum + bare fields.
    assert!(effects[0]
        .request
        .is_builtin_kind(crate::effect::EffectKind::Http));
    assert_eq!(
        effects[0].request.target_str().unwrap(),
        "https://ok.host/x"
    );
    assert_eq!(effects[0].token.as_deref(), Some(&b"step-1"[..]));

    // The guest wrote its KV counter via the `kv` HOST IMPORT (§4b: the reducer reads/writes its own
    // KV directly during the fold) — and the mutated KV came back out for the host to persist (§4).
    assert_eq!(kv.get(b"count").as_deref(), Some(&[1u8][..]));

    // A result event (resumes = the echoed correlation token) → the guest emits nothing (it stops).
    let ct2 = ContentType {
        family: "effect-result".into(),
        version: 1,
    };
    let (more, _kv) = reducer
        .apply(Kv::new(), ct2, None, Some(b"step-1".to_vec()))
        .expect("apply on a result event");
    assert!(
        more.is_empty(),
        "the guest emits no further effects on the result event"
    );
}

/// Operator wasmtime-instance-pooling perf directive: a dependency-free fold-exporting reducer is
/// PRE-INSTANTIATED once at construction (cached `ReducerPre`), so every fold skips the link/type-check
/// and just `.instantiate(store)`s. The real guest declares no deps + exports `fold`, so it takes the
/// fast path; a fold still works identically through it (behavior unchanged — the cache is a perf lever,
/// not a semantic one). True Instance-reuse across folds is unsafe (Instance is Store-bound); caching
/// the pre-instantiation is the safe form.
#[tokio::test(flavor = "current_thread")]
async fn dependency_free_reducer_takes_the_cached_instance_pre_fast_path() {
    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP dependency_free_reducer_takes_the_cached_instance_pre_fast_path: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    let reducer = ComponentReducer::from_component_bytes(&guest).expect("valid reducer component");
    assert!(
        reducer.uses_cached_instance_pre(),
        "a dep-free fold-exporting reducer must cache its ReducerPre (the per-fold fast path)"
    );
    // The fast path still folds correctly (same result as the slow path would give).
    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    let (effects, kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .expect("cached-pre fold works");
    assert_eq!(effects.len(), 1);
    assert_eq!(kv.get(b"count").as_deref(), Some(&[1u8][..]));
}

/// §22d fuel bound: a fold that exceeds its fuel budget is aborted with a DISTINCT
/// `ComponentError::FuelExhausted` (not a semantic `Trap`) — the runaway-guest DoS guard (Copilot
/// PR#1009). We can't easily commit a forever-looping component fixture, so we starve the REAL guest:
/// a 1-fuel budget is smaller than even this tiny fold's instruction cost, so it exhausts mid-`apply`.
/// This proves (a) the budget is charged against the fold, and (b) exhaustion is classified correctly.
#[tokio::test(flavor = "current_thread")]
async fn a_fold_that_exceeds_its_fuel_budget_is_aborted_as_fuel_exhausted() {
    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP a_fold_that_exceeds_its_fuel_budget_is_aborted_as_fuel_exhausted: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    let reducer = ComponentReducer::from_component_bytes(&guest)
        .expect("valid reducer component")
        // A budget of 1 fuel unit: instantiation gets full headroom (reset internally), but the fold
        // itself executes real instructions, so 1 unit can't cover it → OutOfFuel.
        .with_fuel_budget(1);
    assert_eq!(reducer.fuel_budget(), 1);

    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    // `apply` returns the KV alongside the error (so `fold` restores it without cloning); the
    // fuel-exhaustion classification is unchanged.
    match reducer.apply(Kv::new(), ct, Some(b"hello".to_vec()), None) {
        Err((ComponentError::FuelExhausted { budget }, _kv)) => {
            assert_eq!(
                budget, 1,
                "the reported budget is the one that was exhausted"
            );
        }
        Err((other, _kv)) => panic!("expected FuelExhausted, got {other:?}"),
        Ok(_) => panic!("a 1-fuel budget must not let the fold complete"),
    }
}

/// The counterpart: the DEFAULT budget is generous enough that a legitimate fold completes normally
/// (guards against a budget so tight it breaks real reducers — the default must not false-positive).
#[tokio::test(flavor = "current_thread")]
async fn a_normal_fold_completes_within_the_default_fuel_budget() {
    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP a_normal_fold_completes_within_the_default_fuel_budget: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    let reducer = ComponentReducer::from_component_bytes(&guest).expect("valid reducer component");
    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    let (effects, _kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .expect("a normal fold completes within the default fuel budget");
    assert_eq!(
        effects.len(),
        1,
        "the normal fold still produces its effect"
    );
}

/// §19b/§19e slice 2b-ii: the REAL wasm guest drives the KERNEL LOOP via `impl Reducer for
/// ComponentReducer`. Deliver an inbound "message" to a Session whose reducer IS the component; assert
/// the kernel dispatched the guest's Http effect AND recorded the guest's correlation token in the
/// Dispatched frame (§19e: emit token → Dispatched). This is the operator's §19b bar — a wasm reducer
/// folding on the same loop as a Rust one — proven end to end through the kernel.
#[tokio::test(flavor = "current_thread")]
async fn the_wasm_guest_drives_the_kernel_loop_and_its_token_reaches_the_dispatched_frame() {
    use crate::authz::Authorizer;
    use crate::effect::{Capability, EffectKind as KKind, Payload, ResourcePredicate};
    use crate::event::{ContentType as KContentType, EventBody};
    use crate::executor::RecordingExecutor;
    use crate::hash::Hash;
    use crate::kernel::Session;

    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP the_wasm_guest_drives_the_kernel_loop_and_its_token_reaches_the_dispatched_frame: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    let mut reducer =
        ComponentReducer::from_component_bytes(&guest).expect("valid reducer component");
    // Grant the guest's Http target (SEC-F1) so the effect isn't denied.
    let authz = Authorizer::new(vec![Capability {
        kind: KKind::Http,
        predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
    }]);
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"wasm-reducer-v1"), Hash::of(b"test-spawn-nonce"));
    let captured = crate::test_log_source::attach_recording_sink(&mut session);

    // Deliver an inbound "message" — the guest emits one Http effect with correlation "step-1".
    session
        .deliver(
            EventBody::Inbound {
                content_type: KContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"hello".to_vec().into()),
            },
            None,
            &mut reducer,
            &authz,
            &mut exec,
        )
        .await
        .unwrap();

    // The kernel routed the guest's Http effect to the executor.
    assert_eq!(
        exec.seen.len(),
        1,
        "the guest's one Http effect was dispatched"
    );
    assert_eq!(exec.seen[0].0.target_str().unwrap(), "https://ok.host/x");
    // And the guest's continuation token rode into the durable Dispatched frame (§19e: emit→Dispatched).
    let dispatched_token = crate::test_log_source::replay_input(&captured)
        .iter()
        .find_map(|e| match &e.body {
            EventBody::Dispatched { token, .. } => Some(token.clone()),
            _ => None,
        });
    assert_eq!(
        dispatched_token,
        Some(Some(b"step-1".to_vec())),
        "the wasm guest's correlation token must reach the Dispatched frame through the adapter"
    );
}

/// PR#1076/#1150 perf + error-atomicity through the real `Reducer::fold` path. `fold` moves the KV
/// into the guest WITHOUT cloning (a `BTreeMap` deep-copy every event would be O(KV size)), and the
/// guest's writes go through a TRANSACTIONAL overlay that commits on success / discards on failure.
/// This test pins BOTH ends via the real guest:
/// - SUCCESS: a normal fold commits the guest's `count` write AND leaves pre-existing keys intact
///   (commit MERGES into the base, it doesn't replace it).
/// - FAILURE: a fuel-starved fold leaves a pre-populated KV byte-for-byte intact (restored, not
///   emptied — guarding the `mem::take`). (The write-then-discard atomicity itself is pinned at the
///   host-overlay unit level in `host_kv_writes_are_discarded_without_commit`, which is deterministic;
///   the fuel path can't reliably write-then-trap without brittle budget calibration.)
#[tokio::test(flavor = "current_thread")]
async fn fold_commits_on_success_and_leaves_the_kv_intact_on_failure() {
    use crate::event::{ContentType as KContentType, EventBody};
    use crate::reducer::Reducer;

    // An inner async fn (not a closure — a closure returning a future that borrows its `&reducer` arg
    // trips the borrow checker; an `async fn` scopes the borrow to its own await cleanly).
    async fn inbound(kv: &mut Kv, reducer: &mut ComponentReducer) -> crate::reducer::FoldOutput {
        reducer
            .fold(
                &crate::event::Event {
                    seq: 1,
                    cause: None,
                    body: EventBody::Inbound {
                        content_type: KContentType {
                            family: "message".into(),
                            version: 1,
                        },
                        payload: crate::effect::Payload::Inline(b"hello".to_vec().into()),
                    },
                },
                kv,
            )
            .await
    }

    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP fold_commits_on_success_and_leaves_the_kv_intact_on_failure: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    // SUCCESS path: default budget. Pre-populate an unrelated key; the fold bumps "count".
    let mut ok_reducer =
        ComponentReducer::from_component_bytes(&guest).expect("valid reducer component");
    let mut kv = Kv::new();
    kv.put(b"unrelated".to_vec(), b"keep".to_vec());
    let out = inbound(&mut kv, &mut ok_reducer).await;
    assert_eq!(out.effects.len(), 1, "the successful fold emits its effect");
    // Commit MERGED: the guest's new "count" AND the pre-existing "unrelated" both present.
    assert_eq!(
        kv.get(b"count").as_deref(),
        Some(&[1u8][..]),
        "guest write committed"
    );
    assert_eq!(
        kv.get(b"unrelated").as_deref(),
        Some(&b"keep"[..]),
        "commit merges into the base — pre-existing keys survive"
    );

    // FAILURE path: 1-fuel budget → the fold fails. A pre-populated KV must be restored intact, not
    // left empty by the `mem::take`.
    let mut fail_reducer = ComponentReducer::from_component_bytes(&guest)
        .expect("valid reducer component")
        .with_fuel_budget(1);
    let mut kv2 = Kv::new();
    kv2.put(b"a".to_vec(), b"1".to_vec());
    kv2.put(b"b".to_vec(), b"2".to_vec());
    let out2 = inbound(&mut kv2, &mut fail_reducer).await;
    assert!(out2.effects.is_empty(), "a failed fold emits no effects");
    // The failure is CAPTURED (not a silent empty fold) — error-resilience direction.
    assert!(
        out2.failure.is_some(),
        "a failed fold surfaces a failure reason, not a silent none"
    );
    assert_eq!(kv2.len(), 2, "a failed fold must not shrink the KV");
    assert_eq!(kv2.get(b"a").as_deref(), Some(&b"1"[..]));
    assert_eq!(kv2.get(b"b").as_deref(), Some(&b"2"[..]));
}

/// Error-resilience / supervision (§17): a fold that FAILS (wasm guest trap / fuel-exhaustion) is
/// CAPTURED as a first-class `FoldFailed` LOG event — driven through the real kernel loop — rather than
/// vanishing into a silent empty fold ("errors into the void"). A supervisor reading the log sees the
/// failure + which event caused it. The session doesn't die: the loop continues (§17 can't-brick).
#[tokio::test(flavor = "current_thread")]
async fn a_failed_wasm_fold_records_a_foldfailed_event_on_the_log() {
    use crate::authz::Authorizer;
    use crate::effect::{Capability, EffectKind as KKind, Payload, ResourcePredicate};
    use crate::event::{ContentType as KContentType, EventBody};
    use crate::executor::RecordingExecutor;
    use crate::hash::Hash;
    use crate::kernel::Session;

    let Some(guest) = guest_bytes() else {
        eprintln!("SKIP a_failed_wasm_fold_records_a_foldfailed_event_on_the_log: REDUCER_GUEST_COMPONENT unset");
        return;
    };
    // A 1-fuel budget guarantees the guest fold traps (OutOfFuel) mid-apply.
    let mut reducer = ComponentReducer::from_component_bytes(&guest)
        .expect("valid reducer component")
        .with_fuel_budget(1);
    let authz = Authorizer::new(vec![Capability {
        kind: KKind::Http,
        predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
    }]);
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"foldfail-v1"), Hash::of(b"test-spawn-nonce"));
    let captured = crate::test_log_source::attach_recording_sink(&mut session);

    // Deliver an inbound message — the guest's fold traps on 1 fuel. deliver() must NOT panic (§17).
    session
        .deliver(
            EventBody::Inbound {
                content_type: KContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"hello".to_vec().into()),
            },
            None,
            &mut reducer,
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver does not error on a trapped fold");

    // A FoldFailed event is on the durable log (the failure was CAPTURED, not swallowed) — with a reason.
    let fold_failed = crate::test_log_source::replay_input(&captured)
        .iter()
        .find_map(|e| match &e.body {
            EventBody::FoldFailed { reason, .. } => Some(reason.clone()),
            _ => None,
        });
    assert!(
        fold_failed.is_some(),
        "a trapped wasm fold must record a FoldFailed event, not vanish into a silent empty fold"
    );
    assert!(
        !fold_failed.unwrap().is_empty(),
        "the FoldFailed event carries a non-empty failure reason for a supervisor to read"
    );
    // The guest never dispatched an effect (it trapped before returning), so the executor saw nothing.
    assert_eq!(exec.seen.len(), 0);
}

// The sync dep-path guard (parity with the async twin's #2253 fix): a ComponentReducer that DECLARES a
// `+<hash>` dep builds fine, deps() reports it, and folding through the STRUCTURAL `apply` WITHOUT
// attaching the resolved deps fails with an ACTIONABLE error naming the builders — not an opaque wasmtime
// linker "missing imports" error. (Self-contained synthetic WAT: no fold world needed — the no-attach
// guard fires before instantiation. A full green dep-bearing fold is proven by reducer_cadenza_b1_e2e.)
#[tokio::test(flavor = "current_thread")]
async fn sync_reducer_declaring_a_dep_folds_loud_without_attach() {
    let hex = "b".repeat(64);
    let src = format!(
        r#"(component
             (import "cadenza:runtime/heap@0.0.0+{hex}" (instance
               (export "box-int" (func (param "v" s64) (result u32))))))"#
    );
    let bytes = wat::parse_str(&src).expect("assemble dep-declaring component");

    let reducer =
        ComponentReducer::from_component_bytes(&bytes).expect("dep-bearing sync reducer builds");
    assert_eq!(reducer.deps().len(), 1, "declares one +hash dep");

    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    match reducer.apply(Kv::new(), ct, None, None) {
        Err((ComponentError::Instantiate(msg), _kv)) => assert!(
            msg.contains("with_resolved_deps") && msg.contains("declares"),
            "expected an actionable no-deps-attached error naming the builders, got {msg:?}"
        ),
        other => {
            panic!("expected an actionable Instantiate error for unattached deps, got {other:?}")
        }
    }
}

/// The `reducer_reify.cdz` component bytes (v-nix `packages.reify-probe-reducer`, mirroring the pure/kv
/// precompile drvs), read from `REIFY_PROBE_REDUCER_COMPONENT`; `None` (skip) when unset — same optional-skip
/// contract as the guest, so a bare `cargo test -p cdz-kernel` stays green. A set-but-unreadable path panics.
fn reify_probe_bytes() -> Option<Vec<u8>> {
    let p = std::env::var("REIFY_PROBE_REDUCER_COMPONENT").ok()?;
    Some(std::fs::read(&p).unwrap_or_else(|e| {
        panic!("REIFY_PROBE_REDUCER_COMPONENT={p:?} is set but unreadable: {e}")
    }))
}

/// Schema-hash phase-1a REIFY e2e (the phase-2-window gate): `reducer_reify.cdz` PERFORMS a target-free
/// single-Bytes-arg world-effect (`host Probe in ([Probe.fire(p)])`), so rcdzc REIFIES it via
/// `reify_effect_to_tuple` (NOT a sync HostCall — Probe is not a world import) to the 3-field no-target record
/// `{ correlation=None, kind="effect/probe", payload=Some(p) }`, which the kernel `parse_effect_request`
/// (target-free-tolerant, 14b7c9885) parses back. Proves the phase-1a triad end-to-end, non-vacuously:
/// rcdzc reify emit → wasm component → kernel parse-tolerance. (SKIPs cleanly without the precompiled component.)
#[tokio::test(flavor = "current_thread")]
async fn reify_probe_reducer_reifies_a_target_free_perform_end_to_end() {
    let Some(bytes) = reify_probe_bytes() else {
        eprintln!(
            "SKIP reify_probe_reducer_reifies_a_target_free_perform_end_to_end: REIFY_PROBE_REDUCER_COMPONENT unset"
        );
        return;
    };
    // reducer_reify TARGETS the pure-fold world, so it imports `cadenza:runtime/heap` (value-encode reifies
    // the perform's args into the payload column) — its declared dep resolves from CDZ_STORE exactly as the
    // pure/kv genesis reducers' do. A reducer set but no store = FAIL LOUD (a silent skip would hide broken
    // wiring); an empty CDZ_STORE counts as unset.
    let store_dir = std::env::var("CDZ_STORE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "REIFY_PROBE_REDUCER_COMPONENT is set but CDZ_STORE is not — the world-driven reify reducer \
                 imports cadenza:runtime/heap, which resolves from the store"
            )
        });
    let reducer = ComponentReducer::from_component_bytes(&bytes)
        .expect("reducer_reify must be a valid reducer component");
    // Resolve the declared value-heap dep from CDZ_STORE (get_by_hash — the content-addressed reader the fold
    // uses) + attach the store so the §23 compose resolves the runtime's OWN transitive bare import (nfc).
    // Identical to the pure-genesis reducer's dep-resolve path.
    let store = crate::component_store::ComponentStore::open(&store_dir);
    let deps = reducer.deps().to_vec();
    assert!(
        !deps.is_empty(),
        "reducer_reify imports cadenza:runtime/heap (value-encode) so it must declare a dep"
    );
    let mut resolved = Vec::with_capacity(deps.len());
    for dep in &deps {
        let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
            panic!(
                "CDZ_STORE={store_dir:?} could not resolve reify reducer dep {:?} (hash {}): {e:?}",
                dep.import_name,
                dep.hash.to_hex()
            )
        });
        resolved.push((dep.clone(), dep_bytes));
    }
    let reducer = reducer
        .with_resolved_deps(resolved)
        .with_component_store(store);

    // A payloaded "message" event → the reducer performs Probe.fire(payload), reified to ONE effect.
    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    let (effects, _kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .expect("apply drives the reify reducer without trapping");
    assert_eq!(effects.len(), 1, "one reified Probe.fire perform");

    // The reified 3-field record: kind = the userspace family string `effect/<declared-effect-name>` — the
    // effect type name VERBATIM (`Probe`, capitalized per Cadenza's type/effect naming), NOT lowercased. (The
    // kind is the transitional phase-1a human-facing family tag; phase-2 replaces it with the schema-hash, so
    // its casing is cosmetic — but the reify derives it from the declared name as-written.) payload =
    // Some(Inline(the single Bytes arg)); target-free (no target column, no @resource, no R2).
    assert_eq!(
        effects[0].request.string_family(),
        "effect/Probe",
        "reified kind = effect/<declared-name> (verbatim) for the performed Probe effect"
    );
    match &effects[0].request.payload {
        Some(crate::effect::Payload::Inline(b)) => {
            assert_eq!(
                &b[..],
                b"hello",
                "reify's single-Bytes arg → the payload column"
            )
        }
        other => {
            panic!("reified perform must carry the arg as payload=Some(Inline), got {other:?}")
        }
    }
    assert!(
        effects[0].token.is_none(),
        "a target-free perform carries no reducer correlation token"
    );

    // A payload-free event → the reducer performs nothing → no effects reified.
    let ct2 = ContentType {
        family: "message".into(),
        version: 1,
    };
    let (none_effects, _) = reducer
        .apply(Kv::new(), ct2, None, None)
        .expect("apply on a payload-free event");
    assert!(
        none_effects.is_empty(),
        "a payload-free event reifies no performs"
    );
}
