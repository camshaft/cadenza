# FOLLOW-UP (v-compiler-ml, self): 3+-parameter user functions decline (high corpus value)

Found 2026-07-20 (trunk 9df1855b2) probing user-function shapes.

## The gap
A 3-param (or more) `def` DECLINES while the reference runs it:
```
(do (def (f a b c) (+ a (+ b c))) (def (main) (f 1 2 3)) (export main))   ml=declined   ref=6
(do (def (f a b c d) …) …)                                                ml=declined   ref=…
```
It is an HONEST decline (intentional coverage-not-yet), NOT a miscompile: `sread`'s `read-second-param`
declines when a THIRD param is present (bodyId -1 sentinel), and there is no param3 table. 1- and 2-param defs
(incl. typed, incl. mixed after this session's work) all run.

## Corpus value: HIGH
3+-arg defs are pervasive in the real corpus / self-host source: `(def (eval-head h a b …))`, `(def (sl b s n))`,
`(def (go b i a))`, `(def (skip-elems b i k))`, `(def (entry-byte b e j))`, `(def (neq-go b e l))`, etc. — many
3- and 4-arg helpers. Widening to N params (or at least 3) materially raises run-ml conformance on the corpus.

## Scope (multi-file, MY lane) — mirrors the 2-param (slice-3d) build-out
The param machinery is structurally 2-capped (only `param`/`param2` tables + `arg2`). Adding a 3rd param:
1. **parse-db**: a `param3` table (`record-param3`/`param3-of`) + an `arg3` table (`record-arg3`/`arg3-of`),
   mirroring the param2/arg2 pair. Widen the `Tree.Arena` tuple (currently 7 maps) → 9, OR (cleaner) generalize
   to a `List(paramNodeId)` per def + `List(argId)` per call (a real N-ary rep — bigger but ends the per-arity
   table sprawl; decide at slice time — a `List`-based rep is the idiomatic end state and avoids a 4th/5th
   round of this).
2. **sread**: `read-second-param` → after param2, read an OPTIONAL param3 (typed or untyped) instead of
   declining; extend `read-def-form`'s 6-tuple to carry param3Id (→ 7-tuple), and `read-do-def` to
   `record-param3`. The call reader (`read-param-call`/`read-2nd-arg`) reads a 3rd arg → `record-arg3`.
3. **resolve-db**: extend the NApp arm's body-scope to bind param3 (like param2-of at resolve-db:80).
4. **infer-db**: `infer-param2` → also `infer-param3` (bind param3 : its declared/arg type, fit-check arg3).
5. **lower-db**: the 2-arg nested-CLet (`CLet(p1, a1, CLet(p2, a2, body))`) → a 3-deep nest with arg3/param3.
6. **eval-db/emit-db**: no change (they consume the lowered CLet nest; inline path already handles arbitrary
   CLet depth).
Gate: run-src @tests for `(f a b c)`, mixed typed 3-param, arg-count mismatch declines; reader @tests record
param3; full suite + W4 differential (run-emitted inlines the 3-deep CLet).

## Recommendation
STRONGLY consider the `List`-based N-ary param/arg rep (step 1 alternative) rather than a 4th hardcoded table —
it's the idiomatic end state (the operator's idiomatic-code directive) and stops the 2→3→4→5 table sprawl. That
makes this a slightly bigger but FINAL param-arity slice. Pick up on clean trunk (needs parse-db + sread + the
query columns, so a clean base with no pending MR touching them). This is the highest-value corpus-conformance
feature currently open (ahead of recursion, which is rarer in the integer corpus and needs a non-inlining call
form).

## UPDATE 2026-07-20: 3-PARAM (untyped) DONE — landed. REMAINING: typed-3rd + 4+-param.
The core 3-param slice LANDED this session (6b5f27d38, slice-3e): untyped 3-param defs `(def (f a b c) …)` +
`(f x y z)` parse/resolve/infer/lower/run, arity-checked (over-app→CDZ0203, under-app→CDZ0201). Wiring:
sread read-third-param (7-tuple) + record-param3/arg3; resolve binds param3; infer-param3; lower nests a 3rd
CLet; eval/emit unchanged (inline path). param3/arg3 tables landed in the prior foundation slice (21c92bbcc).
STILL OPEN (this file's remaining scope):
  1. TYPED 3rd param `(: c T)` — read-third-param declines a `(` (typed) 3rd param; add a read-typed-third-param
     analogue (mirror read-typed-second-param) + infer-param3 already fit-checks, so mostly reader work.
  2. 4+-PARAM — read-third-param requires `)` after param3 (4th declines). Needs param4/arg4 OR the List-based
     N-ary rep (RECOMMENDED — ends the 2→3→4 table sprawl; the idiomatic end state).
Both are lower-priority than the forward-ref/recursion PRE-SCAN (see recursive-user-functions follow-up), which
is the higher-value architectural feature. Corpus 4-arg helpers (sl b s n, go b i a, skip-elems b i k) need the
4+-param work eventually.
