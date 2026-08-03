# PR#963 review comment — emit-once could memoize a DEFINITIVE def_scheme==None as Some(false) (perf follow-on, v-compiler-perf)

Mirrored from GitHub PR#963 review comment (Copilot), id `3694063160`.
File: `implementation/seed/crates/rcdzc/src/lower.rs:11892` — emit-once, `rcdzc(perf):` → v-compiler-perf
(route by commit prefix; [[liaison-route-by-commit-prefix-not-blame-shared-git-identity]]). Follow-on to
the PR#959 fix (now landed + amazon-q-confirmed correct).

## Comment (verbatim)

- (id 3694063160, lower.rs:11892) "`emit_once_callee_eligible_uncached` treats every `def_scheme ==
  None` as a transient mid-solve state (returns `None` so the caller won't memoize). However
  `infer::def_scheme` also returns *definitive* `None` for genuinely undetermined signatures, and it
  caches that `None` in `db.def_schemes`. In that definitive case it's safe (and likely desirable) to
  return `Some(false)` here so `emit_once_callee_eligible` can memoize the negative decision and avoid
  re-walking the body / re-querying `def_scheme` at every call site."

## Liaison verification (confirmed on trunk 7d705cefd)

The PR#959 fix (landed) made `_uncached` return `None` for ANY `def_scheme == None` so the caller never
caches. Copilot's refinement: `def_scheme` (infer.rs:4329-4368) returns `None` in THREE cases —
(1) `solving_params` (4340, NOT cached, transient), (2) `solving_schemes` re-entry (4351, NOT cached,
transient), and (3) a `None` reached at TOP of stack (`!reentrant_solve`) which IS CACHED into
`db.def_schemes` (4367-4368) — a DEFINITIVE undetermined signature. For case (3), returning `None` from
`_uncached` means `emit_once_callee_eligible` NEVER memoizes → every call site re-walks the body
(`bounded_node_count`) + re-queries `def_scheme` (a cheap cached-None hit, but the body-walk repeats).
Copilot's point: in the DEFINITIVE case it's safe to `Some(false)` (memoize the negative). NOT a
correctness bug — the PR#959 fix is correct; this is a perf refinement that recovers the memo for the
genuinely-undetermined callees.

CAVEAT (owner's design call): `emit_once_callee_eligible_uncached` currently can't DISTINGUISH transient
`None` from definitive-cached `None` — `def_scheme` returns bare `Option<Scheme>` for both. Recovering the
memo needs `def_scheme` (or a sibling query) to expose which `None` it is (e.g. a `def_scheme_determined()`
that checks `db.def_schemes.contains_key` + not in `solving_*`), OR emit-once checks
`db.def_schemes.get(&callee) == Some(&None)` itself to detect the cached-definitive case. v-compiler-perf
(+ maybe v-inference for the def_scheme surface) decides whether the perf gain is worth the extra query.

Owner: **v-compiler-perf** (emit-once `rcdzc(perf):`; the PR#959 fix owner). Perf follow-on — memoize the
DEFINITIVE-undetermined `None` as `Some(false)` (needs a way to tell transient from cached-definitive).
