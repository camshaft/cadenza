//! Authorization — the kernel's one gate before an effect reaches an executor (§2, §12c).
//!
//! v0 is a capability-set authorizer: an effect is permitted iff some held [`Capability`] permits it
//! (kind matches AND its resource predicate admits the resolved target — the SEC-F1 fix). In the full
//! design the authorizer is a swappable wasm component (Cedar as one impl, §1); v0 keeps it a plain
//! in-kernel check so the loop is testable. The swap seam is the [`Authorize`] TRAIT: the kernel gates
//! effects through `&dyn Authorize`, so a Cedar/delegation authorizer drops in later without touching
//! the kernel core. [`Authorizer`] is the v0 impl.

use crate::effect::{Capability, EffectRequest};

/// The authorization SEAM (§1/§12c): the kernel gates every effect through this, taken as
/// `&dyn Authorize` so the decision engine is swappable WITHOUT touching the kernel core — a Cedar
/// policy component (§20b), a delegation/attenuation authorizer (§12f), or the v0 [`Authorizer`] below all
/// satisfy it. There is ONE authorizer trait, and it is async (operator ruling: "one async trait only").
///
/// Contract: total + PURE — it inspects the request and the held policy, performs no mutation, and returns
/// a decision. `Ok(())` to permit; `Err(reason)` to deny (logged in the `AuthzDenied` event, §10). Must not
/// panic (§17). A wasm-component authorizer ([`crate::wasm_host::ComponentAuthorizer`], Cedar-as-wasm) may
/// `.await` a fuel-yielding policy evaluation internally; an in-kernel check like [`Authorizer`] just
/// returns (an `async fn` with no `.await`).
///
/// **Object-safe via `async-trait`.** The kernel gates through `&dyn Authorize`, so the trait stays
/// dyn-compatible via `#[async_trait(?Send)]`; `?Send` for the single-threaded kernel (a ComponentAuthorizer
/// holds a non-`Send` wasmtime store).
#[async_trait::async_trait(?Send)]
pub trait Authorize {
    /// Is this request permitted? `Ok(())` to proceed to dispatch; `Err(reason)` to deny — the denied
    /// request never reaches an executor and the reason is recorded. Total + pure; may `.await` a wasm
    /// policy evaluation internally. The un-suffixed name (the trait is `async`, so an `_async` suffix
    /// would be redundant).
    async fn authorize(&self, req: &EffectRequest) -> Result<(), String>;
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

#[async_trait::async_trait(?Send)]
impl Authorize for Authorizer {
    /// SEC-F1: permission requires a capability whose predicate admits the *resolved target*. Native async
    /// (no `.await` — a flat capability-set check does no I/O).
    async fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
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
        EffectRequest::new(kind, target, None, crate::effect::Timeliness::Interactive)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permits_only_scoped_targets() {
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
        }]);
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://ok.host/x"))
            .await
            .is_ok());
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://evil.host/x"))
            .await
            .is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authz_gates_on_content_type_family_not_the_kind_enum() {
        // The seq-39 seam's authz half: a grant permits by the effect FAMILY string
        // (Capability.kind.family() vs req.content_type.family), NOT EffectKind enum equality. Prove it by
        // divorcing kind from family — an Http-kind request whose content_type.family is "model" is NOT
        // permitted by an Http grant (family mismatch), and IS permitted by a Model grant, regardless of
        // the enum. (Via `new` they agree; register-by-string will let a family stand alone — pin it now.)
        let http_grant = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let model_grant = Authorizer::new(vec![Capability {
            kind: EffectKind::Model,
            predicate: ResourcePredicate::Any,
        }]);
        let mut r = req(EffectKind::Http, "x");
        r.content_type.family = EffectKind::Model.family().into();
        // The Http grant's family ("http") no longer matches the request's family ("model") → denied...
        assert!(http_grant.authorize(&r).await.is_err());
        // ...and the Model grant's family ("model") matches → permitted, despite kind == Http.
        assert!(model_grant.authorize(&r).await.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deny_all_denies_everything() {
        let authz = Authorizer::deny_all();
        assert!(authz.authorize(&req(EffectKind::Now, "")).await.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_custom_authorize_impl_satisfies_the_seam() {
        // The point of the trait (§1 swap seam): a NON-Authorizer decision engine drops in as
        // `&dyn Authorize` with no kernel change. A trivial policy — permit only `Now` — proves it.
        struct OnlyNow;
        #[async_trait::async_trait(?Send)]
        impl Authorize for OnlyNow {
            async fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
                if req.kind == EffectKind::Now {
                    Ok(())
                } else {
                    Err("only Now permitted".into())
                }
            }
        }
        let authz: &dyn Authorize = &OnlyNow;
        assert!(authz.authorize(&req(EffectKind::Now, "")).await.is_ok());
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://ok.host/x"))
            .await
            .is_err());
    }
}
