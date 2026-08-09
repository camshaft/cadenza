//! End-to-end: a capability manifest projected against the REAL host wiring — the Clock/Http/Model
//! executors as the mechanism axis (through `CompositeExecutor::handles_family`) and a real Cedar
//! `ComponentAuthorizer` (the lifted wasm policy) as the policy axis. This is the host half of
//! host-capability-discovery (I2/I3): the kernel's own `project_manifest` tests use the native
//! `Authorizer` + a `RecordingExecutor` stub; this proves the projection over the actual executors a
//! deployed host registers, gated by a real Cedar decision, and pins the I3 scoped-grant probe-target
//! override that only a live policy component can exercise.
//!
//! Fixture delivery (CI-only, same strategy as `cedar_authz_e2e`): the lifted Cedar policy component is
//! ~3.3 MB (embeds the Cedar engine) so it is not committed; the `cdz-agent-host` CI job builds it and
//! points this test at it via `CEDAR_POLICY_COMPONENT`. Unset (a plain local `cargo test` without the
//! wasm toolchain) → this test skips cleanly. When set the component must read — a missing/corrupt
//! component is a CI misconfig, so the shared loader panics rather than silently skipping (same
//! discipline as the S1 store-guard / `cedar_authz_e2e`; see `common::policy_component_bytes`).
//!
//! The guest policy (see `fixtures/cedar-policy-guest/src/lib.rs`): permit now/timer (any), permit http
//! broadly + forbid IMDS, permit model only at `claude-test`, default-deny. Crossed with the host
//! mechanism (executors registered for Now/Http/Model, not Timer/Shell/Emit) the manifest must show all
//! three grant-states, and the model family must flip Denied→Granted under a session probe-target override.

use crate::common::policy_component_bytes;
use bytes::Bytes;
use cdz_agent_host::{
    ClockExecutor, HttpExecutor, HttpMethod, HttpResponse, HttpTransport, ModelExecutor,
    ModelTransport,
};
use cdz_kernel::effect::{effect_ct, project_manifest, GrantState};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::wasm_host::ComponentAuthorizer;

/// Stub transports — the manifest only probes `handles_family` (mechanism) + `authorize` (policy); it
/// NEVER routes an effect to an executor, so these `perform`-half impls are never actually invoked. They
/// exist only so the real `HttpExecutor`/`ModelExecutor` can be constructed and registered (that
/// registration is exactly what makes `handles_family` report the family present).
struct UnusedHttp;
#[async_trait::async_trait(?Send)]
impl HttpTransport for UnusedHttp {
    async fn request(
        &self,
        _method: HttpMethod,
        _url: &str,
        _headers: &[(String, String)],
        _body: Option<&[u8]>,
        _key: Hash,
    ) -> Result<HttpResponse, cdz_kernel::event::EffectOutcome> {
        unreachable!("manifest projection probes policy; it never performs an effect")
    }
}

struct UnusedModel;
#[async_trait::async_trait(?Send)]
impl ModelTransport for UnusedModel {
    async fn invoke(
        &self,
        _model_id: &str,
        _body: &[u8],
        _key: Hash,
    ) -> Result<Bytes, cdz_kernel::event::EffectOutcome> {
        unreachable!("manifest projection probes policy; it never performs an effect")
    }
}

/// The real host executor set a deployed agent registers: Clock (Now), Http, Model — and NOTHING for
/// Timer/Shell/Emit. `handles_family` over this is the mechanism axis of the projection.
fn real_host_executors() -> CompositeExecutor {
    CompositeExecutor::new()
        .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()))
        .with_effect(effect_ct::HTTP, Box::new(HttpExecutor::new(UnusedHttp)))
        .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(UnusedModel)))
}

