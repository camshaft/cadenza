//! Real-daemon BOOT + INSTALL + LIVE-TICK smoke (the deployment proof beyond the hermetic unit e2es).
//!
//! The hermetic e2es (agent_tool_call_e2e, self_hosting_tick_loop_e2e) drive a Session/loop DIRECTLY. This
//! smoke exercises the actual DEPLOYED-daemon chain: boot the [`AsyncAgentHost`] loop as a pure control plane
//! (empty registry) with a [`SessionFactory`] install seam + the admin control interface, then — through the
//! ADMIN CHANNEL, exactly as the socket listener does per frame — INSTALL a session at runtime, and let the
//! running loop drive it LIVE (its self-re-arming tick fires through the loop's timer wheel). This is the
//! "a real agent boots + ticks on the harness" demonstration the big goal needs: it moves the proof from
//! "the pieces work when wired by hand in a test" to "the daemon's own boot→admin-install→run path runs an
//! agent end to end."
//!
//! It stays hermetic (no network / no AWS): the installed agent is a TIMER-only tick-loop role (needs no
//! model), and the factory yields it natively (a real ComponentSessionFactory would load a wasm reducer from
//! a blob store — that heavier path is the wasm-reducer smoke, a follow-on; this proves the boot+admin+run
//! control-plane chain). The clock is pinned past every deadline so the live ticks fire promptly.

use cdz_agent_host::{
    AdminChannel, AdminCommand, AdminRequest, AdminResponse, AgentHost, AllowList, AsyncAgentHost,
    HostedSession, InstallSpec, SessionFactory, SessionId,
};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectKind, EffectRequest, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{Event, EventBody};
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};

const TICK_BUDGET: u64 = 3;

/// The installed agent: a self-re-arming tick-loop role (the fleet.rs re-issue shape). Arms a timer on the
/// install kick, re-arms on each fire until a budget, records progress in durable KV. Timer-only → hermetic.
struct TickRole;
impl TickRole {
    fn arm() -> EffectRequest {
        EffectRequest::new_with_family(effect_ct::TIMER, "1000", None, Timeliness::Interactive)
    }
    fn ticks(kv: &Kv) -> u64 {
        kv.get(b"ticks")
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }
}
#[async_trait::async_trait(?Send)]
impl Reducer for TickRole {
    async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => FoldOutput::with(vec![Self::arm()]),
            EventBody::TimerFired { .. } => {
                let n = Self::ticks(kv) + 1;
                kv.put(b"ticks".to_vec(), n.to_string().into_bytes());
                if n < TICK_BUDGET {
                    FoldOutput::with(vec![Self::arm()])
                } else {
                    kv.put(b"done".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
            }
            _ => FoldOutput::none(),
        }
    }
}

/// A minimal [`SessionFactory`] that installs the [`TickRole`] with a `timer` grant — standing in for the
/// deployed [`ComponentSessionFactory`](cdz_agent_host::ComponentSessionFactory) (which loads a wasm reducer
/// from a blob store). The boot→admin-install→run chain it exercises is identical; only the reducer SOURCE
/// (native here vs blob-loaded wasm in prod) differs — the wasm-reducer boot is a heavier follow-on smoke.
struct TickRoleFactory;
#[async_trait::async_trait(?Send)]
impl SessionFactory for TickRoleFactory {
    async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String> {
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }]);
        Ok(HostedSession::genesis(
            spec.reducer_hash,
            Box::new(TickRole),
            Box::new(authz),
            cdz_kernel::executor::CompositeExecutor::new(),
        ))
    }
    async fn build_spawned(
        &mut self,
        _reducer_hash: Hash,
        _parent_genesis: Hash,
        _spawn_nonce: Hash,
    ) -> Result<HostedSession, String> {
        Err("TickRoleFactory does not spawn children".to_string())
    }
}

async fn admin_call(ch: &AdminChannel, command: AdminCommand) -> AdminResponse {
    let (reply, rx) = tokio::sync::oneshot::channel();
    ch.send(AdminRequest {
        command,
        principal: Some("admin".to_string()),
        reply,
    })
    .expect("loop accepts the admin request");
    rx.await.expect("the loop replied")
}

#[tokio::test]
async fn daemon_boots_admin_installs_a_role_and_it_ticks_live_through_the_loop() {
    // The host loop + its sessions are `!Send` (single-threaded, §15b), so drive everything inside a
    // LocalSet where `spawn_local` is valid — the same shape the deployed daemon's `current_thread` runtime
    // + LocalSet provide.
    tokio::task::LocalSet::new().run_until(smoke()).await;
}

async fn smoke() {
    // BOOT: the control-plane daemon shape — empty registry + a factory (install seam) + admin authorizer
    // granting the local admin (the socket-owner-gate stands in for the 0o600 socket in this in-process test).
    let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(TickRoleFactory))
        .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
    let admin = async_host.admin_channel();
    let inbox = async_host.inbox();

    // Spawn the loop on its own task, clock pinned past every deadline so live ticks fire promptly. The loop
    // runs until all producers (admin + inbox) are dropped AND no timer is armed — i.e. once the role stops
    // re-arming at its budget + we drop the channels, it drains + returns.
    let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
    let loop_task =
        tokio::task::spawn_local(async move { async_host.run(sd_rx, || u64::MAX).await });

    // INSTALL a session at runtime through the admin channel (exactly what the socket listener does per frame).
    let id = "worker-1";
    let resp = admin_call(
        &admin,
        AdminCommand::InstallSession(InstallSpec {
            id: SessionId::new(id),
            reducer_hash: Hash::of(b"tick-role-v1"),
            goal: Some("run your tick loop".to_string()),
        }),
    )
    .await;
    assert!(
        matches!(&resp, AdminResponse::Installed { id: got } if got.as_str() == id),
        "the daemon installed the session, got {resp:?}"
    );

    // KICK the installed agent's loop: deliver the start-inbound through the loop's inbox (the producer path).
    // The role arms its first timer; the loop's timer wheel then fires it repeatedly (clock past deadline) →
    // the role re-arms → ticks live to its budget.
    inbox
        .send(cdz_agent_host::Inbound {
            session: SessionId::new(id),
            body: EventBody::Inbound {
                content_type: cdz_kernel::event::ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: cdz_kernel::effect::Payload::Inline(b"go".to_vec().into()),
            },
            cause: None,
            reply_to: None,
        })
        .expect("inbox accepts the kick");

    // Drop the producer handles so the loop can drain once the role stops re-arming (budget reached).
    drop(admin);
    drop(inbox);

    // The loop runs to completion (role stopped re-arming → no timer + closed producers → drains).
    let host = loop_task
        .await
        .expect("loop task joined")
        .expect("loop drained cleanly");

    // The installed agent TICKED LIVE through the deployed loop, to its budget.
    let session = host
        .get(&SessionId::new(id))
        .expect("worker still registered");
    let kv = session.session().kv();
    assert_eq!(
        TickRole::ticks(kv),
        TICK_BUDGET,
        "the admin-installed agent ran its full tick budget live through the daemon loop"
    );
    assert_eq!(
        kv.get(b"done").map(|v| v.to_vec()),
        Some(b"1".to_vec()),
        "the live agent reached its stop condition"
    );
}
