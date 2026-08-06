//! Host-side DOGWOOD lowering — the temporal extension to our Cedar auth stack (operator directive: "include
//! dogwood as part of our cedar auth stack … a temporal extension to cedar, integrate the same way").
//!
//! **What dogwood is + why HOST-SIDE.** [dogwood](https://github.com/dogwood-policy/dogwood) is Cedar with
//! HISTORY operators (`since` / `formerly` / `once` / aggregations). It does NOT replace Cedar — it LOWERS to
//! it: a dogwood policy compiles to a plain [`cedar_policy::PolicySet`] where each temporal condition becomes
//! a hoisted `context.<id>` boolean slot, filled per-decision from the session's event history by a stateful
//! temporal engine, and then PLAIN Cedar evaluates the request. So integrating dogwood "the same way as
//! Cedar" is an ADDITIVE LOWERING STAGE in front of the unchanged Cedar evaluator — the kernel stays
//! mechanism-only, and our existing Cedar decision path is reused verbatim on the lowered policies.
//!
//! It runs HOST-SIDE (native), NOT inside the `cedar-policy-guest` wasm component: the `amzn-dogwood-language`
//! crate pulls a heavy native tree (rhai/pest/…) that does not compile to `wasm32-unknown-unknown` (a
//! transitive `getrandom 0.3` rejects it without the `wasm_js` backend). The stateful temporal engine —
//! which observes an event HISTORY — is a natural fit host-side anyway (a per-request stateless wasm policy
//! component was the wrong home for it). Gated behind the off-by-default `dogwood` feature (the heavy tree
//! stays out of the hermetic default build).
//!
//! This module is the first slice: LOWER a dogwood policy (+ its Cedar action schema) to the Cedar artifacts
//! our authorizer consumes. The temporal-fill engine (observe the session log → fill the `context.<id>`
//! slots) + the `DogwoodAuthorizer` that feeds the filled context to the Cedar evaluator are follow-on
//! slices, built on the [`LoweredDogwoodPolicy`] this produces.

// dogwood re-exports the Cedar types via its `cedar` module SO we don't take a direct `cedar-policy` dep
// (which would have to be version-matched to dogwood's). Use that re-export for our return types.
use dogwood_language::cedar::{PolicySet, Schema};
use dogwood_language::{Error, LoweredPolicySet, PolicySchema, ServiceSchema};

/// A dogwood policy LOWERED to its Cedar artifacts (§dogwood host-lowering): the plain
/// [`cedar_policy::PolicySet`] our Cedar authorizer already evaluates, plus the augmented
/// [`cedar_policy::Schema`] carrying the hoisted `context.<id>` temporal slots. Produced by
/// [`lower`]; consumed by the (follow-on) temporal-fill engine + Cedar decision path.
///
/// Wraps [`LoweredPolicySet`] so the rest of the host talks in OUR types + never depends on the dogwood
/// crate's surface beyond this module (keeps dogwood swappable behind the same evolvable-policy seam Cedar
/// uses). The temporal leaves — the `context.<id>` slots a host engine must fill from event history — are
/// reachable via [`temporal_slot_ids`].
pub struct LoweredDogwoodPolicy {
    lowered: LoweredPolicySet,
}

impl LoweredDogwoodPolicy {
    /// Borrow the lowered plain [`cedar_policy::PolicySet`] — the artifact our existing Cedar authorizer
    /// evaluates unchanged (dogwood's temporal conditions are already lowered to `context.<id>` references).
    pub fn as_cedar_policies(&self) -> &PolicySet {
        self.lowered.as_cedar()
    }

    /// Borrow the augmented [`cedar_policy::Schema`] — the action schema PLUS the hoisted `context.<id>`
    /// fields the lowered policies reference. This is the schema Cedar's validator needs to typecheck them.
    pub fn cedar_schema(&self) -> &Schema {
        self.lowered.cedar_schema()
    }

