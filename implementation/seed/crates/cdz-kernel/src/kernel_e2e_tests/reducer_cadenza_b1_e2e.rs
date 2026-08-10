//! End-to-end test of the BINARY-AST fold boundary (B2/B3) against a REAL Cadenza reducer: drive
//! `packages.reducer-cadenza-b1` (v-nix's nix-built component from `reducer_b1.cdz`) through
//! [`ComponentReducer::apply`] and assert the empty-effects fold.
//!
//! B1 is the minimal "a real Cadenza reducer LOADS + FOLDS + RETURNS through the byte-AST boundary at all"
//! proof: it emits NO effects on any event, so the assertion is an empty effect list. This runs the real
//! rcdzc-compiled reducer (not the synthetic WAT fixture in the unit tests) through the ONE byte apply —
//! `apply(list<u8>) -> list<u8>`: the host builds the event document, the guest (B3: its `value-decode`/
//! `value-encode` composition wraps the fold) returns its effect list, and the host parses it via
//! `ast_marshal::parse_effect_list`. The reducer still declares its `cadenza:runtime/heap` dep (it uses
//! the value heap internally for its own compounds), composed here so the fold can run.
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

use crate::ast_marshal::{build_event_document, ContentTypeRef};
use crate::wasm_host::{ComponentDep, ComponentReducer};

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
async fn reducer_cadenza_b1_folds_empty_effects_through_the_byte_ast_apply() {
    let Some(reducer_component) = reducer_bytes() else {
        eprintln!(
            "SKIP reducer_cadenza_b1_folds_empty_effects_through_the_byte_ast_apply: \
             REDUCER_CADENZA_COMPONENT unset"
        );
        return;
    };
    // The real b1 runtime imports the bare `cadenza:nfc/normalize` (its transitive dep), which the §23
    // compose resolves ONLY from a `CDZ_STORE` (by name via `runtime.toml`). Without CDZ_STORE the fold
    // would fail DEEP in `apply` with an opaque "cadenza:nfc/normalize not found in linker"
    // — so SKIP cleanly + explain here rather than emit a confusing mid-run linker error (a `cargo test`
    // without the nix env stays green; CI always sets CDZ_STORE). Matches the other env-gated e2es.
    // (`RUNTIME_HEAP_COMPONENT` supplies the runtime BYTES but no store, so it can't resolve transitive nfc
    // for a real nfc-importing runtime — CDZ_STORE is the requirement for this e2e.)
    if std::env::var("CDZ_STORE").is_err() {
        eprintln!(
            "SKIP reducer_cadenza_b1_folds_empty_effects_through_the_byte_ast_apply: \
             CDZ_STORE unset — required to resolve the runtime's transitive cadenza:nfc/normalize dep (§23)"
        );
        return;
    }

    let reducer = match ComponentReducer::from_component_bytes(&reducer_component) {
        Ok(r) => r,
        Err(e) => panic!("reducer_b1 must be a valid component: {e:?}"),
    };

    // b1 declares its runtime dep by `+<hash>` (a real Cadenza reducer lowers compounds to opaque
    // value-heap handles internally, so it imports cadenza:runtime/heap). Resolve every declared dep's
    // bytes, so `apply` can compose them into the fold's linker.
    let deps = reducer.deps().to_vec();
    // Assert the HEAP dep SPECIFICALLY (not just any dep): strip the `@version` / `+hash` build-metadata
    // off each import name and match the bare interface — so the check actually verifies what its message
    // claims (a non-heap dep alone must NOT satisfy it, else the fold fails later with an opaque error).
    assert!(
        deps.iter().any(|d| {
            d.import_name.split(['@', '+']).next().map(str::trim) == Some("cadenza:runtime/heap")
        }),
        "a real Cadenza reducer_b1 must declare a cadenza:runtime/heap dep (it can't fold without a heap); \
         declared deps: {:?}",
        deps.iter().map(|d| &d.import_name).collect::<Vec<_>>()
    );
    let resolved = resolve_runtime_deps(&deps).await;
    let mut reducer = reducer.with_resolved_deps(resolved);
    // Attach the component store so the TRANSITIVE compose (§23) can resolve the runtime's OWN bare
    // `cadenza:nfc/normalize` import by name from `CDZ_STORE`'s `runtime.toml` (the value-heap runtime is
    // not a leaf — it imports nfc). Without this, `apply` composing the runtime would fail
    // "imports cadenza:nfc/normalize, not found in linker". Only when CDZ_STORE is wired (the nix path);
    // the RUNTIME_HEAP_COMPONENT direct-path override has no store, so a runtime that imports nfc needs
    // CDZ_STORE. (See `ComponentReducer::with_component_store` + `compose_transitive_bare_deps`.)
    if let Ok(store_dir) = std::env::var("CDZ_STORE") {
        reducer =
            reducer.with_component_store(crate::component_store::ComponentStore::open(&store_dir));
    }

    // Fold an inbound "message" event (B2 binary-AST boundary: the event is one cadenza-AST document, and
    // a B3-emitted rcdzc reducer folds through the SAME `apply(list<u8>)->list<u8>` as any guest). b1 emits
    // ZERO effects → the returned effect list decodes to an empty Vec: the minimal real-reducer proof.
    let event = build_event_document(
        ContentTypeRef {
            family: "message",
            version: 1,
        },
        None,
        None,
    );
    match reducer.apply(crate::kv::Kv::new(), &event) {
        Ok((effects, _kv)) => assert!(
            effects.is_empty(),
            "reducer_b1 is an empty-effects reducer; expected 0 effects, got {}",
            effects.len()
        ),
        Err((e, _kv)) => {
            panic!("apply should drive the real reducer_b1 without error, got {e:?}")
        }
    }
}

