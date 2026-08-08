//! Signature-query part-1 END-TO-END: a reducer emits a `control/signature` effect naming a REAL
//! `cadenza:syntax` component (by its blob hash), the host loop surfaces it, resolves the target bytes
//! through the factory's blob store, reflects the component's exported funcs via the wasmtime-free kernel
//! seam, and FOLDS the descriptor back so the reducer resumes with a decodable `ComponentSignature`.
//!
//! This exercises the HAPPY REFLECT PATH the unit tests can't (they cover the surface + the absent/err-arm
//! settle hermetically): here `HostedSession::settle_signature_query` → `component_signature_from_bytes_owned`
//! reflects a GENUINE component and the reducer resumes with a decodable descriptor. It drives the same
//! per-effect step the async loop's `AgentHost::deliver_answering_signatures` runs (surface the control
//! effect → resolve the target bytes → settle), but directly on the `HostedSession` (no factory/loop plumbing
//! — the loop's `fetch_blob` dispatch is unit-tested separately; the NEW thing here is a real target that
//! actually reflects). GATED on `CDZ_SYNTAX_COMPONENT` (v-nix `packages.syntax-guest`, wired in flake.nix):
//! unset → SKIP cleanly (a plain `cargo test` has no wasm toolchain); set → the component MUST reflect into a
//! descriptor carrying the syntax world's exports.

mod common;

use cdz_agent_host::HostedSession;
use cdz_kernel::blob::{BlobStore, MemBlobStore};
use cdz_kernel::effect::{effect_ct, EffectRequest, Payload, Timeliness};
use cdz_kernel::event::{EffectOutcome, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};

/// A reducer that, on inbound, emits a `control/signature` effect naming `target_hex` (the syntax component's
/// blob hash); when the host folds the reflected descriptor back as the EffectResult, it records the raw
/// descriptor bytes into KV under `sig-ok` (or `sig-err` on the error arm), so the test asserts the reducer
/// RESUMED with a real signature.
struct SignatureQueryAgent {
    target_hex: String,
}

#[async_trait::async_trait(?Send)]
impl Reducer for SignatureQueryAgent {
    async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new_with_family(
                effect_ct::SIGNATURE,
                self.target_hex.clone(),
                None,
                Timeliness::Interactive,
            )]),
            EventBody::EffectResult { result, .. } => {
                match result {
                    EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                        kv.put(b"sig-ok".to_vec(), bytes.to_vec());
                    }
                    EffectOutcome::Err { .. } => {
                        kv.put(b"sig-err".to_vec(), b"1".to_vec());
                    }
                    _ => {}
                }
                FoldOutput::none()
            }
            _ => FoldOutput::none(),
        }
    }
}

fn inbound_go() -> EventBody {
    EventBody::Inbound {
        content_type: cdz_kernel::event::ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(b"go".to_vec().into()),
    }
}

#[tokio::test]
async fn signature_query_reflects_a_real_syntax_component_end_to_end() {
    let Some(component) = common::syntax_component_bytes() else {
        eprintln!("CDZ_SYNTAX_COMPONENT unset — skipping the signature-query reflect E2E");
        return;
    };

    // The target IS the real syntax component; its hash is what the reducer names in the effect. (A blob
    // store round-trip just to derive the content hash — the settle path below is handed the bytes directly,
    // exactly the bytes the loop's fetch_blob would resolve.)
    let mut blob = MemBlobStore::new();
    let target_hash = blob
        .put(&component)
        .await
        .expect("put the syntax component");

    // A signature-query session naming the target by hash. Rust reducer (the thing under test is the HOST
    // reflect+fold path over a REAL target component, not a wasm consumer); control/signature is authz-exempt,
    // so a deny-all session queries with no grant + no executors.
    let mut session = HostedSession::genesis(
        Hash::of(b"sigquery-e2e-agent-v1"),
        Box::new(SignatureQueryAgent {
            target_hex: target_hash.to_hex(),
        }),
        Box::new(cdz_kernel::authz::Authorizer::deny_all()),
        CompositeExecutor::new(),
    );

    // Drive exactly what the loop does per surfaced effect: deliver-surfacing-controls → resolve the target
    // bytes (here the real component) → settle_signature_query (reflect + fold). With a REAL component the
    // reflect produces a genuine descriptor + the reducer resumes on the Ok arm.
    let controls = session
        .deliver_surfacing_controls(inbound_go(), None)
        .await
        .expect("deliver ok");
    let ce = controls
        .into_iter()
        .find(|ce| ce.request.content_type.matches_family(effect_ct::SIGNATURE))
        .expect("the reducer emitted control/signature");
    let settled = session.settle_signature_query(&ce, Some(&component)).await;
    assert!(settled, "the control/signature effect settled");

    // The reducer resumed on the Ok arm with a real descriptor — assert it did NOT take the Err arm, then
    // decode + assert it carries the syntax world's exported funcs (proving a genuine reflection).
    assert!(
        session.session().kv().get(b"sig-err").is_none(),
        "reflection of a real component takes the Ok arm, not Err"
    );
    let descriptor = session
        .session()
        .kv()
        .get(b"sig-ok")
        .map(|b| b.to_vec())
        .expect("the reducer recorded the folded-back descriptor (sig-ok)");
    let sig = cdz_kernel::event_ast::decode_component_signature(&descriptor)
        .expect("the folded-back descriptor decodes as a ComponentSignature");
    let names: Vec<&str> = sig.exports.iter().map(|e| e.name.as_str()).collect();
    assert!(
        !sig.exports.is_empty(),
        "the reflected signature carries the syntax component's exported funcs, got {names:?}"
    );
    // The cadenza:syntax world exports parse/query/doc surfaces; at least one export name should be present.
    // (Exact set depends on the guest's lifted interface; assert non-empty + name-shaped rather than pin an
    // exact list the guest owns.)
    assert!(
        names.iter().all(|n| !n.is_empty()),
        "every reflected export has a name: {names:?}"
    );
}