    /// The hoisted `context.<id>` TEMPORAL SLOT IDs a host temporal engine must fill per decision (one per
    /// `since`/`formerly`/`once`/… condition dogwood lowered). Empty when the policy has no temporal
    /// conditions (a pure-Cedar policy lowered through dogwood is just Cedar). The (follow-on) engine
    /// observes the session's event log and produces a `bool` for each of these before Cedar evaluates.
    pub fn temporal_slot_ids(&self) -> Vec<String> {
        self.lowered
            .temporal_fields()
            .map(|f| f.id.clone())
            .collect()
    }
}

/// LOWER a dogwood policy to its Cedar artifacts (§dogwood host-lowering, the first slice). `policy` is
/// dogwood source (Cedar-derived syntax + `when temporal { … }` blocks); `cedar_action_schema` is the
/// service's Cedar action schema (`.cedarschema` text — the same schema shape Cedar itself uses, so this
/// integrates "the same way"). Uses dogwood's default [`ServiceSchema`] (no information providers — the
/// pure temporal path; provider/Rhai fields are a later concern and off this path).
///
/// Returns a [`LoweredDogwoodPolicy`] wrapping the plain Cedar PolicySet + augmented schema + temporal
/// slots. A malformed policy or schema is a clean [`Error`] (never a panic) the caller surfaces — same
/// decline-not-crash posture as the rest of the host's policy loading.
pub fn lower(policy: &str, cedar_action_schema: &str) -> Result<LoweredDogwoodPolicy, Error> {
    let service = ServiceSchema::defaults();
    let policy_schema = PolicySchema::from_cedarschema_str(cedar_action_schema)?;
    let lowered = LoweredPolicySet::from_str(policy, &service, &policy_schema)?;
    Ok(LoweredDogwoodPolicy { lowered })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal Cedar action schema with a temporal-usable "Read" then "Write" action shape (same
    // `.cedarschema` form Cedar uses). Kept small + self-contained.
    const SCHEMA: &str = r#"
namespace Drupe {
  type ReadInput = { document: String };
  type WriteInput = { document: String };
  entity Gateway;
  entity OAuthUser = { id: String };
  action "Read" appliesTo {
    principal: [OAuthUser], resource: [Gateway],
    context: { input: ReadInput }
  };
  action "Write" appliesTo {
    principal: [OAuthUser], resource: [Gateway],
    context: { input: WriteInput }
  };
}
"#;

    // A TEMPORAL policy: Write is permitted only if the same document was Read within the last hour — a
    // `formerly within` history condition dogwood lowers to a `context.<id>` slot.
    const TEMPORAL_POLICY: &str = r#"
permit (
    principal,
    action == Drupe::Action::"Write",
    resource
)
when temporal {
    formerly within 1h Drupe::Action::"Read"::request{ input.document: context.input.document }
};
"#;

    #[test]
    fn lowers_a_temporal_policy_to_a_cedar_policyset_with_a_temporal_slot() {
        let lowered = lower(TEMPORAL_POLICY, SCHEMA).expect("a well-formed temporal policy lowers");
        // The lowered artifact is a real Cedar PolicySet our authorizer can evaluate (non-empty — the one
        // permit lowered to at least one Cedar policy).
        assert!(
            lowered.as_cedar_policies().policies().count() >= 1,
            "the temporal policy lowered to at least one plain Cedar policy"
        );
        // The `formerly within` condition hoisted to exactly one `context.<id>` temporal slot a host engine
        // must fill from event history — the seam the temporal-fill stage (follow-on) plugs into.
        assert_eq!(
            lowered.temporal_slot_ids().len(),
            1,
            "the single `formerly within` condition lowered to one temporal context slot"
        );
    }

    #[test]
    fn a_malformed_policy_is_a_clean_error_not_a_panic() {
        let out = lower("this is not a dogwood policy", SCHEMA);
        assert!(
            out.is_err(),
            "a malformed policy declines with an Error, never panics"
        );
    }
}
