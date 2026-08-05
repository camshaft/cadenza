//! End-to-end test of the OPTION-C HANDLE-LOWERED reducer path (§19e) against a REAL Cadenza reducer:
//! drive `packages.reducer-cadenza-b1` (v-nix's nix-built component from v-harness-bootstrap's
//! `reducer_b1.cdz`) through [`ComponentReducer::apply_handle_lowered`] and assert the empty-effects fold.
//!
//! B1 is the minimal "a real Cadenza reducer LOADS + FOLDS + RETURNS through the marshalled value-heap
//! boundary at all" proof: it emits NO effects on any event, so the assertion is `vec-len == 0`. This is
//! the first time `apply_handle_lowered` runs against a REAL rcdzc-compiled reducer (not the synthetic WAT
//! fixture in the unit tests) — proving the whole rebind (compose the `cadenza:runtime/heap` dep → bind a
//! HeapHandle on it → marshal the fold inputs → call the interface-nested `apply` → unmarshal the returned
//! effect-list handle) works against the actual lowering rcdzc emits (v-hb confirmed: handle-lowered
//! `apply(u32,u32,u32)->u32`, interface-nested under `cadenza:agent-kernel/fold`, runtime as a composed
//! content-addressed dep).
//!
//! ## Env contract (mirrors the reducer-guest e2e's optional-skip)
//! - `REDUCER_CADENZA_COMPONENT` — path to the compiled b1 component (v-nix exports it in the cdz-kernel
//!   CI job from `packages.reducer-cadenza-b1`). UNSET → the test SKIPs cleanly (a bare `cargo test` with
//!   no nix build stays green), same contract as `REDUCER_GUEST_COMPONENT`.
//! - The b1 component imports `cadenza:runtime/heap@…+<hash>` (a content-addressed dep). The runtime bytes
//!   are supplied by EITHER `RUNTIME_HEAP_COMPONENT` (a direct path to the runtime component, composed
//!   without a hash lookup) OR `CDZ_STORE` (a content-addressed blob store dir whose blobs are named
//!   `<hash>.wasm` — v-nix's `componentStore` layout, NOT the kernel's bare-`<hash>` `DiskBlobStore`
//!   layout — the declared dep is resolved from it by its `+<hash>`; see `resolve_runtime_deps` below).
//!
//! Whichever v-nix wires: the test tries the direct path first, then the store. If the reducer declares a
//! runtime dep but NEITHER is provided, the test FAILS LOUD (a real b1 needs its heap — a silent skip there
//! would hide a broken wiring), distinct from the reducer-env-unset clean skip.

use cdz_kernel::wasm_host::{ComponentDep, ComponentReducer, ContentType};

/// The compiled b1 component bytes from `REDUCER_CADENZA_COMPONENT`; `None` (clean SKIP) when unset. A set
/// path that's unreadable PANICS (a broken CI path must fail loud, not skip).
fn reducer_bytes() -> Option<Vec<u8>> {
    let p = std::env::var("REDUCER_CADENZA_COMPONENT").ok()?;
    Some(
        std::fs::read(&p).unwrap_or_else(|e| {
            panic!("REDUCER_CADENZA_COMPONENT={p:?} is set but unreadable: {e}")
        }),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn reducer_cadenza_b1_folds_empty_effects_through_apply_handle_lowered() {
    let Some(reducer_component) = reducer_bytes() else {
        eprintln!(
            "SKIP reducer_cadenza_b1_folds_empty_effects_through_apply_handle_lowered: \
             REDUCER_CADENZA_COMPONENT unset"
        );
        return;
    };

    let reducer = match ComponentReducer::from_component_bytes(&reducer_component) {
        Ok(r) => r,
        Err(e) => panic!("reducer_b1 must be a valid component: {e:?}"),
    };

    // b1 declares its runtime dep by `+<hash>` (a real Cadenza reducer lowers compounds to opaque
    // value-heap handles, so it imports cadenza:runtime/heap). Resolve every declared dep's bytes, so
    // `apply_handle_lowered` can compose them + bind a HeapHandle on the runtime.
    let deps = reducer.deps().to_vec();
    assert!(
        !deps.is_empty(),
        "a real Cadenza reducer_b1 must declare a cadenza:runtime/heap dep (it can't fold without a heap)"
    );
    let resolved = resolve_runtime_deps(&deps).await;
    let reducer = reducer.with_resolved_deps(resolved);

    // Fold an inbound "message" event. b1 emits ZERO effects → the returned effect-list handle unmarshals
    // to an empty Vec: the minimal real-reducer-through-the-marshalled-boundary proof.
    let ct = ContentType {
        family: "message".into(),
        version: 1,
    };
    match reducer.apply_handle_lowered(cdz_kernel::kv::Kv::new(), ct, None, None) {
        Ok((effects, _kv)) => assert!(
            effects.is_empty(),
            "reducer_b1 is an empty-effects reducer; expected 0 effects, got {}",
            effects.len()
        ),
        Err((e, _kv)) => {
            panic!("apply_handle_lowered should drive the real reducer_b1 without error, got {e:?}")
        }
    }
}

/// Resolve every declared dep's bytes for the b1 e2e, from whichever runtime source v-nix wired:
/// `RUNTIME_HEAP_COMPONENT` (a direct path, composed as-is) takes precedence; else `CDZ_STORE` (a
/// content-addressed [`DiskBlobStore`] the dep is looked up in by its `+<hash>`). A declared dep with
/// neither source available FAILS LOUD.
async fn resolve_runtime_deps(deps: &[ComponentDep]) -> Vec<(ComponentDep, Vec<u8>)> {
    // Direct-path override: compose the runtime component from a path env, no hash lookup. b1 declares a
    // single runtime dep, so a single direct path satisfies it.
    if let Ok(path) = std::env::var("RUNTIME_HEAP_COMPONENT") {
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("RUNTIME_HEAP_COMPONENT={path:?} set but unreadable: {e}"));
        return deps.iter().cloned().map(|d| (d, bytes.clone())).collect();
    }
    // Otherwise resolve by content-address from CDZ_STORE. v-nix's `componentStore` is hash-keyed with a
    // `<hash>.wasm` naming (confirmed: `39358be4….wasm` = the value-heap runtime) — NOT the kernel's
    // `DiskBlobStore` bare-`<hash>` layout, so read `<CDZ_STORE>/<hash>.wasm` directly rather than via
    // DiskBlobStore. (If v-nix's store layout ever changes to bare-hash, swap this back to DiskBlobStore.)
    let store_dir = std::env::var("CDZ_STORE").unwrap_or_else(|_| {
        panic!(
            "reducer_b1 declares a runtime dep but neither RUNTIME_HEAP_COMPONENT nor CDZ_STORE is set \
             — the e2e can't supply the value-heap runtime to compose"
        )
    });
    let mut out = Vec::with_capacity(deps.len());
    for dep in deps {
        let path = std::path::Path::new(&store_dir).join(format!("{}.wasm", dep.hash.to_hex()));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "CDZ_STORE has no blob {path:?} for runtime dep {:?} (hash {}): {e} — is componentStore \
                 hash-keyed with <hash>.wasm naming?",
                dep.import_name,
                dep.hash.to_hex()
            )
        });
        out.push((dep.clone(), bytes));
    }
    out
}
