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
    /// Register-by-string grants for families with no built-in [`crate::effect::EffectKind`] (`store/*` §4c, any extension
    /// family) — see [`Capability::for_family`]/[`crate::effect::FamilyGrant`]. Separate from `caps` so the
    /// `Capability{kind,predicate}` struct + its 45 literals stay untouched (additive); `authorize` admits a
    /// request permitted by EITHER list. Empty unless populated via [`Authorizer::with_family_grants`].
    family_grants: Vec<crate::effect::FamilyGrant>,
    /// Explicit DENY rules that OVERRIDE any grant — the "deny wins" precedence (standard security model:
    /// an explicit deny beats an allow). A request matching ANY deny rule is refused even if a `Capability`
    /// or `FamilyGrant` would permit it. Keyed by family + [`ResourcePredicate`](crate::effect::ResourcePredicate)
    /// like a grant (reuses [`crate::effect::FamilyGrant`], so a deny of `http` to `HostIn(["169.254.169.254"])`
    /// carves an IMDS hole out of a broad `http` grant). Separate list so grants stay purely additive; empty
    /// unless populated via [`Authorizer::with_deny_rules`], so an authorizer with no deny rules behaves
    /// EXACTLY as before (this is additive — deny-overrides only engages when a rule is present).
    deny_rules: Vec<crate::effect::FamilyGrant>,
}

impl Authorizer {
    /// An authorizer holding a fixed capability set (the session's grants). Delegation/attenuation
    /// (§12f) layers on later; v0 is a flat grant set for one operator.
    pub fn new(caps: Vec<Capability>) -> Self {
        Authorizer {
            caps,
            family_grants: Vec::new(),
            deny_rules: Vec::new(),
        }
    }

    /// Add register-by-string [`FamilyGrant`](crate::effect::FamilyGrant)s (from
    /// [`Capability::for_family`]) — grants for families with no built-in [`crate::effect::EffectKind`]
    /// (`store/*` §4c, extension families). Builder-style; composes with the `Capability` set passed to
    /// [`Authorizer::new`]. A request is permitted if EITHER a `Capability` OR a `FamilyGrant` admits it.
    pub fn with_family_grants(mut self, grants: Vec<crate::effect::FamilyGrant>) -> Self {
        self.family_grants.extend(grants);
        self
    }

    /// Add explicit DENY rules that OVERRIDE any grant (the "deny wins" precedence). A request matching any
    /// deny rule is refused even if a `Capability`/`FamilyGrant` permits it — so a broad grant can be carved
    /// with a narrow hole (e.g. grant `http` to `Any` but deny `http` to the IMDS host, or grant `store/set`
    /// on `system/` but deny it on `system/compiler/`). Each rule is a [`FamilyGrant`](crate::effect::FamilyGrant)
    /// (family + predicate); a request is denied iff its family matches AND the predicate admits its target.
    /// Builder-style, additive: an authorizer with no deny rules is unchanged (deny-overrides engages only
    /// when a rule matches). Composes with [`Authorizer::new`]/[`Authorizer::with_family_grants`].
    pub fn with_deny_rules(mut self, rules: Vec<crate::effect::FamilyGrant>) -> Self {
        self.deny_rules.extend(rules);
        self
    }

