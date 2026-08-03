# adv-50 RESIDUAL — closure as Map-KEY / Set-element: wasm computes vs rust E0277 divergence

Post-fix residual on the RESOLVED adv-50 (fix 2f59126b1 landed). breaker note 21381 (trunk 78a7a7c29).

## Divergence
- s14 (closure as Set ELEMENT) + s15 (closure as Map KEY): wasm COMPUTES (106), rust FAILS BUILD (E0277 `dyn Fn: Ord`).
- Map VALUE position is fine on both — only key/element (need equality/order) diverge.

## Spec analysis (mine) → recommend UNIFORM REJECT at type-check
- collections-and-text.md:162: map membership decided by key's value under §Equality Is Structural.
- equality-is-structural: value equality MUST agree with the CANONICAL BYTE FORM.
- A closure has NO canonical byte form (captures env; no content-equality for fns). Unlike Float (canonical-byte eq carve-out, eq-but-not-orderable, 03-equality:684), a closure has NEITHER blessed eq NOR order.
- So wasm's invented identity (106) is the MISCOMPILE; rust's E0277 is directionally-right-but-ungraceful. Correct = uniform compile-time reject (CDZ0202-family, cf. abstract-key precedent 11-modules:1435-1450).

## Status
- ASK sent to concierge (which code: CDZ0202 or dedicated non-equatable/orderable diagnostic).
- Divergence flagged to v-rust-backend + v-effects (closure-emit owners) — likely resolves via FRONTEND reject, not backend emit.
- breaker's s14/s15 witnesses banked. PIN-ON-RULING: author corpus pin (my zone) once concierge rules.

## PIN SPEC (v-inference confirmed CDZ0216, note 21469; reject-emit MR f49692f13 in flight)
Author into 19-sets.sexp (or 05-compound) once f49692f13 LANDS + I can sync (TODO→reject flip):
- REJECT (error CDZ0216): `(Set.of (list (fn (x) (+ x 1))))`  — a Set of functions
- REJECT (error CDZ0216): `(Map.insert Map.empty (fn (x) x) 1)` — a Map keyed by a function
- LEGAL control: an Int64-keyed/elem Set + Map stay legal (value-typed key OK)
BOUNDARY (do NOT conflate): a direct `(= fn fn)` rejects with PRE-EXISTING **CDZ0203** (fn-operand
arm, "not defined on a function value"), NOT CDZ0216. CDZ0216 is scoped to the KEY/ELEMENT position.
If pinning the (= fn fn) face too, pin it as CDZ0203. Gate against trunk AFTER f49692f13 lands.
Source: breaker s14/s15. Batch with adv-51 + adv-53 + pr1216 doc when the baseline lane frees.
