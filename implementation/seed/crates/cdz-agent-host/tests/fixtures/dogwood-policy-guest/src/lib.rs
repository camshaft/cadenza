//! A DOGWOOD-policy wasm-component authorizer FIXTURE — the TEMPORAL extension to our Cedar auth stack, as
//! a wasm component EXACTLY like `cedar-policy-guest` (operator: "just like cedar it should be a wasm
//! component"). A wit-bindgen Rust guest that embeds a DOGWOOD policy, LOWERS it to a Cedar `PolicySet`
//! in-wasm (dogwood's temporal operators become `context.<id>` slots), and exports the same
//! `cadenza:agent-kernel-authz` authorizer-world's `authorize` the kernel's `ComponentAuthorizer`
//! instantiates + calls — so a dogwood component is drop-in interchangeable with the cedar one.
//!
//! **Why dogwood-as-a-component works in wasm:** dogwood LOWERS to plain Cedar — a policy compiles to a
//! `cedar_policy::PolicySet` where temporal conditions (`since`/`formerly`/`once`) become hoisted
//! `context.<id>` boolean slots. So the guest runs the SAME Cedar decision cedar-policy-guest does, on the
//! lowered policies. The temporal `context.<id>` bindings are HOST-PROVIDED (the host observes the session
//! event log + passes each slot's bool in the request) — keeping this component a PURE decision function
//! like cedar (no event history inside the guest). This slice wires the lowering + Cedar decision; the
//! host→guest temporal-binding channel is the next slice (today the request carries only principal/action/
//! target, so a policy whose temporal slot is unbound denies until that channel lands — fail-closed).
//!
//! **Totality (host is fail-closed):** the kernel treats a trap / instantiate-failure as a DENY, so a
//! broken policy can never accidentally PERMIT. Any lowering/parse/build problem degrades to a reasoned
//! `deny`, never a panic.

wit_bindgen::generate!({
    world: "authorizer-world",
    path: "wit/authorizer.wit",
});

use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, Request};
use dogwood_language::{LoweredPolicySet, PolicySchema, ServiceSchema};
use exports::cadenza::agent_kernel_authz::authorizer::{
    AuthRequest, Decision as WitDecision, Guest,
};
use std::str::FromStr;

/// The Cedar ACTION schema the dogwood policy types against (`.cedarschema` text — the same shape Cedar
/// itself uses, so dogwood integrates "the same way"). Minimal agent-authz surface: an `http` action a
/// temporal rule can gate. A real deployment ships its own schema + policy as the component's embedded
/// bytes; this fixture's are representative so the e2e proves a real lowered-Cedar decision.
const SCHEMA_SRC: &str = r#"
namespace Agent {
  entity Session;
  entity Resource;
  action "http" appliesTo {
    principal: [Session],
    resource: [Resource],
    context: {}
  };
  action "now" appliesTo {
    principal: [Session],
    resource: [Resource],
    context: {}
  };
}
"#;

/// The embedded DOGWOOD policy. Pure-Cedar rules lower unchanged (dogwood ⊇ Cedar); a temporal rule would
/// add `when temporal { … }` and lower to a `context.<id>` slot the host fills. This fixture's set proves
/// the LOWERING + Cedar decision path: `now` is permitted, `http` is permitted — both plain-Cedar rules
/// that lower to plain Cedar and decide with no temporal binding needed (the temporal-slot host-input
/// channel is the next slice; a temporal rule is added with it so the e2e can prove deny-then-allow).
const POLICY_SRC: &str = r#"
permit(principal, action == Agent::Action::"now", resource);
permit(principal, action == Agent::Action::"http", resource);
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
                    "denied by dogwood-lowered Cedar policy: action {:?} on target {:?}",
                    request.action, request.target
                ),
            },
            // A malformed request / policy is a DENY with the error as reason — never a trap (host is
            // fail-closed; we degrade cleanly to a reasoned deny anyway).
            Err(reason) => WitDecision {
                allow: false,
                reason: format!("dogwood authz error (deny): {reason}"),
            },
        }
    }
}

/// Lower the embedded dogwood policy → a plain Cedar `PolicySet`, build a Cedar `Request` from the
/// auth-request, and evaluate. Returns `Ok(true)`=permit, `Ok(false)`=deny, `Err(reason)`=malformed
/// (→ reasoned deny). The lowering runs per call (the fixture keeps it simple; a real component would
/// lower once at instantiation — a later optimization).
fn decide(request: &AuthRequest) -> Result<bool, String> {
    let policy_schema =
        PolicySchema::from_cedarschema_str(SCHEMA_SRC).map_err(|e| format!("schema: {e:?}"))?;
    let lowered =
        LoweredPolicySet::from_str(POLICY_SRC, &ServiceSchema::defaults(), &policy_schema)
            .map_err(|e| format!("lower: {e:?}"))?;
    let policies = lowered.as_cedar();

    // Build the Cedar entity-uids from the request's opaque id strings (Cedar-escaped so a special-char id
    // stays a valid literal, not a malformed-parse deny). action maps to `Agent::Action::"<action>"`;
    // principal to `Agent::Session::"<principal>"`; target to `Agent::Resource::"<target>"`.
    let principal = EntityUid::from_str(&format!(
        "Agent::Session::{}",
        cedar_quote(&request.principal)
    ))
    .map_err(|e| format!("principal: {e}"))?;
    let action = EntityUid::from_str(&format!("Agent::Action::{}", cedar_quote(&request.action)))
        .map_err(|e| format!("action: {e}"))?;
    let resource = EntityUid::from_str(&format!(
        "Agent::Resource::{}",
        cedar_quote(&request.target)
    ))
    .map_err(|e| format!("resource: {e}"))?;

    let cedar_request = Request::new(
        principal,
        action,
        resource,
        Context::empty(),
        None, // no schema-validation of the request itself — the policies carry the schema
    )
    .map_err(|e| format!("request: {e}"))?;

    let answer = Authorizer::new().is_authorized(&cedar_request, policies, &Entities::empty());
    Ok(matches!(answer.decision(), Decision::Allow))
}

/// Cedar-quote an id: wrap in double-quotes with `\` and `"` escaped, so a special-char id can't break out
/// of (or malform) the `Type::"id"` literal — CEDAR escaping, not Rust Debug. Mirrors cedar-policy-guest.
fn cedar_quote(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 2);
    out.push('"');
    for c in id.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

export!(Guest0);
