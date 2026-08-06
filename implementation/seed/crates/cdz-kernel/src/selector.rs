//! Artifact OUTPUT-ROUTING — the selector program that decides where each emitted artifact goes
//! (operator invoke-ABI ruling, Slack seq 108/109, 2026-08-04).
//!
//! A generic component invocation (see [`crate::wasm_host::invoke_component`]) returns a SET of
//! [`Artifact`](crate::wasm_host::Artifact)s. The caller supplies a **selector** that routes each
//! artifact to a [`Sink`] — back into the SESSION as a response, or into the CAS (content-addressed
//! store, optionally also publishing a mutable-name pointer). This module is the PURE routing DECISION:
//! given the artifact set + the selector, partition it into per-sink groups. It does NO I/O — the actual
//! sinks (a `blob.put` for a CAS artifact, folding a session-response artifact back into the session's
//! log) wire in at the effect-integration slice, which pairs the CAS write with v-agent-harness-host's
//! store. Keeping the decision pure makes it fully unit-testable and keeps the I/O seam separate.
//!
//! 🔑 KEYSTONE INVARIANT (operator seq 109): the host/kernel knows NOTHING about the compiler or any
//! program — its entire model is (1) a method got invoked, (2) a response came back, (3) outputs get
//! routed to locations. So the selector matches ONLY on an artifact's OPAQUE `kind`/`name` strings; it
//! never encodes "this is a .wasm from the compiler" or any program-specific knowledge. The CALLER
//! supplies the rules; the host blindly applies them. A selector that could answer "is this the
//! compiler?" would be a bug.

use crate::wasm_host::Artifact;

/// WHERE a routed artifact goes. The two sinks the operator named (seq 108): back into the session as a
/// response, or into the CAS. Not a bool — a sum so a third sink (e.g. an outbound inbox) grows cleanly
/// (no-sentinels standing directive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sink {
    /// Fold the artifact back INTO THE SESSION as (part of) the invocation's response — the in-line
    /// result the caller reads. The default destination for a "give me the answer" invocation.
    SessionResponse,
    /// Write the artifact into the CAS (content-addressed by its hash). `name` optionally ALSO publishes
    /// a mutable-name pointer (`name → hash`) via the §4c name store, so a later session can resolve it
    /// by name (e.g. `system/compiler/latest`). `None` = content-address only (addressable by hash, no
    /// mutable pointer). The name is an OPAQUE string the caller chose — the host attaches no meaning.
    Cas { name: Option<String> },
}

/// One routing RULE: match an artifact by its opaque `kind` and/or `name`, and send a match to `sink`.
/// Both matchers `None` = a catch-all (matches every artifact) — the natural DEFAULT rule at the end of
/// a rule list. `kind` matches EXACTLY; `name_prefix` matches a name PREFIX (mirrors the effect-layer
/// [`crate::effect::ResourcePredicate::Prefix`] style, which authz reuses — a prefix is enough for real
/// routing and needs no glob/regex dependency). When both are set, BOTH must match (AND). Matching is on
/// opaque strings only (keystone invariant): a rule carries zero knowledge of what produced the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorRule {
    /// Match only artifacts of EXACTLY this kind, or `None` to match any kind.
    pub kind: Option<String>,
    /// Match only artifacts whose name starts with this prefix, or `None` to match any name.
    pub name_prefix: Option<String>,
    /// Where a matching artifact goes.
    pub sink: Sink,
}

impl SelectorRule {
    /// Does this rule match `artifact`? Both present matchers must hold (AND); an absent matcher is a
    /// wildcard. A rule with both matchers absent matches everything (catch-all).
    pub fn matches(&self, artifact: &Artifact) -> bool {
        let kind_ok = self.kind.as_ref().is_none_or(|k| *k == artifact.kind);
        let name_ok = self
            .name_prefix
            .as_ref()
            .is_none_or(|p| artifact.name.starts_with(p));
        kind_ok && name_ok
    }
}

/// A caller-supplied selector PROGRAM: an ordered list of rules, FIRST-MATCH-WINS. Slice-2 form is the
/// declarative kind/name→sink map (concierge-approved as the base-case of a fuller expression program;
/// "a map IS a degenerate program" — the evaluator can grow to an invokable selector-component later
/// without changing the rule→sink seam). An artifact that matches NO rule is NOT silently dropped — it's
/// a routing error (see [`Selector::route`]): an emitted artifact with nowhere to go is a caller bug.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selector {
    /// The rules, applied in order; the first whose matcher holds wins. An empty list routes nothing
    /// (every artifact is unrouted) — a caller that wants a default must append a catch-all rule.
    pub rules: Vec<SelectorRule>,
}

impl Selector {
    /// A selector that sends EVERY artifact to the session as the response — the simplest program (one
    /// catch-all rule → [`Sink::SessionResponse`]). The "just give me the answer inline" default.
    pub fn all_to_session() -> Self {
        Selector {
            rules: vec![SelectorRule {
                kind: None,
                name_prefix: None,
                sink: Sink::SessionResponse,
            }],
        }
    }

