# RESOLVED (#6693) — a `#set(<in-scope-elem>)` membership pattern spuriously raised CDZ0306

**Status:** RESOLVED by v-ast-compound #6693 (`fix(rcdzc): a #set(e…) membership element is a value
expression, not a binder — no spurious CDZ0306`). Correct behavior now pinned in
`spec/semantics/19-sets.sexp` (v-rcdzc-ts-2, batch-100).

## Original edge-hunt observation (v-rcdzc-ts-2 batch-97)

Probing `match` over native `#set(…)` patterns, `(match #set(1 2) (#set(a) 9) (_ 0))` reported
CDZ0101 "unbound name `a`". I initially proposed the fix "a set pattern cannot bind its elements".

## SPEC CORRECTION (v-spec-oracle, relayed via v-rcdzc-test-shrink) — that proposal was WRONG

Per § *A Set Is Matched By Element-Membership Patterns*, a set-match element is an **ordinary value
expression** (the set twin of a map KEY), **not a binder**. A `#set(e…)` pattern matches iff the
scrutinee CONTAINS each element's value. Therefore:

- `#set(a)` with `a` **not** in scope → CDZ0101 "unbound name `a`" is **CORRECT** — `a` is a genuine
  (unbound) value reference, not a binder. A "set cannot bind" reject would have been wrong; it would
  break the valid in-scope `#set(k)` membership case.
- The **real** bug this repro exposed: a **spurious CDZ0306** "unused match binding" on an **in-scope**
  set element (`#set(k)` for a parameter `k`) — `arm_pattern_binders` wrongly collected the value
  expression `k` as a binder. Fixed by #6693.

## Post-fix pins (spec/semantics/19-sets.sexp)

- "a set pattern with a RUNTIME in-scope element matches by membership of its value" — `#set(k)` for a
  param `k` matches when `k`'s value is a member (`f(1)`→9, `f(5)`→0), NOT flagged unused (the #6693
  regression guard).
- "a set-pattern element that names no in-scope value is an unbound value reference (CDZ0101)" — pins
  that the unbound case is correct (a value ref, not a binder).
- The literal-element containment cases ("Set patterns match by CONTAINMENT", batch-97) remain correct:
  a literal is a value expression, and each listed element must be a member.

v-ast-compound was also asked to add a clarifying note to the CDZ0101 message steering an author who
meant to bind toward `Set.contains` / `Set.len`.
