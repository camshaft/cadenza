//! A minimal wasm-component reducer FIXTURE (§19b/§21c interim, concierge Option A) — a wit-bindgen Rust
//! guest implementing the `cadenza:agent-kernel` reducer world. Its whole job is to prove the component
//! path end-to-end: the kernel loads this as a component, calls `fold.apply`, and gets effect-requests
//! back — exercising `ComponentReducer::apply` against a REAL guest (not just compiling it).
//!
//! DEPENDENCY-FREE: a Rust guest declares no content-addressed component deps, so this fixture needs no
//! §23 dep-resolution/compose — it's the right first end-to-end fixture (proves host machinery + apply
//! without the dep-compose path). A REAL Cadenza reducer (via rcdzc→component, §21c) is the eventual
//! bar; this Rust guest is the interim bring-up.
//!
//! Behavior (deliberately tiny + observable): on an inbound message it (a) records a counter in KV via
//! the `kv` import (proving the guest reads/writes its own KV directly, §4b) and (b) emits ONE Http
//! effect-request with a correlation token; on any other event (a result echoing that token, etc.) it
//! emits nothing. Total: never traps (§17 can't-brick).

wit_bindgen::generate!({
    world: "reducer",
    path: "../../../wit/reducer.wit",
});

use exports::cadenza::agent_kernel::fold::Guest;
use cadenza::agent_kernel::kv;
use cadenza::agent_kernel::types::{ContentType, EffectKind, EffectRequest};

struct Guest0;

impl Guest for Guest0 {
    fn apply(
        content_type: ContentType,
        _payload: Option<Vec<u8>>,
        resumes: Option<Vec<u8>>,
    ) -> Vec<EffectRequest> {
        // A result/timer event (resumes = Some(token)) — the effect it correlates to completed. This
        // tiny fixture just stops (no cascade); a real reducer would look the token up + continue.
        if resumes.is_some() {
            return Vec::new();
        }
        // Inbound message → bump a KV counter (direct kv import, §4b) and request one Http effect.
        if content_type.family == "message" {
            let prev = kv::get(b"count").and_then(|v| v.first().copied()).unwrap_or(0);
            kv::put(b"count", &[prev.wrapping_add(1)]);
            return vec![EffectRequest {
                kind: EffectKind::Http,
                target: "https://ok.host/x".to_string(),
                payload: None,
                // The guest's own continuation token (the single resume mechanism): echoed back on the
                // result event so the guest can resume. Here just a marker proving round-trip.
                correlation: Some(b"step-1".to_vec()),
                // This fixture emits a built-in family (http), so it does NOT use the register-by-string
                // family override — `None` derives the family from `kind` as before.
                family: None,
            }];
        }
        Vec::new()
    }
}

export!(Guest0);
