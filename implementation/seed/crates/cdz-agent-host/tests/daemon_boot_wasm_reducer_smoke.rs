//! Real-daemon BOOT + INSTALL-A-**WASM-REDUCER** + LIVE-RUN smoke — the FULLER deployment proof.
//!
//! The sibling `daemon_boot_install_tick_smoke` proves the boot→admin-install→run control-plane chain, but
//! its factory yields a NATIVE reducer (a hand-written `TickRole`), standing in for the deployed reducer
//! SOURCE. This smoke closes that last gap: it boots the daemon with the REAL [`ComponentSessionFactory`]
//! over a [`MemBlobStore`] holding an actual WASM reducer component, then — through the ADMIN CHANNEL,
//! exactly as the socket listener does per frame — installs a session BY CONTENT HASH, so the factory
//! blob-fetches the bytes + LIFTS them into an [`AsyncComponentReducer`](cdz_kernel::wasm_host::AsyncComponentReducer)
//! (the deployed reducer-load seam, not a native stand-in), and the running loop drives that wasm program
//! LIVE to quiescence. This is "a real *compiled* agent boots + runs on the harness" — the self-hosting
//! endgame's load path exercised end to end through the daemon's own boot chain.
//!
//! GATED on `CDZ_LIVE_REDUCER_COMPONENT` (a path to a real wasm reducer component — the nix build produces
//! one): unset → SKIP cleanly (a plain `cargo test` has no wasm toolchain / artifact), so the hermetic
//! default gate is unaffected. The nix CI job that sets the var exercises the full blob→lift→run path. Same
//! skip/read contract as the other live-reducer e2es ([`common::reducer_component_bytes`]).
//!
//! It stays network-free: the per-session executor set is empty (`CompositeExecutor::new`) and the
//! per-session authorizer is deny-all — whatever effects the wasm reducer emits are denied/decline cleanly;
//! the point is that the LIFTED program RUNS one turn to quiescence through the deployed loop (§17 totality),
//! not what it's permitted to do. The clock is pinned so any timer the reducer arms fires promptly.

mod common;

use cdz_agent_host::{
    AdminChannel, AdminCommand, AdminRequest, AdminResponse, AgentHost, AllowList, AsyncAgentHost,
    ComponentSessionFactory, InstallSpec, SessionId,
};
use cdz_kernel::blob::{BlobStore, MemBlobStore};
use cdz_kernel::event::EventBody;
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use common::reducer_component_bytes;
use std::time::Duration;

/// A hard ceiling on the whole boot→install→run→drain arc: the installed session runs a REAL, externally
/// supplied wasm reducer, so a misbehaving/looping component could otherwise hang the suite. Bounding the
/// join surfaces a runaway as a clear timeout error, not a wedge — same discipline as the name_store e2es.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

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
async fn daemon_boots_admin_installs_a_wasm_reducer_by_hash_and_it_runs_live() {
    let Some(component) = reducer_component_bytes() else {
        eprintln!(
            "SKIP daemon_boot_wasm_reducer_smoke::daemon_boots_admin_installs_a_wasm_reducer_by_hash_\
             and_it_runs_live: CDZ_LIVE_REDUCER_COMPONENT unset — set it to a real wasm reducer component \
             (the nix build produces one) to exercise the boot→admin-install→blob-lift→run path."
        );
        return;
    };

    // The host loop + its sessions are `!Send` (single-threaded, §15b), so drive everything inside a
    // LocalSet where `spawn_local` is valid — the same shape the deployed daemon's `current_thread` runtime
    // + LocalSet provide.
    tokio::task::LocalSet::new()
        .run_until(smoke(component))
        .await;
}

async fn smoke(component: Vec<u8>) {
    // The reducer blob store the deployed daemon's install factory loads from (BlobConfig::Memory arm). Put
    // the real wasm component in it → content-addressed, so the hash we get back is the hash we install by.
    let mut blob = MemBlobStore::new();
    let reducer_hash = Hash::of(&component);
    blob.put(reducer_hash, bytes::Bytes::from(component.clone()))
        .await
        .expect("put the wasm reducer component into the blob store");

    // BOOT the REAL ComponentSessionFactory (the deployed reducer-load seam) — network-free: an empty
    // per-session executor set + a deny-all per-session authorizer (the daemon's v0 fail-closed default).
    // This is the same factory value the daemon binary builds for BlobConfig::Memory, minus the live
    // transports (irrelevant to the lift-and-run path this smoke proves).
    let factory = ComponentSessionFactory::new(
        blob,
        CompositeExecutor::new,
        || -> Box<dyn cdz_kernel::authz::Authorize> {
            Box::new(cdz_kernel::authz::Authorizer::deny_all())
        },
    );

    let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(factory))
        .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
    let admin = async_host.admin_channel();
    let inbox = async_host.inbox();

    // Spawn the loop on its own task, clock pinned past every deadline so any timer the reducer arms fires
    // promptly. A shutdown signal ends the loop once it drains (below).
    let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
    let loop_task =
        tokio::task::spawn_local(async move { async_host.run(sd_rx, || u64::MAX).await });

    // INSTALL a session BY CONTENT HASH through the admin channel (exactly what the socket listener does per
    // frame). The factory blob-fetches `reducer_hash` + LIFTS the bytes into a runnable AsyncComponentReducer
    // — a missing blob or a non-lifting component would surface here as AdminResponse::Error, failing loud.
    let id = "wasm-worker-1";
    let resp = admin_call(
        &admin,
        AdminCommand::InstallSession(InstallSpec {
            id: SessionId::new(id),
            reducer_hash,
            goal: Some("run the compiled reducer".to_string()),
        }),
    )
    .await;
    assert!(
        matches!(&resp, AdminResponse::Installed { id: got } if got.as_str() == id),
        "the daemon lifted the wasm reducer from the blob store + installed the session, got {resp:?}"
    );

    // KICK it: deliver one inbound through the loop's inbox (the producer path). The lifted wasm reducer
    // folds one live turn — whatever it emits is denied/declined by the deny-all authorizer + empty executor
    // set; the point is that the LIFTED program RUNS through the deployed loop without panicking (§17).
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

    // Signal shutdown + drop the producer handles so the loop drains after the kick's turn settles. (Unlike
    // the native tick smoke, this reducer is externally supplied — we don't assume a self-stopping budget, so
    // an explicit shutdown bounds the run.)
    let _ = sd_tx.send(());
    drop(admin);
    drop(inbox);

    // The loop drains cleanly (the wasm reducer's kick turn ran to quiescence, then shutdown). Bounded so a
    // runaway lifted component is a clear timeout, not a hung suite.
    let host = tokio::time::timeout(RUN_TIMEOUT, loop_task)
        .await
        .expect(
            "the boot→install→run→drain arc completes within RUN_TIMEOUT (no runaway wasm reducer)",
        )
        .expect("loop task joined")
        .expect("the loop drained cleanly after running the lifted wasm reducer");

    // The admin-installed WASM reducer is registered + ran live through the deployed loop: the deployed
    // reducer-load seam (blob-fetch → lift → run) worked end to end through the daemon's own boot chain.
    let session = host
        .get(&SessionId::new(id))
        .expect("the wasm worker is registered after its live turn");
    assert_eq!(
        session.open_effects(),
        0,
        "the lifted wasm reducer's kick turn settled (any effects it emitted resolved/denied — it RAN)"
    );
}
