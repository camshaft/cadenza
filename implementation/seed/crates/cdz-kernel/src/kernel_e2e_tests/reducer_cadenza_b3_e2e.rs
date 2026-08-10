//! End-to-end test of the BINARY-AST fold boundary (B2/B3) against the REAL B3 Cadenza reducer
//! (`packages.reducer-cadenza-b3` — v-nix's nix-built component from `reducer_b3.cdz`).
//! B3 is the FAITHFUL Rust-guest-parity port (§21c real-reducer bar): it exercises the FULL reducer surface,
//! and — unlike b1 (empty effects) / b2 (one effect, no kv) — the BOUND kv import (`cadenza:agent-kernel/kv`
//! get+put). This is the first kernel-e2e that proves a real reducer's KV writes land during the fold: the
//! guest reads/writes its own KV via the host `kv` import while the host drives it through the byte apply.
//!
//! B3's documented behavior (reducer_b3.cdz, mirrors ../reducer-guest/src/lib.rs):
//!   - content-type.family == "message" → bump a KV "count" counter (kv get→+1→put) + emit ONE Http effect
//!     to `https://ok.host/x` with correlation token "step-1".
//!   - resumes = Some(_) (a completed effect's result) → emit nothing.
//!   - any other event → emit nothing.
//!
//! So folding ONE "message" event on an EMPTY KV asserts: kv["count"] == [1] (the bound-kv get(None→0)+put
//! landed through the shared heap) AND exactly one Http effect (target + correlation) — proving BOTH the
//! bound-kv write path and the effect-construction path in one fold.
//!
//! Driven through the ASYNC `AsyncComponentReducer::apply` (the byte-AST fold) — the path the production
//! host (cdz-agent-host) uses, which serves the guest's `kv` import during the fold.
//!
//! ## Env contract (identical to the b1/b2 e2e — env-gated skip)
//! - `REDUCER_CADENZA_B3_COMPONENT` — path to the compiled b3 component (v-nix exports it in the cdz-kernel
//!   CI job from `packages.reducer-cadenza-b3`). UNSET → clean SKIP (a bare `cargo test` stays green).
//! - `CDZ_STORE` — the componentStore dir (`<hash>.wasm` + `runtime.toml`) the §23 transitive compose reads
//!   to resolve the runtime's own bare `cadenza:nfc/normalize`. UNSET → SKIP (else an opaque mid-run
//!   linker error — same rationale as the b1/b2 e2e).

use crate::ast_marshal::{build_event_document, ContentTypeRef};
use crate::wasm_host::{AsyncComponentReducer, ComponentDep};

/// The compiled b3 component bytes from `REDUCER_CADENZA_B3_COMPONENT`; `None` (clean SKIP) when unset. A
/// set path that's unreadable PANICS (a broken CI path must fail loud, not skip).
fn reducer_bytes() -> Option<Vec<u8>> {
    let p = std::env::var("REDUCER_CADENZA_B3_COMPONENT").ok()?;
    Some(std::fs::read(&p).unwrap_or_else(|e| {
        panic!("REDUCER_CADENZA_B3_COMPONENT={p:?} is set but unreadable: {e}")
    }))
}