    /// Route an artifact SET through this selector — the PURE decision (no I/O). Each artifact takes the
    /// sink of the FIRST rule it matches; an artifact matching no rule is a [`RouteError::Unrouted`]
    /// (fail-loud — an emitted artifact must have a destination, never a silent drop). Returns the
    /// artifacts partitioned by sink, ready for the effect-integration slice to actually write (CAS
    /// `blob.put` / session-response fold).
    pub fn route(&self, artifacts: Vec<Artifact>) -> Result<RoutedArtifacts, RouteError> {
        let mut routed = RoutedArtifacts::default();
        for artifact in artifacts {
            match self.rules.iter().find(|r| r.matches(&artifact)) {
                Some(rule) => match &rule.sink {
                    Sink::SessionResponse => routed.session.push(artifact),
                    Sink::Cas { name } => routed.cas.push((artifact, name.clone())),
                },
                None => {
                    return Err(RouteError::Unrouted {
                        kind: artifact.kind,
                        name: artifact.name,
                    })
                }
            }
        }
        Ok(routed)
    }
}

/// An artifact set partitioned by sink — the output of [`Selector::route`]. The effect-integration slice
/// consumes this: `session` artifacts fold into the invocation's response; each `cas` artifact is
/// `blob.put` (and, if its `Option<String>` name is `Some`, a `name → hash` pointer is published).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoutedArtifacts {
    /// Artifacts to fold back into the session as the response.
    pub session: Vec<Artifact>,
    /// Artifacts to write to the CAS, each with its optional mutable-name pointer (`Some(name)` also
    /// publishes `name → hash`; `None` = content-address only).
    pub cas: Vec<(Artifact, Option<String>)>,
}

