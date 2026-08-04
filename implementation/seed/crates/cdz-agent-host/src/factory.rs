//! The real [`SessionFactory`] — turns an admin `install-session` command into a running agent by LOADING
//! its reducer from the blob store.
//!
//! Building a reducer from a `reducer_hash` means loading its wasm component, which is a HOST concern — the
//! kernel exposes the pieces ([`BlobStore::get`] + [`AsyncComponentReducer::from_component_bytes`]) and the
//! host assembles them behind the [`SessionFactory`] seam. This is that assembly:
//! [`ComponentSessionFactory`] holds a [`BlobStore`] + the per-session executor set, and on each install it
//!
//! 1. fetches the reducer component bytes by content hash (`blob.get(reducer_hash)`),
//! 2. lifts them into an [`AsyncComponentReducer`] (`from_component_bytes` — the kernel's reducer-from-bytes
//!    seam), and
//! 3. assembles a [`HostedSession`] via [`HostedSession::genesis`] with that reducer + a fresh copy of the
//!    executor set + the session's authorizer.
//!
//! It is GENERIC over the blob store (so a test drives a [`MemBlobStore`](cdz_kernel::blob::MemBlobStore)
//! with a real component + the deployed daemon a durable backend) and takes the executor set + authorizer
//! as caller-supplied builders, so the factory itself stays hermetically testable — the network/AWS
//! executor tree lives in the daemon's `live_executor_set()` (behind `live-net`), NOT here. A malformed or
//! absent component is a clean `Err(String)` (surfaced as an [`AdminResponse::Error`](crate::AdminResponse)),
//! never a panic.

use crate::admin::{InstallSpec, SessionFactory};
use crate::host::HostedSession;
use cdz_kernel::authz::Authorize;
use cdz_kernel::blob::BlobStore;
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::wasm_host::AsyncComponentReducer;

/// Builds a per-session executor set (the effects a freshly-installed session may perform). Called ONCE per
/// install so each session gets its own [`CompositeExecutor`] (executors hold per-session transport state);
/// the deployed daemon returns the live set (Clock + HTTP + Bedrock), a test returns a hermetic one.
pub trait ExecutorSetBuilder {
    fn build(&self) -> CompositeExecutor;
}

impl<F: Fn() -> CompositeExecutor> ExecutorSetBuilder for F {
    fn build(&self) -> CompositeExecutor {
        self()
    }
}

/// Builds a per-session authorizer (the policy gating what an installed session may do). Called ONCE per
/// install. The deployed daemon derives it from the configured policy; a test returns a fixed one. Kept
/// separate from the executor set because "what a session CAN do" (mechanism) and "what it MAY do" (policy)
/// are the two independent axes the kernel authorizes on.
pub trait AuthorizerBuilder {
    fn build(&self) -> Box<dyn Authorize>;
}

impl<F: Fn() -> Box<dyn Authorize>> AuthorizerBuilder for F {
    fn build(&self) -> Box<dyn Authorize> {
        self()
    }
}

/// The real [`SessionFactory`]: load a reducer component from a [`BlobStore`] by hash and assemble a live
/// [`HostedSession`]. Generic over the blob store; the executor set + authorizer come from caller-supplied
/// builders (so the network/AWS tree stays in the daemon, not this hermetically-testable factory).
pub struct ComponentSessionFactory<B, E, A> {
    blob: B,
    executors: E,
    authz: A,
}

