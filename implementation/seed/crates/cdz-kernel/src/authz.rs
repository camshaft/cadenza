//! Authorization — the kernel's one gate before an effect reaches an executor (§2, §12c).
//!
//! v0 is a capability-set authorizer: an effect is permitted iff some held [`Capability`] permits it
//! (kind matches AND its resource predicate admits the resolved target — the SEC-F1 fix). In the full
//! design the authorizer is a swappable wasm component (Cedar as one impl, §1); v0 keeps it a plain
//! in-kernel check so the loop is testable. The swap seam is the [`Authorize`] TRAIT: the kernel gates
//! effects through `&dyn Authorize`, so a Cedar/delegation authorizer drops in later without touching
//! the kernel core. [`Authorizer`] is the v0 impl.

use crate::effect::{Capability, EffectRequest};

/// The authorization SEAM (§1/§12c): the kernel gates every effect through this, and takes it as
/// `&dyn Authorize` so the decision engine is swappable WITHOUT touching the kernel core — a Cedar
/// policy component (§20b), a delegation/attenuation authorizer (§12f), or the v0 [`Authorizer`] below
/// all satisfy it. The design's "the authorizer is a swappable component" is this trait; making it real
/// (kernel takes `&dyn Authorize`, not the concrete struct) is what lets the swap happen later with no
/// kernel change.
///
/// Contract: total + PURE — it inspects the request and the held policy, performs no I/O and no
/// mutation, and returns a decision. `Ok(())` to permit; `Err(reason)` to deny (the reason is logged in
/// the `AuthzDenied` event, §10). It must not panic (§17).
pub trait Authorize {
    /// Is this request permitted? `Ok(())` to proceed to dispatch; `Err(reason)` to deny — the denied
    /// request never reaches an executor and the reason is recorded.
    fn authorize(&self, req: &EffectRequest) -> Result<(), String>;
}

/// The ASYNC authorization seam — the async counterpart of [`Authorize`] (operator all-async directive).
///
/// A wasm-component authorizer ([`crate::wasm_host::ComponentAuthorizer`], Cedar-as-wasm) evaluates a
/// policy by instantiating + calling a wasm guest — a call that, on the all-async engine, can cooperatively
/// yield. So the authz gate becomes awaitable too (no residual sync interface — the operator's "all async,
/// no sync at all"). Introduced ADDITIVELY alongside sync [`Authorize`] (nothing in the kernel loop
/// consumes it yet; the async drive loop switches to it in the same follow-up slice as the async executor,
/// once the authorizer impls are migrated). Removed with the sync path at the end.
///
/// **Object-safe via `async-trait`.** The kernel gates through `&dyn Authorize`, so the async trait stays
/// dyn-compatible via `#[async_trait(?Send)]`; `?Send` for the single-threaded kernel (a ComponentAuthorizer
/// holds a non-`Send` wasmtime store). A sync [`Authorize`] wraps in [`SyncAuthorizeAsAsync`] (explicit
/// adapter, not a blanket — same coherence reason as [`crate::reducer::SyncAsAsync`]).
#[async_trait::async_trait(?Send)]
pub trait AsyncAuthorize {
    /// Is this request permitted? Async counterpart of [`Authorize::authorize`] — same total/pure contract
    /// (`Ok(())` permit, `Err(reason)` deny, no panic); may `.await` a wasm policy evaluation internally.
    async fn authorize_async(&self, req: &EffectRequest) -> Result<(), String>;
}

/// Adapt any sync [`Authorize`] into an [`AsyncAuthorize`] (its `authorize_async` runs the sync `authorize`
/// — no await point, correct for the in-kernel capability check). The explicit alternative to a blanket
/// impl (so a genuinely-async authorizer, e.g. a wasm ComponentAuthorizer, writes its own `AsyncAuthorize`
/// without a coherence collision). Wrap a sync authorizer — `SyncAuthorizeAsAsync(MyAuthorizer)` — to gate
/// through the async kernel loop.
pub struct SyncAuthorizeAsAsync<A>(pub A);

#[async_trait::async_trait(?Send)]
impl<A: Authorize> AsyncAuthorize for SyncAuthorizeAsAsync<A> {
    async fn authorize_async(&self, req: &EffectRequest) -> Result<(), String> {
        self.0.authorize(req)
    }
}

/// Decides whether a requested effect may be performed. The result is logged either way (§10): a
/// permitted effect proceeds to dispatch; a denied one becomes an `AuthzDenied` event and never runs.
/// The v0 [`Authorize`] impl: a flat capability-set check (SEC-F1). A future Cedar/delegation authorizer
/// implements the same trait and drops into the kernel unchanged.
pub struct Authorizer {
    caps: Vec<Capability>,
}

impl Authorizer {
    /// An authorizer holding a fixed capability set (the session's grants). Delegation/attenuation
    /// (§12f) layers on later; v0 is a flat grant set for one operator.
    pub fn new(caps: Vec<Capability>) -> Self {
        Authorizer { caps }
    }

    /// Grant nothing — every effect is denied. Useful for pure-fold reducers that should have no
    /// ambient authority (the §9c "deny the clock entirely" case).
    pub fn deny_all() -> Self {
        Authorizer { caps: Vec::new() }
    }
}

impl Authorize for Authorizer {
    /// SEC-F1: permission requires a capability whose predicate admits the *resolved target*.
    fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
        if self.caps.iter().any(|c| c.permits(req)) {
            Ok(())
        } else {
            Err(format!(
                "no capability permits {:?} to target {:?}",
                req.kind, req.target
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectKind, ResourcePredicate};

    fn req(kind: EffectKind, target: &str) -> EffectRequest {
        EffectRequest {
            kind,
            target: target.to_string(),
            payload: None,
            timeliness: crate::effect::Timeliness::Interactive,
        }
    }

    #[test]
    fn permits_only_scoped_targets() {
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
        }]);
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://ok.host/x"))
            .is_ok());
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://evil.host/x"))
            .is_err());
    }

    #[test]
    fn deny_all_denies_everything() {
        let authz = Authorizer::deny_all();
        assert!(authz.authorize(&req(EffectKind::Now, "")).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sync_authorizer_wrapped_in_adapter_gates_via_async_trait() {
        // A sync Authorize wrapped in SyncAuthorizeAsAsync gates via the async path (authorize_async runs
        // the sync authorize). Exercise through &dyn AsyncAuthorize (object-safety) — via the adapter.
        let authz = SyncAuthorizeAsAsync(Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
        }]));
        let dyn_authz: &dyn AsyncAuthorize = &authz;
        assert!(dyn_authz
            .authorize_async(&req(EffectKind::Http, "https://ok.host/x"))
            .await
            .is_ok());
        assert!(dyn_authz
            .authorize_async(&req(EffectKind::Http, "https://evil.host/x"))
            .await
            .is_err());
    }

    #[test]
    fn a_custom_authorize_impl_satisfies_the_seam() {
        // The point of the trait (§1 swap seam): a NON-Authorizer decision engine drops in as
        // `&dyn Authorize` with no kernel change. A trivial policy — permit only `Now` — proves it.
        struct OnlyNow;
        impl Authorize for OnlyNow {
            fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
                if req.kind == EffectKind::Now {
                    Ok(())
                } else {
                    Err("only Now permitted".into())
                }
            }
        }
        let authz: &dyn Authorize = &OnlyNow;
        assert!(authz.authorize(&req(EffectKind::Now, "")).is_ok());
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://ok.host/x"))
            .is_err());
    }
}
