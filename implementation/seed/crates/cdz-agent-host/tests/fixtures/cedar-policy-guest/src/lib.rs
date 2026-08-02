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
//! any parse/build problem degrades to a `deny` with a reason, not a panic. (The fixture's own unit test
//! below asserts the embedded policy parses + decides as intended; note this fixture crate is EXCLUDED
//! from the workspace, so that test runs only when this fixture is built directly — the cdz-agent-host CI
//! job builds it — not in a normal `cargo test` of the parent crate.)

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

/// Escape an id for Cedar's double-quoted string syntax: backslash and double-quote must be escaped so
/// the id can't break out of (or malform) the `Type::"id"` entity-uid literal. This is CEDAR escaping —
/// NOT Rust `{:?}` Debug escaping, which only incidentally quotes bare strings and would turn any id with
/// a quote/backslash/control char into MALFORMED Cedar → a silent parse-error DENY (PR#1295). Escaping
/// here keeps a special-char principal/action/target a VALID literal, so the policy decides it for real.
fn cedar_escape(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out
}

/// Evaluate the embedded policy set for the request. `Ok(true)` = allow, `Ok(false)` = deny, `Err` = a
/// request/policy build error (mapped to a reasoned deny by the caller).
fn decide(request: &AuthRequest) -> Result<bool, String> {
    let policies = PolicySet::from_str(POLICY_SRC).map_err(|e| format!("policy parse: {e}"))?;
    // Build the Cedar PARC triple from the request strings. principal = the session identity, action =
    // Action::"<kind>", resource = Resource::"<target>". Cedar entity-uid syntax: `Type::"id"`, with the
    // id CEDAR-escaped (not Rust `{:?}`) so a quote/backslash in it can't malform the literal.
    let principal: EntityUid = EntityUid::from_str(&format!(
        "Principal::\"{}\"",
        cedar_escape(&request.principal)
    ))
    .map_err(|e| format!("principal uid: {e}"))?;
    let action: EntityUid =
        EntityUid::from_str(&format!("Action::\"{}\"", cedar_escape(&request.action)))
            .map_err(|e| format!("action uid: {e}"))?;
    let resource: EntityUid =
        EntityUid::from_str(&format!("Resource::\"{}\"", cedar_escape(&request.target)))
            .map_err(|e| format!("resource uid: {e}"))?;
    let req = Request::new(principal, action, resource, Context::empty(), None)
        .map_err(|e| format!("request: {e}"))?;
    let answer = Authorizer::new().is_authorized(&req, &policies, &Entities::empty());
    Ok(matches!(answer.decision(), Decision::Allow))
}

export!(Guest0);

#[cfg(test)]
mod tests {
    // NOTE: this test runs on the NATIVE host (a direct `cargo test` in this fixture dir — the fixture
    // crate is workspace-excluded, so it does NOT run in a normal `cargo test` of cdz-agent-host; the
    // cdz-agent-host CI job builds this fixture and this runs there). It exercises the `decide` policy
    // logic directly (cedar-policy compiles native too) — a guard that the embedded POLICY_SET parses and
    // expresses the intended decisions, so a malformed policy is caught rather than degrading to a runtime
    // deny. We call `decide` with a hand-built AuthRequest.
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

    #[test]
    fn special_char_ids_decide_via_the_policy_not_a_parse_error_deny() {
        // PR#1295: a principal/target with a quote or backslash must build a VALID Cedar entity via
        // cedar_escape (not `{:?}` Debug quoting, which would malform the literal → a silent parse-error
        // deny). So a special-char id still reaches a real policy DECISION (`Ok(_)`), not `Err`.
        // A quote-containing target under the broad http permit → still ALLOWED (it's a valid, non-IMDS
        // resource once escaped), proving the escape kept the literal valid rather than erroring.
        assert_eq!(
            decide(&req("agent", "http", "https://ok.host/a\"b")),
            Ok(true),
            "a quote in the target must escape into a valid Cedar literal + decide, not Err"
        );
        // A backslash in the principal likewise builds a valid entity and decides (http still permitted).
        assert_eq!(
            decide(&req("agent\\x", "http", "https://ok.host/x")),
            Ok(true)
        );
        // Directly assert cedar_escape's contract.
        assert_eq!(cedar_escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(cedar_escape(r"a\b"), r"a\\b");
        assert_eq!(cedar_escape("plain"), "plain");
    }
}
