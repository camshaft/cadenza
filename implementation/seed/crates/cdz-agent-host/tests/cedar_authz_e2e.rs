//! End-to-end: a real agent gated by a REAL Cedar policy decision through the kernel's
//! `ComponentAuthorizer` — the operator's "Cedar as a content-addressable wasm component" pillar (§20b),
//! proven live. This is the executable fail-closed-DECISION pin the reviewer/concierge asked for (the
//! kernel's ComponentAuthorizer is otherwise only inspection-tested for allow/deny, because a decision
//! test needs THIS crate's Cedar policy guest).
//!
//! **Fixture delivery (the CI-only strategy).** The lifted Cedar policy component is ~3.3 MB (it embeds
//! the Cedar engine), so it is NOT committed to the repo (unlike the 22 KB reducer fixture). Instead the
//! `cdz-agent-host` CI job builds the guest to a component (`wasm-tools component new`) and points this
//! test at it via the `CEDAR_POLICY_COMPONENT` env var. When that var is unset — a plain local
//! `cargo test` without the wasm toolchain — this test SKIPS cleanly (prints a note, passes), so the
//! crate's default gate needs no wasm-tools + no 3.3 MB blob enters the repo. The real decision assertion
//! runs in CI where the component exists.

use cdz_agent_host::{ModelExecutor, ModelTransport};
use cdz_kernel::effect::{EffectKind, EffectRequest, Payload, Timeliness};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};
use cdz_kernel::wasm_host::ComponentAuthorizer;

/// Load the lifted Cedar policy component the CI job built, or `None` to skip (local run without the
/// wasm toolchain / the env var).
fn policy_component_bytes() -> Option<Vec<u8>> {
    let path = std::env::var("CEDAR_POLICY_COMPONENT").ok()?;
    std::fs::read(&path).ok()
}

/// A stub model transport (the Model executor's I/O half) — the agent's Model effect that gets AUTHORIZED
/// reaches this; a canned completion keeps the loop hermetic. (An unauthorized effect never gets here.)
struct StubModel;
impl ModelTransport for StubModel {
    fn invoke(&self, _model_id: &str, _body: &[u8], _key: Hash) -> Result<bytes::Bytes, String> {
        Ok(bytes::Bytes::from_static(b"a completion"))
    }
}

fn inbound(kind_marker: &str) -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(kind_marker.as_bytes().to_vec().into()),
    }
}

/// An agent whose single effect on inbound is chosen by the inbound marker: "model-ok" → a Model call to
/// the policy-allow-listed id; "http-imds" → an Http call to the IMDS host the policy FORBIDs. Lets one
/// reducer drive both the permit and deny cases against the real Cedar component.
struct PolicyProbe;
impl Reducer for PolicyProbe {
    fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { payload, .. } => {
                let Payload::Inline(m) = payload else {
                    return FoldOutput::none();
                };
                match m.as_ref() {
                    b"model-ok" => FoldOutput::with(vec![EffectRequest {
                        kind: EffectKind::Model,
                        target: "claude-test".into(), // policy permits model → this id
                        payload: Some(Payload::Inline(b"hi".to_vec().into())),
                        timeliness: Timeliness::Interactive,
                    }]),
                    b"http-imds" => FoldOutput::with(vec![EffectRequest {
                        kind: EffectKind::Http,
                        target: "http://169.254.169.254/latest/meta-data/".into(), // policy FORBIDs this
                        payload: None,
                        timeliness: Timeliness::Interactive,
                    }]),
                    _ => FoldOutput::none(),
                }
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(_),
                ..
            } => {
                kv.put(b"permitted".to_vec(), b"1".to_vec());
                FoldOutput::none()
            }
            _ => FoldOutput::none(),
        }
    }
}

#[test]
fn a_real_agent_is_gated_by_a_real_cedar_decision() {
    let Some(bytes) = policy_component_bytes() else {
        eprintln!(
            "SKIP cedar_authz_e2e: CEDAR_POLICY_COMPONENT unset (build the cedar-policy-guest component \
             + set the env var — the cdz-agent-host CI job does this). Skipping the live Cedar decision."
        );
        return;
    };

    // The REAL Cedar authorizer, built from the lifted policy component bytes, for this session's principal.
    let authz = ComponentAuthorizer::from_policy_bytes(&bytes, "agent://test")
        .expect("the lifted cedar-policy-guest is a valid authorizer component");

    // PERMIT case: a Model call to the allow-listed id → the policy permits → the executor runs → the
    // result folds back → the agent records "permitted".
    {
        let reducer = PolicyProbe;
        let mut exec = CompositeExecutor::new()
            .with(EffectKind::Model, Box::new(ModelExecutor::new(StubModel)));
        let mut session = Session::genesis(Hash::of(b"cedar-permit-v1"));
        session
            .deliver(inbound("model-ok"), None, &reducer, &authz, &mut exec)
            .unwrap();
        assert_eq!(
            session.kv().get(b"permitted"),
            Some(&b"1"[..]),
            "a policy-permitted Model call must run + fold back"
        );
        assert_eq!(session.open_effects(), 0);
        // No denial on the log for the permitted case.
        assert!(!session
            .log()
            .iter()
            .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
    }

    // DENY case: an Http call to the IMDS host the policy FORBIDs → the Cedar decision denies at the gate
    // → the executor is never consulted → an AuthzDenied is on the log + the agent is not "permitted".
    {
        let reducer = PolicyProbe;
        // A Model executor is registered, but the Http effect is denied before any executor runs; an
        // Http executor isn't even needed to prove the deny (the gate stops it first).
        let mut exec = CompositeExecutor::new()
            .with(EffectKind::Model, Box::new(ModelExecutor::new(StubModel)));
        let mut session = Session::genesis(Hash::of(b"cedar-deny-v1"));
        session
            .deliver(inbound("http-imds"), None, &reducer, &authz, &mut exec)
            .unwrap();
        assert_ne!(
            session.kv().get(b"permitted"),
            Some(&b"1"[..]),
            "a policy-FORBIDden effect must NOT run"
        );
        assert!(
            session
                .log()
                .iter()
                .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
            "a real Cedar deny must be recorded as AuthzDenied on the log (§10 audit)"
        );
        assert_eq!(session.open_effects(), 0);
    }
}
