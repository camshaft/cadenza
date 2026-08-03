# PR #1591 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1591 (PR: "xtask/fleet: scope the re-dispatch
guard to in-flight+ref; retire reaped records (executor nit)"). Copilot marked "🟡 Not ready to
approve" on point 2. Both verified against the code.

## 1. `publish_candidate` guard no longer matches `dispatch_plan`'s "already dispatched" logic — preview/executor divergence (Copilot, fleet.rs:7906) — correctness
> `publish_candidate` now scopes the in-flight guard to `dispatch_is_in_flight` + `refs_match`, but
> `dispatch_plan` still considers a candidate "already in flight" when either `d.r#ref == r#ref` OR
> `d.agent == agent` (and without filtering to in-flight). This makes the plan output disagree with the
> executor guard (e.g., it may report "already dispatched" for a different ref from the same agent, even
> though `publish_candidate` would proceed).

VERIFIED: the #1591 diff rewrites `publish_candidate`'s guard to `.filter(dispatch_is_in_flight)
.find(|d| refs_match(&d.r#ref, r#ref))` but does NOT touch `dispatch_plan` (fleet.rs:7826), which still
uses `.find(|d| d.r#ref == r#ref || d.agent == agent)`. So `dispatch-plan` (the PREVIEW) and
`publish-candidate` (the EXECUTOR) now diverge — the preview can report "in-flight: YES, do NOT
re-dispatch" for a different ref from the same agent while the executor would proceed. The PR's OWN
comment states the preview is a "shared source of truth [that] can never drift from the executor" — so
this drift is a regression against the stated invariant. SUBSTANTIVE: apply the same
`dispatch_is_in_flight` + `refs_match` scoping to `dispatch_plan`'s `already` computation.

## 2. `refs_match` doc "name the same commit" overstates a prefix heuristic (Copilot, fleet.rs:7652) — doc/accuracy
> `refs_match`'s doc comment currently claims it checks whether two refs "name the same commit", but
> the implementation is only a case-insensitive prefix heuristic. That can be true for an abbreviation,
> but it does not actually prove identity/uniqueness, so the comment is stronger than what the function
> guarantees.

VERIFIED: `refs_match` is a case-insensitive either-direction prefix match (per the diff + its unit
test `refs_match_is_prefix_tolerant_case_insensitive_never_matches_empty`). The doc "name the same
commit" implies identity; a prefix match can't prove uniqueness (two distinct shas could share a
prefix, though astronomically unlikely for full shas). Soften the doc to "the same commit by
abbreviation-tolerant prefix match (git convention; not a uniqueness proof)". LOW/doc.