/// Resolve every declared dep's bytes for the b1 e2e, from whichever runtime source v-nix wired:
/// `RUNTIME_HEAP_COMPONENT` (a direct path, composed as-is) takes precedence; else `CDZ_STORE` (a
/// content-addressed `ComponentStore`, read via `get_by_hash` — the SHA-256-verified content-address
/// path the fold itself uses). A declared dep with neither source available FAILS LOUD.
async fn resolve_runtime_deps(deps: &[ComponentDep]) -> Vec<(ComponentDep, Vec<u8>)> {
    // Direct-path override: compose the runtime component from a path env, no hash lookup. The override
    // supplies ONE component's bytes, so it only makes sense for a single-dep reducer — b1 declares exactly
    // one (its cadenza:runtime/heap). FAIL LOUD if a future reducer declares >1 dep (github-liaison #2312):
    // mapping the same heap bytes to every dep would silently hand an unrelated dep the wrong component →
    // a deep compose/link failure instead of this targeted error. (Mirrors apply's
    // "MORE THAN ONE cadenza:runtime/heap dep" fail-loud.) Use CDZ_STORE for a genuine multi-dep reducer.
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
    // Otherwise resolve by content-address from CDZ_STORE through the REAL ComponentStore reader
    // (`get_by_hash`) — the SAME production path the fold uses — NOT a manual `std::fs::read`. This exercises
    // the #2210 SHA-256 content-address verify, so a corrupted/substituted store blob surfaces as
    // `ContentAddressMismatch` here instead of composing silently (reviewer + github-liaison note; mirrors
    // #2269's genesis-e2e fix). Open the store ONCE + reuse. v-nix's `componentStore` is hash-keyed with the
    // `<hash>.wasm` naming get_by_hash expects (confirmed: `39358be4….wasm` = the value-heap runtime).
    let store_dir = std::env::var("CDZ_STORE").unwrap_or_else(|_| {
        panic!(
            "reducer_b1 declares a runtime dep but neither RUNTIME_HEAP_COMPONENT nor CDZ_STORE is set \
             — the e2e can't supply the value-heap runtime to compose"
        )
    });
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
