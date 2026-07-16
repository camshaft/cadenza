//! Cedar authorization for the agent harness (Inc-3) — the decision every tool dispatch and resource
//! access passes through. Backed by the real `cedar-policy` evaluator (the concierge-approved 4.3-A
//! path; a Cadenza-native evaluator over the `(cedar-policyset …)` arena is the later 4.3-B flagship).
//!
//! ## The model (DESIGN-agent-harness.md §4)
//!
//! A request is (principal, action, resource, context). The principal is the acting AGENT; when it acts
//! ON BEHALF OF a user, the `context` carries `on_behalf_of` and the request is authorized against BOTH
//! the agent's own policies AND the user's delegation grant — the agent may do only what it *is* allowed
//! AND what the user *delegated* (the intersection; the narrower bound wins). [`authorize`] evaluates one
//! policy set; [`authorize_on_behalf_of`] takes the intersection of two.
//!
//! Entities/schema are kept minimal here (empty entity store, no schema) — the policies name concrete
//! entity UIDs (`Agent::"agent:v-cad"`, `Action::"tool:write-file"`, `File::"/repo/x"`) and condition on
//! `context`, which needs no entity hierarchy. A richer entity/`in`-group model is a later widening.

use anyhow::{anyhow, Result};
use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request};
use std::str::FromStr;

/// The outcome of an authorization request — a thin, host-agnostic mirror of Cedar's `Decision` so the
/// harness (and, later, the Cadenza loop across the boundary) needn't know the `cedar-policy` types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzDecision {
    /// At least one `permit` matched and no `forbid` overrode it — the action proceeds.
    Allow,
    /// No `permit` matched, or a `forbid` overrode — the action is denied (Cedar is deny-by-default).
    Deny,
}

impl AuthzDecision {
    /// Whether the action may proceed.
    pub fn is_allow(self) -> bool {
        matches!(self, AuthzDecision::Allow)
    }
}

fn from_cedar(d: Decision) -> AuthzDecision {
    match d {
        Decision::Allow => AuthzDecision::Allow,
        Decision::Deny => AuthzDecision::Deny,
    }
}

/// Authorize one request against one policy set. `policies` is Cedar policy TEXT (the same surface
/// `cadenza-syntax` round-trips); `principal`/`action`/`resource` are entity UIDs in Cedar syntax
/// (`Agent::"agent:v-cad"`, `Action::"tool:write-file"`, `File::"/repo/x"`); `context_json` is a JSON
/// object of the request context (`{}` for none — e.g. `{"on_behalf_of":"user:cameron"}`). Deny-by-
/// default: an empty/parse-valid policy set with no matching `permit` denies.
pub fn authorize(
    policies: &str,
    principal: &str,
    action: &str,
    resource: &str,
    context_json: &str,
) -> Result<AuthzDecision> {
    let policy_set = PolicySet::from_str(policies).map_err(|e| anyhow!("parse policy set: {e}"))?;
    let principal = EntityUid::from_str(principal).map_err(|e| anyhow!("parse principal: {e}"))?;
    let action = EntityUid::from_str(action).map_err(|e| anyhow!("parse action: {e}"))?;
    let resource = EntityUid::from_str(resource).map_err(|e| anyhow!("parse resource: {e}"))?;
    let context = if context_json.trim().is_empty() || context_json.trim() == "{}" {
        Context::empty()
    } else {
        Context::from_json_str(context_json, None).map_err(|e| anyhow!("parse context: {e}"))?
    };
    // No schema (None) + an empty entity store: the policies name concrete UIDs and condition on
    // context, so no entity hierarchy is needed for the harness's flat principal/action/resource model.
    let request = Request::new(principal, action, resource, context, None)
        .map_err(|e| anyhow!("build request: {e}"))?;
    let response = Authorizer::new().is_authorized(&request, &policy_set, &Entities::empty());
    Ok(from_cedar(response.decision()))
}

