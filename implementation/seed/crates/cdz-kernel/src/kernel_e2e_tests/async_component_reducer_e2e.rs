//! End-to-end async fold: drive the REAL reducer-guest component through
//! [`AsyncComponentReducer`] (the cooperative-gas-yield path, operator all-async directive) and assert it
//! behaves identically to the sync `ComponentReducer` — same effects, same KV mutation — but via the async
//! engine (`async_support` + `fuel_async_yield_interval`) and an `.await`ed `call_apply`. This is the
//! async twin of `component_reducer_e2e.rs`'s first case: proof that the async apply round-trips the guest
//! effect-request + the `kv` host import + the mutated KV across the component boundary, on a real
//! single-threaded (current-thread) executor (the runtime the kernel is designed for — no Send).

use crate::event::ContentType;
use crate::kv::Kv;
use crate::reducer::Reducer;
use crate::wasm_host::{AsyncComponentReducer, ComponentError};

// The SAME guest the sync e2e uses (via REDUCER_GUEST_COMPONENT) — a dependency-free wit-bindgen reducer
// that, on an inbound "message", requests one Http effect + increments a KV counter.
/// The reducer-guest component bytes, from the `REDUCER_GUEST_COMPONENT` env path (nix-built
/// `packages.reducer-guest`; the cdz-kernel CI job exports it). `None` when UNSET so a bare
/// `cargo test -p cdz-kernel` SKIPS these e2e tests cleanly (optional-skip, same as the Cedar guest —
/// v-nix N2). A set-but-unreadable path PANICS (a broken CI path must fail loud, not skip).
fn guest_bytes() -> Option<Vec<u8>> {
    let p = std::env::var("REDUCER_GUEST_COMPONENT").ok()?;
    Some(
        std::fs::read(&p)
            .unwrap_or_else(|e| panic!("REDUCER_GUEST_COMPONENT={p:?} is set but unreadable: {e}")),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn real_guest_folds_through_async_apply_end_to_end() {
    let Some(guest) = guest_bytes() else {
        eprintln!(
            "SKIP real_guest_folds_through_async_apply_end_to_end: REDUCER_GUEST_COMPONENT unset"
        );
        return;
    };
    let reducer = match AsyncComponentReducer::from_component_bytes(&guest) {
        Ok(r) => r,
        Err(e) => panic!("the guest fixture must be a valid async reducer component: {e:?}"),
    };

    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    let (effects, kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .await
        .expect("async apply drives the guest without trapping");

    // Identical observable behavior to the sync path: exactly one Http effect with the guest's own
    // correlation token, round-tripped across the component boundary.
    assert_eq!(effects.len(), 1);
    // Bytes boundary: kind crosses as the family STRING, kernel `Effect` is `{request, token}`.
    assert!(effects[0]
        .request
        .is_builtin_kind(crate::effect::EffectKind::Http));
    assert_eq!(
        effects[0].request.target_str().unwrap(),
        "https://ok.host/x"
    );
    assert_eq!(effects[0].token.as_deref(), Some(&b"step-1"[..]));

    // The guest wrote its KV counter via the `kv` host import, and the mutated KV came back out.
    assert_eq!(kv.get(b"count").as_deref(), Some(&[1u8][..]));
}

#[tokio::test(flavor = "current_thread")]
async fn async_reducer_drives_via_the_async_reducer_trait() {
    // Drive through the `Reducer` trait (what the async kernel loop will call) via a `&dyn` reference
    // — proving the native `impl Reducer for AsyncComponentReducer` is object-safe + folds a real
    // Inbound event, mutating the passed-in KV (the fold contract).
    let Some(guest) = guest_bytes() else {
        eprintln!(
            "SKIP async_reducer_drives_via_the_async_reducer_trait: REDUCER_GUEST_COMPONENT unset"
        );
        return;
    };
    let mut reducer =
        AsyncComponentReducer::from_component_bytes(&guest).expect("valid async reducer component");
    let dyn_reducer: &mut dyn Reducer = &mut reducer;

    let event = crate::event::Event {
        seq: 0,
        cause: None,
        body: crate::event::EventBody::Inbound {
            content_type: crate::event::ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: crate::effect::Payload::Inline(b"hello".to_vec().into()),
        },
    };
    let mut kv = Kv::new();
    let out = dyn_reducer.fold(&event, &mut kv).await;

    // The fold succeeded (no failure), emitted the guest's one Http effect, and mutated the KV in place.
    // `fold` yields KERNEL effect types (crate::effect), not the guest-wire `wasm_host::EffectKind`.
    assert!(
        out.failure.is_none(),
        "fold should not fail: {:?}",
        out.failure
    );
    assert_eq!(out.effects.len(), 1);
    assert!(out.effects[0]
        .request
        .is_builtin_kind(crate::effect::EffectKind::Http));
    assert_eq!(kv.get(b"count").as_deref(), Some(&[1u8][..]));
}

#[tokio::test(flavor = "current_thread")]
async fn async_reducer_from_component_bytes_rejects_garbage() {
    // Garbage bytes are an InvalidComponent at construction. (The async path now COMPOSES §23 component
    // deps per-fold like the sync ComponentReducer — it no longer DECLINES a dep-bearing reducer; a
    // dep-bearing component builds fine and composes at fold time. This test is the invalid-bytes guard.)
    match AsyncComponentReducer::from_component_bytes(b"not a component") {
        Err(ComponentError::InvalidComponent(_)) => {}
        Err(e) => {
            panic!("expected InvalidComponent for garbage bytes, got a different error: {e:?}")
        }
        Ok(_) => panic!("garbage bytes must not build an async reducer"),
    }
}

// The async dep-path API + guard (reviewer #2253): a component that declares a `+<hash>` dep builds fine
// (no longer declined), `deps()` reports it, and folding WITHOUT attaching the resolved deps fails with an
// ACTIONABLE error naming the builders — not an opaque wasmtime linker error. (A full green dep-bearing
// async fold is proven downstream by v-ah-host's genesis E2E, the async twin of the sync b1 e2e.)
#[tokio::test(flavor = "current_thread")]
async fn async_reducer_declaring_a_dep_reports_it_and_folds_loud_without_attach() {
    // A minimal component that IMPORTS a content-addressed dep (`+<64-hex>`). declared_deps recognizes the
    // `+<hash>` build-metadata → this is a dep-bearing reducer (no fold world needed: the actionable
    // no-deps-attached guard fires before instantiation).
    let hex = "a".repeat(64);
    let src = format!(
        r#"(component
             (import "cadenza:runtime/heap@0.0.0+{hex}" (instance
               (export "box-int" (func (param "v" s64) (result u32))))))"#
    );
    let bytes = wat::parse_str(&src).expect("assemble dep-declaring component");

    let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
        .expect("dep-bearing async reducer builds");
    // deps() reports the declared dep (API parity with the sync path).
    assert_eq!(reducer.deps().len(), 1, "declares one +hash dep");
    assert!(
        reducer.deps()[0]
            .import_name
            .starts_with("cadenza:runtime/heap@0.0.0+"),
        "deps() surfaces the declared import name"
    );

    // Fold WITHOUT with_resolved_deps → actionable error naming the builders (not an opaque linker error).
    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    match reducer.apply(Kv::new(), ct, None, None).await {
        Err((ComponentError::Instantiate(msg), _kv)) => assert!(
            msg.contains("with_resolved_deps") && msg.contains("declares"),
            "expected an actionable no-deps-attached error naming the builders, got {msg:?}"
        ),
        other => {
            panic!("expected an actionable Instantiate error for unattached deps, got {other:?}")
        }
    }
}
