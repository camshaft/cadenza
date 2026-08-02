//! A Cedar-policy wasm-component authorizer FIXTURE — a wit-bindgen Rust guest that embeds a Cedar
//! `PolicySet` and exports the `cadenza:agent-kernel-authz` authorizer-world's `authorize` function.
//! The kernel's `ComponentAuthorizer` instantiates the lifted component and calls `authorize` to gate
//! every effect; this proves the operator's "Cedar as a content-addressable wasm component" pillar
//! (§20b) end-to-end — a REAL Cedar decision gating a REAL agent.
//!
//! On `authorize(request)`: build a Cedar `Request` from the request's `principal` / `action` / `target`
//! strings, evaluate it against the embedded `POLICY_SET` with a Cedar `Authorizer`, and map
//! `Decision::Allow` → `decision { allow: true }`, anything else → `{ allow: false, reason }`.
//!
//! **Totality (host is fail-closed):** the kernel treats a trap / instantiate-failure as a DENY, so a
//! broken policy can never accidentally PERMIT. This guest therefore never traps on a normal decision —
//! any parse/build problem degrades to a `deny` with a reason, not a panic. (A malformed embedded policy
//! is caught by the fixture's own build-time test, not at authorize time.)

wit_bindgen::generate!({
    world: "authorizer-world",
    path: "wit/authorizer.wit",
});

use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request};
use exports::cadenza::agent_kernel_authz::authorizer::{
    AuthRequest, Decision as WitDecision, Guest,
};
use std::str::FromStr;

/// The embedded Cedar policy set — a deny-by-default agent-authz example demonstrating the forbid-
/// overrides-permit shape (the model the operator wants and Cedar expresses natively):
/// - permit `now` and `timer` to any resource (cheap, safe, ambient);
/// - permit `http` broadly BUT `forbid` the IMDS metadata host (SSRF/exfil guard — forbid wins over the
///   broad permit, the case a flat capability set can't express);
/// - permit `model` only to a specific allow-listed model id;
/// - everything else → denied by default (no matching permit).
///
/// A real deployment ships its own policy set as the component's embedded bytes; this fixture's set is
/// representative so the e2e proves a real Cedar decision (permit + default-deny + forbid-override).
const POLICY_SRC: &str = r#"
permit(principal, action == Action::"now", resource);
permit(principal, action == Action::"timer", resource);
permit(principal, action == Action::"http", resource);
forbid(principal, action == Action::"http", resource == Resource::"http://169.254.169.254/latest/meta-data/");
permit(principal, action == Action::"model", resource == Resource::"claude-test");
"#;

struct Guest0;

impl Guest for Guest0 {
    fn authorize(request: AuthRequest) -> WitDecision {
        match decide(&request) {
            Ok(true) => WitDecision {
                allow: true,
                reason: String::new(),
            },
            Ok(false) => WitDecision {
                allow: false,
                reason: format!(
                    "denied by Cedar policy: action {:?} on target {:?}",
                    request.action, request.target
                ),
            },
            // A malformed request (an id Cedar can't parse) is a DENY with the parse error as the reason —
            // never a trap (the host is fail-closed, but we degrade cleanly to a reasoned deny anyway).
            Err(reason) => WitDecision {
                allow: false,
                reason: format!("authz error (deny): {reason}"),
            },
        }
    }
}

/// Evaluate the embedded policy set for the request. `Ok(true)` = allow, `Ok(false)` = deny, `Err` = a
/// request/policy build error (mapped to a reasoned deny by the caller).
fn decide(request: &AuthRequest) -> Result<bool, String> {
    let policies = PolicySet::from_str(POLICY_SRC).map_err(|e| format!("policy parse: {e}"))?;
    // Build the Cedar PARC triple from the request strings. principal = the session identity, action =
    // Action::"<kind>", resource = Resource::"<target>". Cedar entity-uid syntax: Type::"id".
    let principal: EntityUid = EntityUid::from_str(&format!("Principal::{:?}", request.principal))
        .map_err(|e| format!("principal uid: {e}"))?;
    let action: EntityUid = EntityUid::from_str(&format!("Action::{:?}", request.action))
        .map_err(|e| format!("action uid: {e}"))?;
    let resource: EntityUid = EntityUid::from_str(&format!("Resource::{:?}", request.target))
        .map_err(|e| format!("resource uid: {e}"))?;
    let req = Request::new(principal, action, resource, Context::empty(), None)
        .map_err(|e| format!("request: {e}"))?;
    let answer = Authorizer::new().is_authorized(&req, &policies, &Entities::empty());
    Ok(matches!(answer.decision(), Decision::Allow))
}

export!(Guest0);

#[cfg(test)]
mod tests {
    // NOTE: this test runs on the NATIVE host (cargo test in this fixture dir), not in wasm — it exercises
    // the `decide` policy logic directly (cedar-policy compiles native too). It's a build-time guard that
    // the embedded POLICY_SET parses and expresses the intended decisions, so a malformed policy is caught
    // here rather than degrading to a runtime deny. The wit-bindgen generated types aren't constructed
    // here; we call `decide` with a hand-built AuthRequest-shaped input via a small local mirror.
    use super::*;

    fn req(principal: &str, action: &str, target: &str) -> AuthRequest {
        AuthRequest {
            principal: principal.to_string(),
            action: action.to_string(),
            target: target.to_string(),
        }
    }

    #[test]
    fn policy_parses_and_expresses_the_intended_decisions() {
        // permit: now / timer / a normal http host / the allow-listed model id.
        assert_eq!(decide(&req("agent", "now", "")), Ok(true));
        assert_eq!(decide(&req("agent", "timer", "1000")), Ok(true));
        assert_eq!(decide(&req("agent", "http", "https://ok.host/x")), Ok(true));
        assert_eq!(decide(&req("agent", "model", "claude-test")), Ok(true));
        // forbid overrides the broad http permit: the IMDS metadata host is denied.
        assert_eq!(
            decide(&req(
                "agent",
                "http",
                "http://169.254.169.254/latest/meta-data/"
            )),
            Ok(false)
        );
        // default-deny: a model id outside the allow-list, and an unmodelled action.
        assert_eq!(decide(&req("agent", "model", "other-model")), Ok(false));
        assert_eq!(decide(&req("agent", "shell", "rm -rf /")), Ok(false));
    }
}
