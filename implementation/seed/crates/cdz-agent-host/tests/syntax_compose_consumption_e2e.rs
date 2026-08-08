//! Signature-query PART-2 (flavor-2, pure-library compose) END-TO-END: a reducer guest that IMPORTS
//! `cadenza:syntax/parse` by content-addressed `+hash` is composed against its dep + driven through a fold;
//! the fold calls the COMPOSED `parse.read-sexpr` and records the result in KV, proving the dep was actually
//! linked + called (not stubbed).
//!
//! This is the flavor-2 half of component-calls-as-cadenza-programs: NO invoke-wire, NO host effect — a direct
//! linked call through the kernel's `compose_dep_into_linker` seam (the same machinery that composes the
//! runtime-heap dep). The reducer-HOST composition (blob-resolve both components → `AsyncComponentReducer`
//! detects the `+hash` dep → `resolve_deps`/`with_resolved_deps` from the blob store → the per-fold
//! `compose_dep_into_linker` links it → drive a fold) is v-agent-harness-host's lane; the producer
//! (cadenza:syntax) + the consumer guest are v-syntax's; the +hash templating is v-nix's flake.
//!
//! GATED on BOTH `CDZ_SYNTAX_CONSUMER_COMPONENT` (the consumer guest bytes) AND `CDZ_SYNTAX_COMPONENT` (the
//! syntax dep bytes) — v-nix wires both into the nix check. Unset → SKIP cleanly (a plain `cargo test` has no
//! wasm toolchain); set → the consumer MUST compose its dep + a valid s-expr source MUST parse clean.

mod common;

use cdz_agent_host::HostedSession;
use cdz_kernel::blob::{BlobStore, MemBlobStore};
use cdz_kernel::event::{ContentType, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::wasm_host::AsyncComponentReducer;

/// A valid Cadenza s-expr source the composed `parse.read-sexpr` should parse clean (no diagnostics).
const SEXPR_SOURCE: &[u8] = b"(def x 1)";

#[tokio::test]
async fn a_reducer_composes_cadenza_syntax_and_calls_the_linked_parse() {
    let (Some(consumer), Some(syntax_dep)) = (
        common::syntax_consumer_component_bytes(),
        common::syntax_component_bytes(),
    ) else {
        eprintln!(
            "CDZ_SYNTAX_CONSUMER_COMPONENT / CDZ_SYNTAX_COMPONENT unset — skipping the part-2 compose E2E"
        );
        return;
    };

    // Put the syntax dep into a blob store: its content hash IS the `+hash` the consumer's import declares
    // (v-nix templates the consumer's cadenza:syntax import with this same producer's hash), so the reducer's
    // resolve_deps finds it by that hash. (put returns the hash; we don't need it — resolve_deps reads the
    // hash off the consumer's declared dep import name.)
    let mut blob = MemBlobStore::new();
    let _dep_hash = blob
        .put(&syntax_dep)
        .await
        .expect("put the syntax dep component");

    // Lift the consumer guest. from_component_bytes detects its declared `+hash` cadenza:syntax dep; resolve
    // its bytes from the blob store + attach, so the per-fold compose_dep_into_linker links it before the
    // guest instantiates. A missing dep (hash not in the store) would be a clean DepMissing error here.
    let reducer =
        AsyncComponentReducer::from_component_bytes(&consumer).expect("consumer guest lifts");
    assert!(
        !reducer.deps().is_empty(),
        "the consumer declares the cadenza:syntax +hash dep (compose has something to link)"
    );
    let resolved = reducer
        .resolve_deps(&blob)
        .await
        .expect("the consumer's cadenza:syntax dep resolves from the blob store by its +hash");
    let reducer = reducer.with_resolved_deps(resolved);

    // Wrap in a HostedSession (deny-all authz — the parse call is a linked dep, not a world-effect, so it
    // needs no capability grant) + a hermetic executor set (the fold performs no effects).
    let mut session = HostedSession::genesis(
        cdz_kernel::hash::Hash::of(b"syntax-consumer-e2e-v1"),
        Box::new(reducer),
        Box::new(cdz_kernel::authz::Authorizer::deny_all()),
        CompositeExecutor::new(),
    );

    // Deliver a `message` inbound carrying valid s-expr source: the fold calls the COMPOSED parse.read-sexpr
    // + records `parse-result` = [clean_flag:u8, ast_len:u32_le] in KV.
    let body = EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: cdz_kernel::effect::Payload::Inline(SEXPR_SOURCE.to_vec().into()),
    };
    session
        .deliver(body, None)
        .await
        .expect("the consumer fold runs (composed parse call included)");

    // Assert the composed dep was actually called: a clean parse of valid source records clean_flag==1 +
    // a non-empty AST byte length. (If compose had failed or the dep were a stub, the fold couldn't have
    // produced a clean parse of real source.)
    let record = session
        .session()
        .kv()
        .get(b"parse-result")
        .map(|b| b.to_vec())
        .expect("the fold recorded parse-result (proving it ran the composed parse)");
    assert!(
        !record.is_empty(),
        "parse-result carries at least the clean flag: {record:?}"
    );
    assert_eq!(
        record[0], 1,
        "the composed parse.read-sexpr parsed the valid s-expr CLEAN (no diagnostics): {record:?}"
    );
    // The AST byte length follows as 4 LE bytes (present on a clean parse) — assert it's non-zero, proving a
    // real AST came back from the linked dep, not an empty/stub result.
    assert!(
        record.len() >= 5,
        "a clean parse records the 4-byte LE ast_len after the flag: {record:?}"
    );
    let ast_len = u32::from_le_bytes([record[1], record[2], record[3], record[4]]);
    assert!(
        ast_len > 0,
        "the composed parse produced a non-empty AST (ast_len > 0): {ast_len}"
    );
}