impl<B, E, A> ComponentSessionFactory<B, E, A>
where
    B: BlobStore,
    E: ExecutorSetBuilder,
    A: AuthorizerBuilder,
{
    /// Assemble the factory over a blob store + the per-session executor-set / authorizer builders.
    pub fn new(blob: B, executors: E, authz: A) -> Self {
        ComponentSessionFactory {
            blob,
            executors,
            authz,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl<B, E, A> SessionFactory for ComponentSessionFactory<B, E, A>
where
    B: BlobStore,
    E: ExecutorSetBuilder,
    A: AuthorizerBuilder,
{
    async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String> {
        // 1. Fetch the reducer component bytes by content hash. Absent = a clean error (the admin asked to
        //    install a reducer the store doesn't have), not a panic.
        let bytes = self
            .blob
            .get(&spec.reducer_hash)
            .await
            .map_err(|e| {
                format!(
                    "blob store error fetching reducer {}: {e}",
                    spec.reducer_hash
                )
            })?
            .ok_or_else(|| {
                format!(
                    "no reducer component in the blob store for hash {}",
                    spec.reducer_hash
                )
            })?;
        // 2. Lift the bytes into an async reducer. A malformed / non-fold / dep-declaring component is a
        //    clean decline (ComponentError → String), never a panic.
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes).map_err(|e| {
            format!(
                "reducer component for {} did not lift: {e:?}",
                spec.reducer_hash
            )
        })?;
        // 3. Assemble the session with a fresh executor set + the session's authorizer. genesis records the
        //    reducer hash as the session's genesis identity (so replay reconstructs it).
        Ok(HostedSession::genesis(
            spec.reducer_hash,
            Box::new(reducer),
            self.authz.build(),
            self.executors.build(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{AdminCommand, AdminResponse};
    use crate::host::{AgentHost, SessionId};
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::blob::MemBlobStore;
    use cdz_kernel::hash::Hash;

    /// The reducer-guest fold-world component bytes, if `CDZ_REDUCER_COMPONENT` points at one (CI builds +
    /// exports it; absent locally → the test skips). Distinguishes "var UNSET → skip" from "var SET but the
    /// file is unreadable → PANIC" (#1979 review): a silent `.ok()` on the read would mask a CI misconfig
    /// (the var set to a bad path) as a green skip. If the var is set, the component MUST be readable.
    fn reducer_component_bytes() -> Option<Vec<u8>> {
        let path = std::env::var("CDZ_REDUCER_COMPONENT").ok()?;
        Some(std::fs::read(&path).unwrap_or_else(|e| {
            panic!("CDZ_REDUCER_COMPONENT is set to {path} but the component is unreadable: {e}")
        }))
    }

    fn hermetic_executors() -> CompositeExecutor {
        // Now-only hermetic executor set (no network) — the factory is generic over the set, so a test
        // needs no live-net tree.
        use cdz_kernel::effect::effect_ct;
        CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(crate::ClockExecutor::new()))
    }

    fn deny_all_authz() -> Box<dyn Authorize> {
        Box::new(Authorizer::deny_all())
    }

    #[tokio::test]
    async fn install_of_an_absent_reducer_hash_is_a_clean_error() {
        // The blob store is empty → get returns None → build errors cleanly (no panic), and apply_admin
        // surfaces it as an AdminResponse::Error with the registry untouched.
        let mut host = AgentHost::new();
        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let spec = InstallSpec {
            id: SessionId::new("ghost"),
            reducer_hash: Hash::of(b"nonexistent-reducer"),
            goal: None,
        };
        let resp = host
            .apply_admin(AdminCommand::InstallSession(spec), Some(&mut factory), None)
            .await;
        assert!(
            matches!(&resp, AdminResponse::Error { message } if message.contains("no reducer component in the blob store")),
            "absent reducer → clean error: {resp:?}"
        );
        assert!(host.is_empty(), "nothing installed for an absent reducer");
    }

    #[tokio::test]
    async fn install_of_non_component_bytes_is_a_clean_error() {
        // Bytes present but NOT a valid component → from_component_bytes declines → clean error, no panic.
        let mut blob = MemBlobStore::new();
        let garbage = b"this is not a wasm component".to_vec();
        let hash = blob.put(&garbage).await.unwrap();
        let mut factory = ComponentSessionFactory::new(blob, hermetic_executors, deny_all_authz);
        let spec = InstallSpec {
            id: SessionId::new("bad"),
            reducer_hash: hash,
            goal: None,
        };
        let err = match factory.build(&spec).await {
            Ok(_) => panic!("non-component bytes must not lift into a session"),
            Err(e) => e,
        };
        assert!(err.contains("did not lift"), "{err}");
    }

    #[tokio::test]
    async fn install_of_a_real_reducer_component_runs_an_agent() {
        // The end-to-end payoff: a REAL reducer component in the blob store → install builds a live session
        // that a subsequent inbound ACTUALLY DRIVES. Env-gated on a built component fixture (skips locally).
        // The reducer-guest fixture, on a `message` inbound, bumps a `count` KV counter (then requests an
        // Http effect); we deliver one message + assert `count` was written — proving the reducer RAN, not
        // merely that a session registered (#1979 review: the prior version stopped at `contains`).
        let Some(bytes) = reducer_component_bytes() else {
            eprintln!("CDZ_REDUCER_COMPONENT unset — skipping the real-component install test");
            return;
        };
        let mut blob = MemBlobStore::new();
        let hash = blob.put(&bytes).await.unwrap();
        // Grant the session Http so the fold's requested effect authorizes (the KV `count` write happens in
        // the fold regardless, but granting Http keeps the turn from erroring on an AuthzDenied).
        let authz = || -> Box<dyn Authorize> {
            use cdz_kernel::effect::{Capability, EffectKind, ResourcePredicate};
            Box::new(Authorizer::new(vec![Capability {
                kind: EffectKind::Http,
                predicate: ResourcePredicate::Any,
            }]))
        };
        let mut factory = ComponentSessionFactory::new(blob, hermetic_executors, authz);

        let mut host = AgentHost::new();
        let resp = host
            .apply_admin(
                AdminCommand::InstallSession(InstallSpec {
                    id: SessionId::new("real"),
                    reducer_hash: hash,
                    goal: None,
                }),
                Some(&mut factory),
                None,
            )
            .await;
        assert_eq!(
            resp,
            AdminResponse::Installed {
                id: SessionId::new("real")
            },
            "a real reducer component installs"
        );

        // Drive the installed session: deliver a `message` inbound → the reducer's fold runs → `count`=1.
        use cdz_kernel::effect::Payload;
        use cdz_kernel::event::{ContentType, EventBody};
        let outcome = host
            .deliver(
                &SessionId::new("real"),
                EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"hello".to_vec().into()),
                },
                None,
            )
            .await;
        assert!(
            matches!(outcome, Some(Ok(()))),
            "the installed real reducer ran a turn: {outcome:?}"
        );
        // The reducer WROTE to its own KV during the fold — proof it actually executed (not just registered).
        assert_eq!(
            host.get(&SessionId::new("real"))
                .unwrap()
                .session()
                .kv()
                .get(b"count"),
            Some(&[1u8][..]),
            "the reducer's fold bumped its count counter — the agent ran"
        );
    }
}
