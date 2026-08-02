//! End-to-end test of the component-model reducer path (§19b/§21c interim): load a REAL wasm-component
//! reducer (the wit-bindgen Rust guest fixture, concierge Option A), drive it through
//! `ComponentReducer::apply`, and assert the fold behaved — the effects it returned AND the KV it
//! mutated via the `kv` host import (§4b). This is the first time `apply` runs against a real guest
//! (not just compiles), proving the reducer INTERFACE + host wiring actually work through the component
//! boundary — the operator's §21c bar (a real Cadenza reducer is the eventual target; this Rust guest
//! is the interim bring-up that proves the machinery).
//!
//! The fixture (`tests/fixtures/reducer_guest.component.wasm`) is a committed build artifact of
//! `tests/fixtures/reducer-guest/` (a wit-bindgen guest). Regenerate it when the WIT or guest changes:
//!   cd tests/fixtures/reducer-guest
//!   cargo build --target wasm32-unknown-unknown --release
//!   wasm-tools component new target/wasm32-unknown-unknown/release/reducer_guest.wasm \
//!       -o ../reducer_guest.component.wasm
//! (CI rebuilds it from source + validates the result is a valid component so a stale fixture is
//! caught — NOT a byte-diff, since the guest's deps/toolchain aren't lock-pinned; see the cdz-kernel CI
//! job. This test loading + folding the committed .wasm is what proves its correctness.)

use cdz_kernel::kv::Kv;
use cdz_kernel::wasm_host::{ComponentError, ComponentReducer, ContentType, EffectKind};

const GUEST: &[u8] = include_bytes!("fixtures/reducer_guest.component.wasm");

