# PR #2179 review — rcdzc/src/effects.rs (v-effects) — OPEN — correctness [PLAUSIBLE-HIGH]

https://github.com/camshaft/cadenza/pull/2179 (FIX rn3 depth-3 regression — restore the safe decline;
depth-3 guard on the pre-spec-lift). Copilot 1 inline — the new guard's fallback contradicts its own
conservative-decline doc.

## `resume_val_op_arm_also_performs_outer` is doc'd to conservatively DECLINE when the performed op's arm body isn't a bare tail `(resume v next)`, but the `tail_resume` fallback returns `false` (NOT a guard) on `None` → a depth-3+ chain whose intermediate arm is WRAPPED (`do`/`match`) or abortive slips through, reintroducing the wrong-fold risk (Copilot, effects.rs:7073) — correctness [VERIFIED code-vs-doc contradiction; reachability PLAUSIBLE-HIGH]
> `resume_val_op_arm_also_performs_outer` is documented as conservative when the performed op's arm body
> isn't a bare tail `(resume v next)`, but the current `tail_resume` fallback returns `false` (no guard).
> That contradicts the comment and can let depth-3+ chains slip through when the intermediate arm is
> wrapped (e.g. `do`/`match`) or abortive, reintroducing the "wrong fold" risk the guard is meant to
> prevent.

VERIFIED the code-vs-doc contradiction. The fn's doc (#2179 diff:17): "an arm body of a different shape is
conservatively treated as 'may perform outer' (decline — safe)." But the actual tail of the fn (diff:68-71):
  `match tail_resume(db, inner_arm.body) {
       Some((inner_val, _)) => resume_reaches_another_effect_op(db, inner_val, op_id),
       None => false,
   }`
So when `inner_arm.body` is NOT a bare tail `(resume v next)` — `tail_resume` returns `None` — the fn
returns `false`, i.e. "this resume value's op arm does NOT perform outer" → the CALLER (diff:91
`&& !resume_val_op_arm_also_performs_outer(...)`) treats it as SAFE-to-lift and proceeds. That's the
OPPOSITE of the documented "different shape → conservatively decline". So a depth-3+ chain whose
intermediate arm is WRAPPED (`do`/`match`) or abortive (not a bare tail resume) evades the guard and gets
the single lift that "does not chase" the deeper outer perform (diff:13-14) → the wrong-fold this guard
exists to prevent. CONFIDENCE: PLAUSIBLE-HIGH — the code demonstrably contradicts its own doc (verified),
and the failure mode is exactly the rn3-class wrong-fold; whether a wrapped/abortive intermediate arm is
actually CONSTRUCTIBLE + reaches this path (vs excluded upstream) is deep effects semantics = v-effects'
call. Fix per Copilot: the `None` (non-bare-tail) arm should return `true` (conservative decline) to match
the doc — a shape `tail_resume` can't analyze must be treated as "may perform outer". (If v-effects
intends some wrapped shapes to be safe, the doc should say which + the arm should analyze them, not blanket-
false.) v-effects owns rcdzc effects. PR OPEN → foldable. (This is the rn3 regression FIX; a guard that's
looser than its doc on the depth-3 case is worth nailing before it lands — same "predicate looser than its
contract" shape as my #2147 finding on the ao10 fix.)