#[tokio::test(flavor = "current_thread")]
async fn reducer_cadenza_b3_bumps_kv_count_and_emits_one_http_effect() {
    let Some(reducer_component) = reducer_bytes() else {
        eprintln!(
            "SKIP reducer_cadenza_b3_bumps_kv_count_and_emits_one_http_effect: \
             REDUCER_CADENZA_B3_COMPONENT unset"
        );
        return;
    };
    // Like b1/b2: the real b3 runtime imports the bare `cadenza:nfc/normalize` (transitive dep), resolvable
    // ONLY from a `CDZ_STORE`. Without it the fold would fail DEEP with an opaque "nfc not found in linker"
    // — SKIP cleanly + explain instead (a bare `cargo test` stays green; CI always sets CDZ_STORE).
    if std::env::var("CDZ_STORE").is_err() {
        eprintln!(
            "SKIP reducer_cadenza_b3_bumps_kv_count_and_emits_one_http_effect: \
             CDZ_STORE unset — required to resolve the runtime's transitive cadenza:nfc/normalize dep (§23)"
        );
        return;
    }

    let reducer = match AsyncComponentReducer::from_component_bytes(&reducer_component) {
        Ok(r) => r,
        Err(e) => panic!("reducer_b3 must be a valid component: {e:?}"),
    };

    // b3 declares its runtime dep by `+<hash>` (a real Cadenza reducer lowers compounds to opaque value-heap
    // handles internally). Resolve + attach so `apply` can compose the runtime (+ its transitive nfc,
    // via the store) into the fold's linker, and serve the reducer's BOUND cadenza:agent-kernel/kv import.
    let deps = reducer.deps().to_vec();
    // Assert the HEAP dep SPECIFICALLY (not just any dep): strip the `@version` / `+hash` build-metadata off
    // each import name and match the bare interface (b1/b2 twin discipline).
    assert!(
        deps.iter().any(|d| {
            d.import_name.split(['@', '+']).next().map(str::trim) == Some("cadenza:runtime/heap")
        }),
        "a real Cadenza reducer_b3 must declare a cadenza:runtime/heap dep (it can't fold without a heap); \
         declared deps: {:?}",
        deps.iter().map(|d| &d.import_name).collect::<Vec<_>>()
    );
    let resolved = resolve_runtime_deps(&deps).await;
    let mut reducer = reducer.with_resolved_deps(resolved);
    if let Ok(store_dir) = std::env::var("CDZ_STORE") {
        reducer =
            reducer.with_component_store(crate::component_store::ComponentStore::open(&store_dir));
    }

    // Fold an inbound "message" event on an EMPTY KV. b3 bumps the "count" counter (get→None→0, +1, put)
    // via its bound handle-ABI kv AND emits one Http effect. The kv assertion is the point: it proves the
    // guest's kv.put landed in the session KV during the fold via the host kv import.
    let event = build_event_document(
        ContentTypeRef {
            family: "message",
            version: 1,
        },
        None,
        None,
    );
    match reducer.apply(crate::kv::Kv::new(), &event).await {
        Ok((effects, kv)) => {
            // The guest's kv.put landed: "count" is a one-byte counter bumped from absent(0) to 1.
            assert_eq!(
                kv.get(b"count").map(|v| v.to_vec()),
                Some(vec![1u8]),
                "reducer_b3 must bump kv \"count\" from absent(0) to [1] via its kv.put"
            );
            // And exactly one Http effect to ok.host/x with correlation "step-1".
            assert_eq!(
                effects.len(),
                1,
                "reducer_b3 emits exactly one effect on a message event, got {}",
                effects.len()
            );
            let e = &effects[0];
            assert_eq!(
                e.request.content_type.family.as_ref(),
                "http",
                "b3's effect family is http, got {:?}",
                e.request.content_type.family
            );
            assert_eq!(
                e.request.target_str().unwrap(),
                "https://ok.host/x",
                "b3's Http target"
            );
            assert_eq!(
                e.token.as_deref(),
                Some(&b"step-1"[..]),
                "b3's correlation token is \"step-1\""
            );
        }
        Err((e, _kv)) => {
            panic!("apply should drive the real reducer_b3 without error, got {e:?}")
        }
    }
}

/// Resolve every declared dep's bytes from whichever runtime source v-nix wired: `RUNTIME_HEAP_COMPONENT`
/// (a direct path, composed as-is) takes precedence; else `CDZ_STORE` (v-nix's componentStore, read via
/// `ComponentStore::get_by_hash` — the SHA-256-verified content-address path the fold uses) — identical to
/// the b1/b2 e2e's resolver.
async fn resolve_runtime_deps(deps: &[ComponentDep]) -> Vec<(ComponentDep, Vec<u8>)> {
    // Direct-path override supplies ONE component's bytes → only valid for a single-dep reducer (b3
    // declares exactly one, its cadenza:runtime/heap). FAIL LOUD on >1 dep (github-liaison #2312) — mapping
    // the same heap bytes to every dep would silently mis-supply an unrelated dep; use CDZ_STORE for a
    // multi-dep reducer. (Mirrors apply's "MORE THAN ONE cadenza:runtime/heap dep" fail-loud.)
    if let Ok(path) = std::env::var("RUNTIME_HEAP_COMPONENT") {
        assert_eq!(
            deps.len(),
            1,
            "RUNTIME_HEAP_COMPONENT override supplies ONE component but the reducer declares {} deps — \
             use CDZ_STORE (content-addressed, per-dep) for a multi-dep reducer",
            deps.len()
        );
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("RUNTIME_HEAP_COMPONENT={path:?} set but unreadable: {e}"));
        return deps.iter().cloned().map(|d| (d, bytes.clone())).collect();
    }
    let store_dir = std::env::var("CDZ_STORE").unwrap_or_else(|_| {
        panic!(
            "reducer_b3 declares a runtime dep but neither RUNTIME_HEAP_COMPONENT nor CDZ_STORE is set \
             — the e2e can't supply the value-heap runtime to compose"
        )
    });
    // Resolve each dep through the REAL ComponentStore reader (`get_by_hash`) — the SAME production path the
    // fold uses — NOT a manual `std::fs::read`. This exercises the #2210 SHA-256 content-address verify, so a
    // corrupted/substituted store blob surfaces as `ContentAddressMismatch` here instead of composing
    // silently (mirrors the b1/b2 resolvers). Open the store ONCE + reuse.
    let store = crate::component_store::ComponentStore::open(&store_dir);
    let mut out = Vec::with_capacity(deps.len());
    for dep in deps {
        let bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
            panic!(
                "CDZ_STORE has no valid blob for runtime dep {:?} (hash {}): {e:?} — is componentStore \
                 hash-keyed with <hash>.wasm naming + content-address intact?",
                dep.import_name,
                dep.hash.to_hex()
            )
        });
        out.push((dep.clone(), bytes));
    }
    out
}