/// A routing failure. Currently only "an emitted artifact matched no rule" — surfaced rather than
/// silently dropped so a caller whose selector doesn't cover an artifact the invokee emitted learns of
/// it (fail-loud). A sum so it grows if later sinks add their own routing-time errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// An artifact matched no rule in the selector — it has no destination. Names the artifact's opaque
    /// `kind`/`name` so the caller can see which one and add a rule (or a catch-all).
    Unrouted { kind: String, name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(kind: &str, name: &str, bytes: &[u8]) -> Artifact {
        Artifact {
            kind: kind.into(),
            name: name.into(),
            bytes: bytes.to_vec(),
        }
    }

    // The simplest program: a catch-all → session sends every artifact back as the response.
    #[test]
    fn all_to_session_routes_every_artifact_to_the_session() {
        let arts = vec![
            artifact("wasm", "prog", &[1]),
            artifact("diag", "log", &[2]),
        ];
        let routed = Selector::all_to_session()
            .route(arts.clone())
            .expect("routes");
        assert_eq!(routed.session, arts);
        assert!(routed.cas.is_empty());
    }

    // Kind-split routing: the `wasm` artifact → CAS (with a name pointer), the `diag` artifact → session.
    // The compiler's natural shape (emit the program to the store, the diagnostics inline) — but the
    // selector matches on OPAQUE kinds, with zero knowledge that a compiler produced them (keystone).
    #[test]
    fn kind_split_routes_wasm_to_cas_and_diagnostics_to_session() {
        let sel = Selector {
            rules: vec![
                SelectorRule {
                    kind: Some("wasm".into()),
                    name_prefix: None,
                    sink: Sink::Cas {
                        name: Some("system/compiler/latest".into()),
                    },
                },
                SelectorRule {
                    kind: None,
                    name_prefix: None,
                    sink: Sink::SessionResponse,
                },
            ],
        };
        let arts = vec![
            artifact("wasm", "prog", &[0xDE, 0xAD]),
            artifact("diag", "log", &[0x2A]),
        ];
        let routed = sel.route(arts).expect("routes");
        assert_eq!(
            routed.cas,
            vec![(
                artifact("wasm", "prog", &[0xDE, 0xAD]),
                Some("system/compiler/latest".into())
            )]
        );
        assert_eq!(routed.session, vec![artifact("diag", "log", &[0x2A])]);
    }

    // A name-prefix rule matches by opaque name prefix (authz-Prefix style), regardless of kind.
    #[test]
    fn name_prefix_rule_matches_by_name_prefix() {
        let sel = Selector {
            rules: vec![
                SelectorRule {
                    kind: None,
                    name_prefix: Some("out/".into()),
                    sink: Sink::Cas { name: None },
                },
                SelectorRule {
                    kind: None,
                    name_prefix: None,
                    sink: Sink::SessionResponse,
                },
            ],
        };
        let routed = sel
            .route(vec![
                artifact("wasm", "out/a", &[1]),
                artifact("wasm", "keep", &[2]),
            ])
            .expect("routes");
        assert_eq!(routed.cas, vec![(artifact("wasm", "out/a", &[1]), None)]);
        assert_eq!(routed.session, vec![artifact("wasm", "keep", &[2])]);
    }

    // FIRST-match-wins: an earlier rule shadows a later one that would also match.
    #[test]
    fn first_matching_rule_wins() {
        let sel = Selector {
            rules: vec![
                SelectorRule {
                    kind: Some("wasm".into()),
                    name_prefix: None,
                    sink: Sink::SessionResponse,
                },
                SelectorRule {
                    kind: Some("wasm".into()),
                    name_prefix: None,
                    sink: Sink::Cas { name: None },
                },
            ],
        };
        let routed = sel
            .route(vec![artifact("wasm", "x", &[1])])
            .expect("routes");
        // The first rule (SessionResponse) wins, not the second (Cas).
        assert_eq!(routed.session, vec![artifact("wasm", "x", &[1])]);
        assert!(routed.cas.is_empty());
    }

    // An artifact matching NO rule is a fail-loud RouteError, never a silent drop.
    #[test]
    fn an_unrouted_artifact_is_a_route_error_not_a_silent_drop() {
        let sel = Selector {
            rules: vec![SelectorRule {
                kind: Some("wasm".into()),
                name_prefix: None,
                sink: Sink::SessionResponse,
            }],
        };
        match sel.route(vec![artifact("diag", "log", &[1])]) {
            Err(RouteError::Unrouted { kind, name }) => {
                assert_eq!(kind, "diag");
                assert_eq!(name, "log");
            }
            other => panic!("expected Unrouted for an uncovered artifact, got {other:?}"),
        }
    }

    // An empty selector routes nothing → the first artifact is unrouted (a caller must supply a rule).
    #[test]
    fn an_empty_selector_leaves_every_artifact_unrouted() {
        let sel = Selector::default();
        match sel.route(vec![artifact("wasm", "x", &[1])]) {
            Err(RouteError::Unrouted { .. }) => {}
            other => panic!("expected Unrouted under an empty selector, got {other:?}"),
        }
    }

    // A rule with BOTH matchers set is an AND: the artifact's kind must match EXACTLY *and* its name must
    // start with the prefix. Pins the documented AND semantics (SelectorRule doc + `matches`) — a future
    // change that flipped it to OR would let one matcher alone route, silently mis-sinking artifacts.
    // Here: the AND rule (kind==wasm AND name starts "out/") sinks ONLY the artifact satisfying both; a
    // wasm with a non-matching name and a non-wasm with a matching name both FALL THROUGH to the catch-all.
    #[test]
    fn a_rule_with_both_kind_and_name_prefix_matches_only_when_both_hold() {
        let sel = Selector {
            rules: vec![
                SelectorRule {
                    kind: Some("wasm".into()),
                    name_prefix: Some("out/".into()),
                    sink: Sink::Cas { name: None },
                },
                SelectorRule {
                    kind: None,
                    name_prefix: None,
                    sink: Sink::SessionResponse,
                },
            ],
        };
        let routed = sel
            .route(vec![
                artifact("wasm", "out/a", &[1]), // both hold → CAS
                artifact("wasm", "keep", &[2]), // kind holds, prefix does NOT → falls through to session
                artifact("diag", "out/b", &[3]), // prefix holds, kind does NOT → falls through to session
            ])
            .expect("routes");
        assert_eq!(
            routed.cas,
            vec![(artifact("wasm", "out/a", &[1]), None)],
            "only the artifact satisfying BOTH matchers is CAS-routed"
        );
        assert_eq!(
            routed.session,
            vec![artifact("wasm", "keep", &[2]), artifact("diag", "out/b", &[3])],
            "an artifact satisfying only ONE matcher must NOT match the AND rule — it falls through"
        );
    }

    // Direct unit of `SelectorRule::matches` AND-truth-table (the routing primitive): both-set matches
    // only on (kind AND prefix); each single-miss is false; both-absent is the catch-all (always true).
    #[test]
    fn selector_rule_matches_is_a_strict_and_over_present_matchers() {
        let both = SelectorRule {
            kind: Some("wasm".into()),
            name_prefix: Some("out/".into()),
            sink: Sink::SessionResponse,
        };
        assert!(
            both.matches(&artifact("wasm", "out/a", &[])),
            "kind✓ prefix✓ → true"
        );
        assert!(
            !both.matches(&artifact("wasm", "keep", &[])),
            "kind✓ prefix✗ → false"
        );
        assert!(
            !both.matches(&artifact("diag", "out/a", &[])),
            "kind✗ prefix✓ → false"
        );
        assert!(
            !both.matches(&artifact("diag", "keep", &[])),
            "kind✗ prefix✗ → false"
        );
        let catch_all = SelectorRule {
            kind: None,
            name_prefix: None,
            sink: Sink::SessionResponse,
        };
        assert!(
            catch_all.matches(&artifact("anything", "whatever", &[])),
            "both matchers absent → catch-all (always true)"
        );
    }
}
