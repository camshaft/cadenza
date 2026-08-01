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
//! (CI rebuilds + diffs it so a stale fixture is caught — see the cdz-kernel CI job.)

use cdz_kernel::kv::Kv;
use cdz_kernel::wasm_host::{ComponentReducer, ContentType, EffectKind};

const GUEST: &[u8] = include_bytes!("fixtures/reducer_guest.component.wasm");

#[test]
fn real_guest_component_folds_through_apply_end_to_end() {
    let reducer = match ComponentReducer::from_component_bytes(GUEST) {
        Ok(r) => r,
        Err(e) => panic!("the guest fixture must be a valid reducer component: {e:?}"),
    };
    // The Rust guest is runtime-free (no Cadenza value-heap runtime import), so no §21b compose needed.
    assert!(
        reducer.runtime_req().is_none(),
        "the Rust guest imports no runtime"
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
