# The arity subset of the type-checker landed first, exactly as scoped — with a let-form tail the fixed-arity check didn't reach

*2026-07-07*

**What happened.** ask-30 (the self-hosted compiler has no type-checker — it compiles ill-typed programs native
rejects) was split, over prior cycles, into two passes of different cost: a cheap **arity/well-formedness check**
(~10 of the 33 mis-accepts, one guard in `read-app`, independently landable) and a **type-inference/rejection
pass** (~20, needs kinds across branches/operands/match). This cycle the spike landed the arity subset, and
re-probing the running compiler confirmed it lands exactly as scoped: `(+ 1)`, `(+ 1 2 3)`, `(if true 1)`,
`(< 5)`, `(not 1 2)` all moved from mis-accept (emitting a truncated value — `(+ 1)` → `i64.const 1`, dropping
the `+`) to a clean **decline** (the component traps); well-formed forms are unregressed (`(+ 1 2)` → 3,
`(if true 10 20)` → 10). Byte gate: 59 → **61 agree**, 148 → **136 disagree**; standing full-oracle WRONG sweep
stayed **0**. Of the original 33 native-rejected mis-accepts, ~12 moved to decline; **21 remain**, and
categorizing them confirmed the split and surfaced a boundary:

- **~19 are type-inference cases** (the bigger half, still open): int-vs-float no-promotion across *all* of
  `+ - * / % & | ^ << >> < > <= >=` (CDZ0301), mismatched-type operations, int/float `if` branches, ordering
  int-vs-string, non-list quasiquote splice, and `match` exhaustiveness. These need the pass that *rejects* on a
  kind mismatch — `kind-of`/`build-ktab` compute kinds but do not yet reject.
- **2 are a LET-FORM tail the fixed-arity check did not reach:** "a bare binding form with no bindings and no
  body" / "a binding form with bindings but no body." The `read-app` fix checks each *fixed*-arity form's operand
  count (`if`=3, `not`=1, binop=2), but `let` is *variable*-arity (a bindings list plus a body), so it falls
  outside the fixed-count check. Confirmed still mis-accepted (`(let () )` → Ok). It needs a small `read-let`
  well-formedness check (bindings present, body present) — the same reader-side structural pattern, one form over.

**Why.** The value here is the confirmation that the earlier enumerate-then-find-the-root-cause analysis was
right and actionable: "the ~10 arity errors are one missing guard in `read-app`, not ten checks" held — a single
fixed-arity check moved the whole fixed-arity subset from mis-accept to decline in one landing, and the split
("land the cheap arity half first, before the type-inference pass") was the correct sequencing. But the re-probe
also caught what the analysis *rounded off*: "arity subset" was really "fixed-arity subset," and the two
`let`-form cases are variable-arity, so they need their own (small, same-shaped) check. This is the recurring
lesson at the finest grain — **a subset named by a category ("arity errors") can hide a member that doesn't fit
the category's implementation shape (a fixed-count check); enumerate the residue after a landing, not just before
it, because the fix's boundary (fixed-arity forms) is narrower than the category (all malformed-form errors).**
The mis-accept → decline move is itself the right reject-don't-miscompile progress: these cases now trap
(honest) rather than emitting a truncated value (a miscompile); `→ agree` still awaits the diagnostics ABI for
the coded `malformed … form` message.

**The requirement it drove.** No new corpus case — the arity and let-form cases are *already* pinned (native
rejects them; that's how the byte gate found them); the gate measured the mis-accept → decline flip directly (+2
agree, −12 disagree). ask-30 updated with the verified split: arity/well-formedness subset ~done (fixed-arity
forms), with a **let-form tail** (2 cases, one small `read-let` check) and the **type-inference subset (~19,
dominated by int-vs-float no-promotion)** as the remaining real work, plus the diagnostics ABI for `→ agree`.
General lesson: **after a subset-fix lands, re-enumerate what's left — the fix closes exactly the shape it
matched (here fixed-arity forms), and the residue tells you both the next subset (type inference) and the fix's
own boundary (a variable-arity `let` the fixed-arity check couldn't reach).**
