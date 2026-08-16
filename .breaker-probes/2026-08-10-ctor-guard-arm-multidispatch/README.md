# FINDING #15: ctor-pattern GUARD in a handler arm breaks under MULTI-DISPATCH (tick 1152, silent wrong x3)

- ggmin4: ONE dispatch, (guard (Wrap v) (> v s)) in the arm -> CORRECT (admits, 5).
- ggmin5: TWO dispatches, same arm -> BOTH dispatches return the FALLBACK (0), even the first
  (correct 50 = 5*10+0; ran 0). Silent wrong value on wasm+rust+rust-async.
- ggmin1/gg1: the original 2-dispatch faces (Box and generic Container) -> 0.
- ggmin2: same ctor-guard in BODY position -> CORRECT (guards fine outside arms).
- ggmin3: if-in-arm control (no guard) -> CORRECT across dispatches.
- CONTRAST landed gp1/gp4/gp5: SCALAR-binder and TUPLE-destructure guards in arms work across
  dispatches. The break is CTOR-pattern guards specifically.
Trigger: [constructor-pattern guard in a handler arm] x [>=2 dispatches of that op].
Suspect: the guard-desugar's arm copy for ctor patterns (guard-desugar duplicates the arm; the
multi-dispatch arm re-instantiation of the DESUGARED form loses the guard's admit path) - adjacent
to the #13 peel/arm-copy family and possibly the same locus class.

## Discriminator sharpened (tick 1153)
- gg2-scope: multi-variant guard w/ WILDCARD fallback (_other) -> CORRECT across dispatches.
- gg3-scope: single-variant guard w/ WILDCARD fallback -> CORRECT.
- gg4-scope: multi-variant guard w/ SAME-CTOR fallback ((Left _v) sibling arm) -> WRONG (0).
Refined trigger: [ctor-pattern guard] x [a SAME-CTOR non-guarded sibling arm] x [>=2 dispatches].
A wildcard fallback dodges it. The guard-desugar's duplicate-arm for the same ctor is the suspect:
the desugared (guard-hit | same-ctor-fallthrough) pair collapses wrongly on arm re-instantiation.

## GUARD NOT REQUIRED (tick 1154) - #15 is a same-ctor MULTI-ARM bug
- gg5-scope: same-ctor DOUBLE arm with NO guard ((Wrap 0) literal then (Wrap v) general),
  2 dispatches -> the LITERAL arm never matches on dispatch 2 (ran 50, correct 150).
- gg6-scope: ONE dispatch hitting the literal -> CORRECT (100).
FINAL trigger: [TWO same-ctor arms in a handler match] x [>=2 dispatches] - the guard was just
one way to produce same-ctor siblings. First-match-wins collapses to the GENERAL arm on
re-instantiation (the specific/literal arm is dropped).
