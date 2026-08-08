//! End-to-end (env-gated): §20b policy-referenced-by-mutable-name — a LIVE policy swap through the host.
//!
//! A privileged admin publishes a new policy by `store/set`ting [`NameStore::POLICY_CURRENT`] → a Cedar
//! policy-component blob hash; the host resolves that pointer, blob-gets the component, and
//! [`HostedSession::reload_policy_from_component_bytes`] lifts it into a `ComponentAuthorizer`, swaps the
//! session's authorizer, and pushes a `capabilities-changed`. This test drives the host half directly with
//! the Cedar guest component (the resolve/blob-get are host mechanics proven elsewhere): a session that
//! STARTS deny-all live-swaps to the Cedar policy (permit now/timer/http broad, model at claude-test) and
//! its capability manifest MOVES — proving a policy swap is observable to the agent.
//!
//! GATED on `CEDAR_POLICY_COMPONENT` (the lifted Cedar guest the cdz-agent-host CI job builds) — unset →
//! SKIP cleanly, same discipline as `cedar_authz_e2e` / `capability_manifest_e2e`.

mod common;

use cdz_agent_host::HostedSession;
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{effect_ct, Payload};
use cdz_kernel::event::{EffectOutcome, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};
use common::policy_component_bytes;

/// Records any folded `EffectResult` inline payload under `capabilities` — so the test can see whether a
/// policy swap folded a capabilities-changed manifest to the agent.
struct CapabilityAwareAgent;
#[async_trait::async_trait(?Send)]
impl Reducer for CapabilityAwareAgent {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        if let EventBody::EffectResult {
            result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
            ..
        } = &event.body
        {
            kv.put(b"capabilities".to_vec(), bytes.to_vec());
        }
        FoldOutput::none()
    }
}

#[tokio::test]
async fn a_policy_swap_via_reload_flips_the_authorizer_and_pushes_a_capabilities_change() {
    let Some(policy) = policy_component_bytes() else {
        eprintln!(
            "SKIP policy_swap_e2e::a_policy_swap_via_reload_flips_the_authorizer_and_pushes_a_capabilities_\
             change: CEDAR_POLICY_COMPONENT unset — set it to the lifted Cedar guest to exercise the live \
             §20b policy swap."
        );
        return;
    };

    // Start deny-all: the session refuses everything, and its born-knowing manifest reflects that. The
    // executor serves Now (the mechanism axis is present for that family); only the POLICY denies.
    let mut hosted = HostedSession::genesis(
        Hash::of(b"policy-swap-v1"),
        Box::new(CapabilityAwareAgent),
        Box::new(Authorizer::deny_all()),
        CompositeExecutor::new().with_effect(
            effect_ct::NOW,
            Box::new(cdz_agent_host::ClockExecutor::new()),
        ),
    );
    hosted.seed_capabilities().await;
    let manifest_under_deny_all = hosted
        .session()
        .kv()
        .get(b"capabilities")
        .expect("seeded baseline manifest")
        .to_vec();

    // LIVE SWAP: reload the session's policy from the Cedar guest component (the bytes a POLICY_CURRENT
    // resolve → blob-get would hand the host). This rebuilds the authorizer + pushes a capabilities-changed.
    let pushed = hosted
        .reload_policy_from_component_bytes(&policy, "agent://policy-swap-v1")
        .await
        .expect("the Cedar guest lifts into an authorizer");
    assert!(
        pushed.is_empty(),
        "the capabilities-changed push is answered inline"
    );

    // The manifest MOVED: deny-all → the Cedar policy (which permits now/http/timer broadly) is a different
    // grant surface, so the recorded manifest differs from the deny-all baseline.
    let manifest_after_swap = hosted
        .session()
        .kv()
        .get(b"capabilities")
        .expect("a capabilities-changed folded after the swap")
        .to_vec();
    assert_ne!(
        manifest_after_swap, manifest_under_deny_all,
        "swapping deny-all → the Cedar policy changed the session's capability manifest (policy swap is \
         observable to the agent)"
    );

    // Independent confirmation that the manifest MOVED THE RIGHT WAY: the post-swap manifest equals the one
    // the kernel projects over this session's served surface (Now) against the Cedar policy — i.e. the swap
    // installed exactly that policy, not "some other bytes". (project against the same ComponentAuthorizer.)
    let expected_after = {
        let exec = CompositeExecutor::new().with_effect(
            effect_ct::NOW,
            Box::new(cdz_agent_host::ClockExecutor::new()),
        );
        let authz = cdz_kernel::wasm_host::ComponentAuthorizer::from_policy_bytes(
            &policy,
            "agent://policy-swap-v1",
        )
        .expect("cedar guest lifts");
        let manifest = cdz_kernel::effect::project_manifest(
            effect_ct::ALL,
            |f| exec.handles_family(f),
            &authz,
            effect_ct::probe_target,
        )
        .await;
        cdz_kernel::event_ast::encode_capability_manifest(&manifest)
    };
    assert_eq!(
        manifest_after_swap, expected_after,
        "the pushed manifest is exactly the one the newly-swapped Cedar policy projects (the swap installed \
         THAT policy, not merely 'changed')"
    );
}
