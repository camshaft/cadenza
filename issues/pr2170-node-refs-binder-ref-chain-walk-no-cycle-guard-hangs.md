# PR #2170 review — rcdzc/src/effects.rs (v-effects) — OPEN — non-termination/DoS [VERIFIED, MED] (pre-existing, on the fix for MY #2128)

https://github.com/camshaft/cadenza/pull/2170 (restore subtree_references_binder short-circuit via a shared
node_refs_binder predicate — the PR for the #2128 fix v-effects landed as 5d3c1ed5). Copilot 1 inline.

## the `node_refs_binder` `Resolved::Ref` chain walk has NO cycle guard: a `Ref` cycle NOT including `binder` loops forever → hangs compilation instead of returning false; called from recursive subtree walks (Copilot, effects.rs:6970) — non-termination [VERIFIED, MED; PRE-EXISTING not a #2170 regression]
> The `Resolved::Ref` ref-chain walk here can loop forever if the chain contains a cycle that does not
> include `binder` (the resolver can represent `Ref` cycles; see `resolve.rs:value_ref_cycle`). This
> predicate is called from recursive subtree walks, so a cycle would hang compilation rather than
> returning `false`.

VERIFIED in the #2170 diff: `node_refs_binder` (diff:15-31) walks `Resolved::Ref { value } => { let mut
target = value; loop { if target == binder { break true } match resolved_of(db, target) { Ref { value:
next } => target = next, _ => break false } } }`. The loop breaks ONLY on `target == binder` (hit) or a
non-Ref node (miss) — there is NO visited-set. So a `Ref` cycle that does NOT pass through `binder` (e.g.
`a → b → a`, neither = binder) makes `target = next` follow Ref→Ref forever → compilation HANGS. Copilot
cites `resolve.rs:value_ref_cycle` — CONFIRMED it exists (resolve.rs:562), so the resolver CAN represent
`Ref` cycles; this isn't hypothetical. MED (a malformed/cyclic resolve graph hangs the compiler rather
than erroring).

IMPORTANT — PRE-EXISTING, NOT a #2170 regression: the current trunk `count_param_refs` (effects.rs:7811+,
the predicate #2170 refactors this out of) has the IDENTICAL unguarded loop (`target = next`, break on
binder / non-Ref, no visited-set). So #2170 PRESERVES the hazard while extracting it into the shared
`node_refs_binder`; it does not introduce it, and my #2128 (which was about short-circuiting the OUTER
subtree walk) didn't either. So this is a latent bug in the shared ref-chain walk, surfaced by the review
of the refactor. Worth fixing HERE since #2170 is the PR that consolidates the walk into one predicate —
one guard now protects both `subtree_references_binder` and `count_param_refs`. Fix: bound the ref-chain
walk with a visited-set (or a depth cap, or reuse whatever `value_ref_cycle` uses) and return false on a
cycle that doesn't reach `binder`. v-effects owns rcdzc effects. PR OPEN → foldable. (If v-effects judges a
`Ref` cycle unreachable in a well-typed program by this point in the pipeline, a debug_assert + comment is
the minimum — but `value_ref_cycle` existing suggests it's reachable enough to have its own detector.)
