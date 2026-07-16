# PR review comment — mirrored from GitHub PR #462 (Copilot inline)

- **PR:** #462 "fleet: batch 92 (…, recursive-generic transformer closure-tie fix, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/infer.rs:2314`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3594207054
- **Link:** https://github.com/camshaft/cadenza/pull/462#discussion_r3594207054

## Comment (verbatim)
> `crate::unify::unify` returns `Result<(), Reject>` and can mutate `subst` before returning an error (e.g. after binding earlier vars in a recursive unify). Ignoring the result here risks leaving `subst` partially-updated on a mismatch, which can cause later inference to proceed from an inconsistent substitution. Unify against a cloned substitution and only commit it on success (or otherwise ensure rollback on failure).

## Liaison triage — CONFIRMED against trunk
Confirmed in infer.rs: `let _ = crate::unify::unify(&mut subst, &cur, at);` — the `Result<(), Reject>`
is discarded (`let _`), and `unify` mutates `subst` in place as it binds vars, so a unify that fails
PART-WAY through a recursive walk leaves `subst` PARTIALLY updated. Subsequent inference then proceeds
from an inconsistent substitution (some vars bound from a unification that ultimately failed) — a
soundness hazard in the seed HM (this is the call-seed-arg grounding path). The `let _` is deliberate
in the sense that a failed unify here is "the arg didn't constrain this param," but the SIDE EFFECT on
`subst` is the bug. FIX (as reviewer): unify against a CLONE of `subst` and commit only on `Ok`, or add
rollback on `Err`. Inference territory (v-inference owns infer.rs + unify.rs). Route to v-inference to
confirm (a case where a call-seed unify partially binds then fails, and a later param is grounded from
the stale binding) and guard. Fix on `trunk`. Quote + link in queue file.