    /// Grant nothing — every effect is denied. Useful for pure-fold reducers that should have no
    /// ambient authority (the §9c "deny the clock entirely" case).
    pub fn deny_all() -> Self {
        Authorizer {
            caps: Vec::new(),
            family_grants: Vec::new(),
            deny_rules: Vec::new(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Authorize for Authorizer {
    /// SEC-F1: permission requires a grant whose predicate admits the *resolved target* — a `Capability`
    /// (keyed on `EffectKind::family`) OR a register-by-string `FamilyGrant` (keyed on the family string, for
    /// families with no built-in kind, e.g. `store/*`). An explicit DENY rule OVERRIDES any grant ("deny
    /// wins"): a request matching a deny rule is refused first, before the grant check, so a narrow deny
    /// carves a hole out of a broad allow. Native async (no `.await` — a flat set check).
    async fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
        // DENY-overrides-ALLOW: an explicit deny rule wins over any grant. Checked first so a matched deny
        // short-circuits the grant check (and can't be re-permitted by a broad Capability/FamilyGrant).
        if let Some(rule) = self.deny_rules.iter().find(|r| r.permits(req)) {
            return Err(format!(
                "explicitly DENIED: a deny rule for family {:?} covers target {:?}",
                rule.family, req.target
            ));
        }
        if self.caps.iter().any(|c| c.permits(req))
            || self.family_grants.iter().any(|g| g.permits(req))
        {
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
    async fn family_grant_permits_a_store_family_no_capability_can() {
        use crate::effect::{effect_ct, EffectRequest};
        use crate::name_store::NameStore;
        // §4c: store/* has NO EffectKind, so a `Capability` (keyed on kind.family()) CANNOT grant it —
        // Capability::for_family + with_family_grants is the register-by-string seam that can. A store/set
        // grant scoped to `system/` permits `store/set system/…` and denies other prefixes / other families.
        let authz = Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("system/".into()),
        )]);
        let store_set = |name: &str| {
            EffectRequest::new_with_family(
                effect_ct::STORE_SET,
                name,
                None,
                crate::effect::Timeliness::Interactive,
            )
        };
        // Permits a system/ name...
        assert!(authz
            .authorize(&store_set(NameStore::COMPILER_LATEST))
            .await
            .is_ok());
        // ...denies a name outside the granted prefix...
        assert!(authz.authorize(&store_set("session/abc/x")).await.is_err());
        // ...and denies a DIFFERENT store family (store/resolve) the grant didn't name.
        assert!(authz
            .authorize(&EffectRequest::new_with_family(
                effect_ct::STORE_RESOLVE,
                NameStore::COMPILER_LATEST,
                None,
                crate::effect::Timeliness::Interactive,
            ))
            .await
            .is_err());
        // A plain Capability-only authorizer CANNOT grant store/* at all (the gap for_family closes).
        let no_store = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        assert!(no_store.authorize(&store_set("system/x")).await.is_err());
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
    async fn a_deny_rule_overrides_a_broad_grant_carving_a_hole() {
        // explicit-DENY-overrides-ALLOW (operator-flagged): a narrow deny rule refuses a request a broad
        // grant would permit. Grant Http to ANY host, then deny Http to the IMDS host specifically — the
        // classic SSRF hole carved out of a broad allow. deny wins.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }])
        .with_deny_rules(vec![Capability::for_family(
            EffectKind::Http.family(),
            ResourcePredicate::HostIn(vec!["169.254.169.254".into()]),
        )]);
        // A normal host is still permitted (the grant applies, no deny matches)...
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://ok.host/x"))
            .await
            .is_ok());
        // ...but the denied host is refused despite the Any grant (deny overrides).
        let denied = authz
            .authorize(&req(
                EffectKind::Http,
                "http://169.254.169.254/latest/meta-data/",
            ))
            .await;
        assert!(
            denied.is_err(),
            "the IMDS host is denied despite the broad Http grant"
        );
        assert!(
            denied.unwrap_err().contains("DENIED"),
            "the denial reason names the explicit deny, not a missing grant"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_deny_rule_only_bites_its_family_and_predicate() {
        // A deny rule is scoped: it refuses only its family+predicate, leaving other families/targets to the
        // normal grant check. Deny store/set on `system/compiler/`, but store/set on `system/other` still
        // rides the grant (deny didn't match), and a different family is untouched.
        use crate::effect::{effect_ct, EffectRequest};
        let store_set = |name: &str| {
            EffectRequest::new_with_family(
                effect_ct::STORE_SET,
                name,
                None,
                crate::effect::Timeliness::Interactive,
            )
        };
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }])
        .with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("system/".into()),
        )])
        .with_deny_rules(vec![Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("system/compiler/".into()),
        )]);
        // Denied: store/set under the carved-out prefix (deny matches, overrides the system/ grant).
        assert!(authz
            .authorize(&store_set("system/compiler/latest"))
            .await
            .is_err());
        // Permitted: store/set to another system/ name (grant applies, deny prefix doesn't match).
        assert!(authz
            .authorize(&store_set("system/policy/current"))
            .await
            .is_ok());
        // A different family (Http) is entirely unaffected by the store/set deny rule.
        assert!(authz
            .authorize(&req(EffectKind::Http, "https://ok.host/x"))
            .await
            .is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_deny_rules_is_unchanged_behavior() {
        // Additive guarantee: an authorizer with no deny rules behaves EXACTLY as before — a grant permits,
        // absence denies, nothing new engages.
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
