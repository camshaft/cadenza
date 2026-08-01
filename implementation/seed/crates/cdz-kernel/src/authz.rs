//! Authorization — the kernel's one gate before an effect reaches an executor (§2, §12c).
//!
//! v0 is a capability-set authorizer: an effect is permitted iff some held [`Capability`] permits it
//! (kind matches AND its resource predicate admits the resolved target — the SEC-F1 fix). In the full
//! design the authorizer is a swappable wasm component (Cedar as one impl, §1); v0 keeps it a plain
//! in-kernel check so the loop is testable, behind a trait so it can be swapped later without touching
//! the kernel core.

use crate::effect::{Capability, EffectRequest};

/// Decides whether a requested effect may be performed. The result is logged either way (§10): a
/// permitted effect proceeds to dispatch; a denied one becomes an `AuthzDenied` event and never runs.
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

    /// Is this request permitted? `Ok(())` to proceed; `Err(reason)` to deny (reason is logged).
    /// SEC-F1: permission requires a capability whose predicate admits the *resolved target*.
    pub fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
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
}