/// Authorize an agent acting ON BEHALF OF a user: the request is allowed iff BOTH the agent's own
/// policies AND the user's delegation grant allow it (the intersection — the agent can do only what it
/// *is* permitted AND what the user *delegated*). This is the safe reading of on-behalf-of: the narrower
/// bound wins, so a broad agent can't exceed a narrow delegation and a broad delegation can't widen a
/// restricted agent. Both are evaluated with the same request; the caller sets `context.on_behalf_of` so
/// the user's grant (which conditions on it) applies.
pub fn authorize_on_behalf_of(
    agent_policies: &str,
    user_delegation: &str,
    principal: &str,
    action: &str,
    resource: &str,
    context_json: &str,
) -> Result<AuthzDecision> {
    let agent = authorize(agent_policies, principal, action, resource, context_json)?;
    if !agent.is_allow() {
        return Ok(AuthzDecision::Deny); // the agent itself isn't permitted → denied regardless of delegation
    }
    // The agent is permitted; the user's delegation must ALSO permit it (intersection).
    authorize(user_delegation, principal, action, resource, context_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = r#"Agent::"agent:v-cad""#;
    const WRITE: &str = r#"Action::"tool:write-file""#;
    const DELETE: &str = r#"Action::"tool:delete-prod""#;
    const FILE: &str = r#"File::"/repo/src/foo.rs""#;

    #[test]
    fn a_matching_permit_allows() {
        let policies = r#"permit(principal, action == Action::"tool:write-file", resource);"#;
        assert_eq!(
            authorize(policies, AGENT, WRITE, FILE, "{}").unwrap(),
            AuthzDecision::Allow
        );
    }

    #[test]
    fn no_matching_permit_denies_by_default() {
        // A permit for write only; a delete request has no permit → deny (Cedar is deny-by-default).
        let policies = r#"permit(principal, action == Action::"tool:write-file", resource);"#;
        assert_eq!(
            authorize(policies, AGENT, DELETE, FILE, "{}").unwrap(),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn a_forbid_overrides_a_permit() {
        // Permit all actions, but forbid delete-prod: the forbid wins (Cedar forbid > permit).
        let policies = concat!(
            r#"permit(principal, action, resource);"#,
            r#"forbid(principal, action == Action::"tool:delete-prod", resource);"#,
        );
        assert_eq!(
            authorize(policies, AGENT, DELETE, FILE, "{}").unwrap(),
            AuthzDecision::Deny,
            "a forbid must override a broad permit"
        );
        // …but a non-forbidden action still passes the broad permit.
        assert_eq!(
            authorize(policies, AGENT, WRITE, FILE, "{}").unwrap(),
            AuthzDecision::Allow
        );
    }

    #[test]
    fn on_behalf_of_takes_the_intersection() {
        // The AGENT may write + delete; the USER's delegation only grants write. Acting on behalf of the
        // user, the agent may write (both allow) but NOT delete (delegation denies) — the narrower wins.
        let agent = r#"permit(principal, action, resource);"#; // agent can do anything
        let delegation = r#"permit(principal, action == Action::"tool:write-file", resource);"#; // user grants write only
        assert_eq!(
            authorize_on_behalf_of(agent, delegation, AGENT, WRITE, FILE, "{}").unwrap(),
            AuthzDecision::Allow,
            "write: agent allows AND delegation allows → allow"
        );
        assert_eq!(
            authorize_on_behalf_of(agent, delegation, AGENT, DELETE, FILE, "{}").unwrap(),
            AuthzDecision::Deny,
            "delete: agent allows but delegation does NOT → deny (intersection)"
        );
    }

    #[test]
    fn on_behalf_of_denies_when_the_agent_itself_cannot() {
        // The delegation is broad (anything) but the AGENT itself may only write — acting for the user,
        // the agent still can't delete (its own policy denies). The narrower bound is the agent here.
        let agent = r#"permit(principal, action == Action::"tool:write-file", resource);"#;
        let delegation = r#"permit(principal, action, resource);"#;
        assert_eq!(
            authorize_on_behalf_of(agent, delegation, AGENT, DELETE, FILE, "{}").unwrap(),
            AuthzDecision::Deny,
            "the agent's own policy is the narrower bound → deny"
        );
    }

    #[test]
    fn a_context_condition_gates_the_permit() {
        // A permit that only applies when acting on behalf of a specific user (a delegation shape).
        let policies = concat!(
            r#"permit(principal, action, resource)"#,
            r#" when { context.on_behalf_of == "user:cameron" };"#,
        );
        // With the matching context → allow.
        assert_eq!(
            authorize(
                policies,
                AGENT,
                WRITE,
                FILE,
                r#"{"on_behalf_of":"user:cameron"}"#
            )
            .unwrap(),
            AuthzDecision::Allow
        );
        // Without it (empty context) → the when-clause fails → deny.
        assert_eq!(
            authorize(policies, AGENT, WRITE, FILE, "{}").unwrap(),
            AuthzDecision::Deny,
            "the permit's when-clause requires the on_behalf_of context"
        );
    }

    #[test]
    fn a_malformed_policy_is_a_clean_error_not_a_panic() {
        let err = authorize("this is not cedar", AGENT, WRITE, FILE, "{}").unwrap_err();
        assert!(
            format!("{err}").contains("parse policy set"),
            "a malformed policy set must be a clean error: {err}"
        );
    }
}
