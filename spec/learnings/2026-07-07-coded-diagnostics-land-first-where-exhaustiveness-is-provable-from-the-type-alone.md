# Coded diagnostics land first where exhaustiveness is provable from the type alone — bool before user-sum, and a payload that carries the code, not a flag

*2026-07-07*

**What happened.** The self-hosted compiler's `Diag` channel stopped being monochrome: its `KError` payload,
previously a 0/1 decline-vs-reject FLAG, was generalized to carry the actual CDZ CODE (0 = silent decline; any
other value = the code to `Diag.emit`). With that, the compiler detected a NEW rejection family it couldn't before
— a one-arm bool `match` (`(match b (true 1))`) is provably non-exhaustive and now emits CDZ0210, matching native.
The byte gate moved: agree 95 → 98, disagree 96 → 92, the bool non-exhaustive cases reaching AGREE *by code* (the
compiler emits the same CDZ0210 native does), value-harness 0 hard / 0 error, 0 false-rejects. I verified by
isolation (one-case corpus) that both the parameter-scrutinee bool case AND my Run-108 constant-scrutinee case now
agree — the const-bool case I added last cycle as an under-reject flipped to agree this cycle as the check landed.

The remaining CDZ0210 disagreements are all one shape: user-SUM non-exhaustive (`(match (Some 5) ((Some x) x))`,
a Sign match missing variants) — native rejects, the compiler still declines. That is the ask-13 frontier, and
the split between it and the now-landed bool case is the point.

**Why.** Two lessons.

The first is about which rejections a self-hosting compiler can detect first, and it is a property of the TYPE,
not the effort: **a rejection lands when its premise is provable from information the compiler already has.** Bool
exhaustiveness is provable from the type ALONE — a Bool has exactly two values, so "one arm, no wildcard" is
non-exhaustive by a fact the compiler knows without consulting anything (the arity of the bool type is a
constant). User-sum exhaustiveness needs the declared VARIANT SET — "does this arm set cover the sum" requires
knowing how many variants the sum has and which, which the self-hosted compiler doesn't yet track. So bool
exhaustiveness (CDZ0210 over a 2-value type) landed while sum exhaustiveness (CDZ0210 over an N-variant type)
waits — the SAME diagnostic code, split by whether the value-set size is a constant or a lookup. This generalizes:
when forward-porting a rejection family to a self-hosted compiler, order the sub-cases by what the premise needs —
the ones provable from the type's fixed structure land first; the ones needing a declared-set lookup (variant
counts, field sets, imported signatures) wait on that table being tracked. A "family" of rejections sharing one
code is not one unit of work; it is several, ordered by the information each needs.

The second is about the diagnostic payload. The `Diag` channel was first wired carrying PRESENCE (a diagnostic
happened) and then a decline/reject FLAG (0/1); this cycle it carries the CODE. That progression is the honest
one: presence proves the plumbing (emit → handler → collect → record), the flag proves the plumbing distinguishes
two outcomes, and the code proves the plumbing carries arbitrary DATA end-to-end — `Diag.emit <code>` → handler
`List.push` → `Diag.collect` → `codes->diagnostics` maps each Int64 to its CDZ string. Only at the code stage is
the effect-based diagnostics data path actually realized (a diagnostic is data with a code, a message, a span —
not a boolean). The lesson: a channel that carries a flag is not yet carrying diagnostics; generalize the payload
from flag to code early, because the flag encoding (0/1) silently caps the channel at two outcomes and hides
whether the data path can carry real per-diagnostic data — and the moment it carries the code, the "lean on
effects for diagnostics" direction is genuinely realized rather than demonstrated on a proxy.

**The requirement it drove.** No new corpus case — the bool non-exhaustive cases (parameter and my Run-108
constant form) are already pinned and now AGREE, and the user-sum non-exhaustive cases are already pinned as the
ask-13 under-rejects; the corpus already spans the bool-vs-sum split, which is why the gate could show the bool
half landing while the sum half held. The output is this learning and the confirmed accounting (agree 95→98 via
CDZ0210-by-code on the bool cases; the 4 residual CDZ0210 disagreements are all user-sum, the ask-13 frontier;
WRONG=0, 0 false-rejects). General lesson: **a rejection family sharing one diagnostic code is several units of
work ordered by what each sub-case's premise needs — those provable from the type's fixed structure (bool's two
values) land before those needing a declared-set lookup (a sum's variant count); and a diagnostic channel carrying
a flag is not yet carrying diagnostics — generalize the payload from flag to code early, because only then is the
data path proven to carry real per-diagnostic data end-to-end.**
