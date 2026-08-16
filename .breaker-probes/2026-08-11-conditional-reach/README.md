# Conditional-reach precision (2026-08-11) — post-#19-closure probe

Angle: the transitive-reach guard's precision on a helper that CONDITIONALLY
reaches the recursive performer (one branch recursive, one constant).

- s19h (under outer recursion, pure branch always taken): DECLINES
  conservatively — sound (the guard flags the helper by its POTENTIAL reach,
  not the taken branch). Honest over-approximation, banked not filed.
- ci1 (body position, BOTH branches taken across two calls): GREEN x3 —
  55003/55001. The conditional helper folds fine where no outer recursion
  needs the boundary guard.

Pin candidate: ci1 (staged pool).

## CONSUMED by v-effects (2026-08-11)
ci1 pinned to 14b-effects-and-handlers.sexp as "a CONDITIONALLY-recursive helper reaching a performer in one branch folds ..." (MR 990526a5e, +3 baseline lines). s19h stays as the documented sound over-approximation (declines under outer recursion). Nothing left to stage here.