#[tokio::test]
async fn a_manifest_over_real_executors_and_a_real_cedar_policy_shows_all_three_grant_states() {
    let Some(bytes) = policy_component_bytes() else {
        eprintln!(
            "SKIP capability_manifest_e2e: CEDAR_POLICY_COMPONENT unset (build the cedar-policy-guest \
             component + set the env var — the cdz-agent-host CI job does this). Skipping the live \
             manifest projection."
        );
        return;
    };

    let authz = ComponentAuthorizer::from_policy_bytes(&bytes, "agent://test")
        .expect("the lifted cedar-policy-guest is a valid authorizer component");
    let exec = real_host_executors();

    // Project over the canonical family set with the KERNEL-DEFAULT probe target (one source of truth).
    let manifest = project_manifest(
        effect_ct::ALL,
        |f| exec.handles_family(f),
        &authz,
        effect_ct::probe_target,
    )
    .await;

    let state = |fam: &str| {
        manifest
            .entries
            .iter()
            .find(|e| e.family == fam)
            .unwrap_or_else(|| panic!("family {fam} missing from the manifest"))
            .grant
            .clone()
    };

    // Complete by construction: one entry per canonical family.
    assert_eq!(manifest.entries.len(), effect_ct::ALL.len());

    // NOW: mechanism (ClockExecutor) present + policy permits `now` at any resource → Granted.
    assert_eq!(
        state(effect_ct::NOW),
        GrantState::Granted,
        "now: executor + permit"
    );

    // HTTP: mechanism (HttpExecutor) present + broad `http` permit; the default probe target
    // "https://probe.invalid/" is NOT the forbidden IMDS host, so the broad permit wins → Granted.
    assert_eq!(
        state(effect_ct::HTTP),
        GrantState::Granted,
        "http: executor + broad permit"
    );

    // MODEL: mechanism (ModelExecutor) present, BUT the grant is scoped (`model == "claude-test"`) and the
    // kernel-default probe target for model is "" (no session-agnostic model id) → the decide-only probe
    // reads Denied-at-probe. This is the honest "maybe, at your real target — emit to find out" state, NOT
    // "never usable". The override case below proves it flips.
    assert_eq!(
        state(effect_ct::MODEL),
        GrantState::Denied,
        "model: executor present but scoped grant reads Denied at the empty default probe target"
    );

    // TIMER: policy PERMITS `timer` at any resource — but the host registered NO timer executor. The
    // mechanism axis is distinct from and dominates policy: no executor → Absent regardless of the permit.
    // This is the value of probing mechanism separately (a flat "policy allows" would mislead here).
    assert_eq!(
        state(effect_ct::TIMER),
        GrantState::Absent,
        "timer: policy permits it but there is no executor — mechanism axis → Absent"
    );

    // SHELL / EMIT: neither an executor nor a permit → Absent.
    assert_eq!(
        state(effect_ct::SHELL),
        GrantState::Absent,
        "shell: no executor"
    );
    assert_eq!(
        state(effect_ct::EMIT),
        GrantState::Absent,
        "emit: no executor"
    );
}

#[tokio::test]
async fn a_session_probe_target_override_flips_a_scoped_model_grant_to_granted() {
    let Some(bytes) = policy_component_bytes() else {
        eprintln!("SKIP capability_manifest_e2e (override case): CEDAR_POLICY_COMPONENT unset.");
        return;
    };

    let authz = ComponentAuthorizer::from_policy_bytes(&bytes, "agent://test")
        .expect("the lifted cedar-policy-guest is a valid authorizer component");
    let exec = real_host_executors();

    // I3 crux: a session that KNOWS its granted model id overrides the probe target for the model family
    // (the kernel default can't know a session-specific id). Probing `model` at "claude-test" — the exact
    // scoped grant — now reads Granted, while every other family keeps the kernel default. This is the
    // host-side seam the manifest was designed around: a decide-only authorizer CAN report a scoped grant
    // as usable once the host supplies the real target to probe.
    let probe_target = |family: &str| -> &'static str {
        if family == effect_ct::MODEL {
            "claude-test"
        } else {
            effect_ct::probe_target(family)
        }
    };

    let manifest = project_manifest(
        effect_ct::ALL,
        |f| exec.handles_family(f),
        &authz,
        probe_target,
    )
    .await;

    let model = manifest
        .entries
        .iter()
        .find(|e| e.family == effect_ct::MODEL)
        .expect("model family present")
        .grant
        .clone();
    assert_eq!(
        model,
        GrantState::Granted,
        "with the session's real model id as the probe target, the scoped grant reads Granted"
    );

    // The override is surgical: HTTP still probes at the kernel default and stays Granted (broad permit),
    // proving we didn't accidentally widen every family's probe.
    let http = manifest
        .entries
        .iter()
        .find(|e| e.family == effect_ct::HTTP)
        .expect("http family present")
        .grant
        .clone();
    assert_eq!(
        http,
        GrantState::Granted,
        "http unaffected by the model-only override"
    );
}
