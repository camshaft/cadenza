//! End-to-end async fold: drive the REAL committed reducer-guest component through
//! [`AsyncComponentReducer`] (the cooperative-gas-yield path, operator all-async directive) and assert it
//! behaves identically to the sync `ComponentReducer` — same effects, same KV mutation — but via the async
//! engine (`async_support` + `fuel_async_yield_interval`) and an `.await`ed `call_apply`. This is the
//! async twin of `component_reducer_e2e.rs`'s first case: proof that the async apply round-trips the guest
//! effect-request + the `kv` host import + the mutated KV across the component boundary, on a real
//! single-threaded (current-thread) executor (the runtime the kernel is designed for — no Send).

use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::Reducer;
use cdz_kernel::wasm_host::{AsyncComponentReducer, ComponentError, ContentType, EffectKind};

// The SAME committed guest fixture the sync e2e uses — a dependency-free wit-bindgen reducer that, on an
// inbound "message", requests one Http effect + increments a KV counter.
const GUEST: &[u8] = include_bytes!("fixtures/reducer_guest.component.wasm");

#[tokio::test(flavor = "current_thread")]
async fn real_guest_folds_through_async_apply_end_to_end() {
    let reducer = match AsyncComponentReducer::from_component_bytes(GUEST) {
        Ok(r) => r,
        Err(e) => panic!("the guest fixture must be a valid async reducer component: {e:?}"),
    };

    let ct = ContentType {
        family: "message".to_string(),
        version: 1,
    };
    let (effects, kv) = reducer
        .apply(Kv::new(), ct, Some(b"hello".to_vec()), None)
        .await
        .expect("async apply drives the guest without trapping");

    // Identical observable behavior to the sync path: exactly one Http effect with the guest's own
    // correlation token, round-tripped across the component boundary.
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].kind, EffectKind::Http);
    assert_eq!(effects[0].target, "https://ok.host/x");
    assert_eq!(effects[0].correlation.as_deref(), Some(&b"step-1"[..]));

    // The guest wrote its KV counter via the `kv` host import, and the mutated KV came back out.
    assert_eq!(kv.get(b"count"), Some(&[1u8][..]));
}

#[tokio::test(flavor = "current_thread")]
async fn async_reducer_drives_via_the_async_reducer_trait() {
    // Drive through the `Reducer` trait (what the async kernel loop will call) via a `&dyn` reference
    // — proving the native `impl Reducer for AsyncComponentReducer` is object-safe + folds a real
    // Inbound event, mutating the passed-in KV (the fold_async contract).
    let reducer =
        AsyncComponentReducer::from_component_bytes(GUEST).expect("valid async reducer component");
    let dyn_reducer: &dyn Reducer = &reducer;

    let event = cdz_kernel::event::Event {
        seq: 0,
        cause: None,
        body: cdz_kernel::event::EventBody::Inbound {
            content_type: cdz_kernel::event::ContentType {
                family: "message".to_string(),
                version: 1,
            },
            payload: cdz_kernel::effect::Payload::Inline(b"hello".to_vec().into()),
        },
    };
    let mut kv = Kv::new();
    let out = dyn_reducer.fold_async(&event, &mut kv).await;

    // The fold succeeded (no failure), emitted the guest's one Http effect, and mutated the KV in place.
    // `fold_async` yields KERNEL effect types (crate::effect), not the guest-wire `wasm_host::EffectKind`.
    assert!(
        out.failure.is_none(),
        "fold should not fail: {:?}",
        out.failure
    );
    assert_eq!(out.effects.len(), 1);
    assert_eq!(
        out.effects[0].request.kind,
        cdz_kernel::effect::EffectKind::Http
    );
    assert_eq!(kv.get(b"count"), Some(&[1u8][..]));
}

#[tokio::test(flavor = "current_thread")]
async fn async_reducer_declines_a_component_with_dependencies() {
    // The async path doesn't yet compose §23 component deps (a follow-up); it must DECLINE such a
    // component loudly, not instantiate one whose deps it would silently drop. A non-reducer blob also
    // declines. (Here: garbage bytes are an InvalidComponent — the nearest available negative fixture.)
    match AsyncComponentReducer::from_component_bytes(b"not a component") {
        Err(ComponentError::InvalidComponent(_)) => {}
        Err(e) => {
            panic!("expected InvalidComponent for garbage bytes, got a different error: {e:?}")
        }
        Ok(_) => panic!("garbage bytes must not build an async reducer"),
    }
}
