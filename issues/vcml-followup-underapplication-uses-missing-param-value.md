# FOLLOW-UP (v-compiler-ml, self): under-application binds fewer args than params — body that DOESN'T use the missing param silently runs

Found 2026-07-21 (trunk 2bc0ba7ea) while wiring param4 (slice-3f). An UNDER-applied call — fewer args than the
def has params — does NOT uniformly decline. It declines ONLY if the body actually references the unbound param
(unbound NVar → resolve/infer poison → decline); if the body ignores the missing param, it silently runs.

## Repro (ml compiler; consistent across ALL arities)
```
(do (def (f a b)     (+ a a))   (def (main) (f 1))     (export main))   ml=value 2    ref=? (expect decline/arity error)
(do (def (f a b c)   (+ a b))   (def (main) (f 1 2))   (export main))   ml=value 3    ref=?
(do (def (f a b c d) (+ a b))   (def (main) (f 1 2 3)) (export main))   ml=value 3    ref=?
```
Contrast — a body that USES the missing param correctly declines (existing test se-three-param-under-application-declines
uses body `(+ a (+ b c))`, so 2-arg `(f 1 2)` leaves `c` unbound → declines):
```
(do (def (f a b c) (+ a (+ b c))) (def (main) (f 1 2)) (export main))   ml=declined   (c unbound)
```

## Mechanism
A call lowers to a nested CLet, ONE per ARGUMENT present (arg2-of/arg3-of/arg4-of drive how many CLets nest).
When fewer args than params are supplied, the trailing params simply get no CLet binding. If the body never
references them, lowering/eval succeed — the def is effectively treated as its lower arity. The arity MISMATCH
(param count vs arg count) is never checked directly; the only thing that catches it is a downstream unbound-var
poison, which requires the body to actually use the missing param.

## Why NOT fixed in slice-3f
Slice-3f wired the 4-param RUN path (the corpus's calls are always FULLY applied — 4 args to a 4-param def — and
those all work + are gated). This under-application gap is PRE-EXISTING (present since the 2-param slice, not
introduced by param4) and orthogonal to the wiring. The clean fix is a direct arity check in read-*-call / lower's
NApp arm: count the def's declared params (param-of/param2-of/param3-of/param4-of present) vs the args recorded
(argId, arg2-of..arg4-of) and decline on mismatch (CDZ0201 under-app / CDZ0203 over-app), rather than relying on
the unbound-var poison. Over-application is ALREADY handled (the trailing-token guard declines a >Nth arg); this
is the under-application complement. Small, self-contained, MY lane (sread call-readers + a lower/infer arity
guard). Verify against rcdzc for the exact diagnostic (CDZ0201 vs a dedicated arity code).