#[test]
fn real_guest_component_folds_through_apply_end_to_end() {
    let reducer = match ComponentReducer::from_component_bytes(GUEST) {
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
        family: "message".to_string(),
        version: 1,
    };
    let (effects, kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .expect("apply drives the guest without trapping");

    // The guest requested exactly one Http effect, with its own correlation token — proving the
    // effect-request round-trips across the component boundary intact.
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].kind, EffectKind::Http);
    assert_eq!(effects[0].target, "https://ok.host/x");
    assert_eq!(effects[0].correlation.as_deref(), Some(&b"step-1"[..]));

    // The guest wrote its KV counter via the `kv` HOST IMPORT (§4b: the reducer reads/writes its own
    // KV directly during the fold) — and the mutated KV came back out for the host to persist (§4).
    assert_eq!(kv.get(b"count"), Some(&[1u8][..]));

    // A result event (resumes = the echoed correlation token) → the guest emits nothing (it stops).
    let ct2 = ContentType {
        family: "effect-result".to_string(),
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

/// §22d fuel bound: a fold that exceeds its fuel budget is aborted with a DISTINCT
/// `ComponentError::FuelExhausted` (not a semantic `Trap`) — the runaway-guest DoS guard (Copilot
/// PR#1009). We can't easily commit a forever-looping component fixture, so we starve the REAL guest:
/// a 1-fuel budget is smaller than even this tiny fold's instruction cost, so it exhausts mid-`apply`.
/// This proves (a) the budget is charged against the fold, and (b) exhaustion is classified correctly.
#[test]
fn a_fold_that_exceeds_its_fuel_budget_is_aborted_as_fuel_exhausted() {
    let reducer = ComponentReducer::from_component_bytes(GUEST)
        .expect("valid reducer component")
        // A budget of 1 fuel unit: instantiation gets full headroom (reset internally), but the fold
        // itself executes real instructions, so 1 unit can't cover it → OutOfFuel.
        .with_fuel_budget(1);
    assert_eq!(reducer.fuel_budget(), 1);

    let ct = ContentType {
        family: "message".to_string(),
        version: 1,
    };
    // `apply` returns the KV alongside the error now (so `fold` can restore it without cloning); the
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
#[test]
fn a_normal_fold_completes_within_the_default_fuel_budget() {
    let reducer = ComponentReducer::from_component_bytes(GUEST).expect("valid reducer component");
    let ct = ContentType {
        family: "message".to_string(),
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
#[test]
fn the_wasm_guest_drives_the_kernel_loop_and_its_token_reaches_the_dispatched_frame() {
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, EffectKind as KKind, Payload, ResourcePredicate};
    use cdz_kernel::event::{ContentType as KContentType, EventBody};
    use cdz_kernel::executor::RecordingExecutor;
    use cdz_kernel::hash::Hash;
    use cdz_kernel::kernel::Session;

    let reducer = ComponentReducer::from_component_bytes(GUEST).expect("valid reducer component");
    // Grant the guest's Http target (SEC-F1) so the effect isn't denied.
    let authz = Authorizer::new(vec![Capability {
        kind: KKind::Http,
        predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
    }]);
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"wasm-reducer-v1"));

    // Deliver an inbound "message" — the guest emits one Http effect with correlation "step-1".
    session
        .deliver(
            EventBody::Inbound {
                content_type: KContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"hello".to_vec()),
            },
            None,
            &reducer,
            &authz,
            &mut exec,
        )
        .unwrap();

    // The kernel routed the guest's Http effect to the executor.
    assert_eq!(
        exec.seen.len(),
        1,
        "the guest's one Http effect was dispatched"
    );
    assert_eq!(exec.seen[0].0.target, "https://ok.host/x");
    // And the guest's continuation token rode into the durable Dispatched frame (§19e: emit→Dispatched).
    let dispatched_token = session.log().iter().find_map(|e| match &e.body {
        EventBody::Dispatched { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        dispatched_token,
        Some(Some(b"step-1".to_vec())),
        "the wasm guest's correlation token must reach the Dispatched frame through the adapter"
    );
}

/// PR#1076 perf + correctness: `fold` moves the session KV into the guest WITHOUT cloning (a `BTreeMap`
/// deep-copy every event would be O(KV size)), and on a FAILED fold it RESTORES the pre-fold KV rather
/// than leaving the `mem::take`-d slot empty. This pins the error-atomicity guarantee: a trapped /
/// fuel-exhausted fold leaves the session's derived state exactly as it was — never half-cleared.
#[test]
fn a_failed_fold_leaves_the_session_kv_untouched_not_emptied() {
    use cdz_kernel::event::{ContentType as KContentType, EventBody};
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::Reducer;

    // A 1-fuel budget guarantees the fold exhausts mid-`apply` (as in the fuel test above).
    let reducer = ComponentReducer::from_component_bytes(GUEST)
        .expect("valid reducer component")
        .with_fuel_budget(1);

    // Pre-populate the KV with state the fold must NOT lose on failure.
    let mut kv = Kv::new();
    kv.put(b"pre-existing".to_vec(), b"keep-me".to_vec());
    kv.put(b"another".to_vec(), b"also-keep".to_vec());
    let before_len = kv.len();

    // Fold an inbound event: the guest starts, exhausts its 1-fuel budget, and the fold fails.
    let out = reducer.fold(
        &cdz_kernel::event::Event {
            seq: 1,
            cause: None,
            body: EventBody::Inbound {
                content_type: KContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: cdz_kernel::effect::Payload::Inline(b"hello".to_vec()),
            },
        },
        &mut kv,
    );

    // Fail-safe (§17): a failed fold emits NO effects...
    assert!(
        out.effects.is_empty(),
        "a fuel-exhausted fold emits no effects (fail-safe)"
    );
    // ...and CRUCIALLY the KV is untouched — the pre-existing entries survive, NOT an empty map (which
    // is what a `mem::take` with no restore-on-error would leave behind).
    assert_eq!(kv.len(), before_len, "a failed fold must not shrink the KV");
    assert_eq!(kv.get(b"pre-existing"), Some(&b"keep-me"[..]));
    assert_eq!(kv.get(b"another"), Some(&b"also-keep"[..]));
}
