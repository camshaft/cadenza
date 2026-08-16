# Multibyte rope state + FINDING #18 (2026-08-11)

GREEN x3 (pin candidates):
- mb1: multibyte rope state grows a 2-byte scalar per dispatch; byte-len vs
  scalar-len diverge exactly at the 20-deep drain — 41021/1001

FINDING #18 (filed): wasm-only INVALID COMPONENT.
- Trigger: recursion-grown String state + String.at dispatch with a COMPUTED
  op-argument index. BOTH required. rust/rust-async run it green.
- Bisect: literal index OK / bare-param OK / computed (- n 1) inline or
  let-bound INVALID / computed without recursive walk OK. ASCII == multibyte.
- Queue: adv-string-at-computed-index-invalid-wasm.sexp (rust-verified pins).
- Same outward class as finding #4 (slot-clobber invalid wasm).
- mb2 (the 3-pick drain that found it) held back until the fix; then promote
  mb2 + the repro.

Vocab learned: String.scalar-at is COMPILE-TIME ONLY (runtime Char has no
representation; the error names String.at as the runtime one-scalar read).
Char.to-int/from-int exist; String.byte-len on a Char rejects (CDZ0203).

## #18 scope controls (tick 1214)
All /tmp: l18 (List.at, effect-grown) OK · b18 (Bytes.at) OK · s18 (String.slice
computed bounds) OK · s18b (String.at computed in BODY after dump) INVALID ·
s18c (PURE-recursion rope, computed String.at) OK · s18d (bare-param in body,
even out-of-range) OK.
Trigger = String.at + computed index + EFFECT-grown rope (any position).
Noted to corpus-bugfix: fix hunt = String.at emit when index is computed and
the string traces to a multi-value-upgraded state thread.

## #18 root evidence (tick 1215)
wasm-tools validate: func 18 "type mismatch: expected i64, found i32" @0x549.
Dump shows the pick emit tee-ing the i32 rope HANDLE into an i64 local — the
finding-#4 width-confusion slot class. Further gate refinement: walk bound may
be LITERAL (walk 4 + (- n 1) fails); (+ n 1) fails like (- n 1); (- n n)
CONST-FOLDS to 0 and passes (not a real computed index). Trigger final form:
computed-index String.at over ANY effect-grown rope state.

## #18 class widened (tick 1216)
- e18a: (Bytes.at (String.to-bytes s) i) computed-index over effect-grown rope
  = INVALID (same i64/i32 mismatch, func 17 @0x4fa)
- e18b: byte-len with computed-arg arm = OK (scalar reads fine)
- e18c: Bytes.len of the to-bytes view = OK (unindexed view fine)
Class final: ANY indexed read THROUGH A ROPE VIEW with a computed index, when
the rope came through the effect state thread. Native Bytes states differ
(b18 passed). Reported to corpus-bugfix.

## #18 final controls (tick 1217)
- q18: THREE sequential adds (no recursion) + computed pick = OK — recursion
  (the multi-value upgrade) genuinely required.
- t18: rope inside a TUPLE state + recursive walk + computed pick = INVALID
  (func 19) — compound-state field position does not shield.
- h18: abort-arm computed read after ONE non-recursive add = OK (consistent).
Net trigger: multi-value-upgraded recursive growth of a rope-bearing state +
computed-index rope-view read anywhere downstream.

## Post-fix sweep (tick 1243, fresh binary IN-CHAIN per the new rule)
- mb2 GREEN x3 (the probe that found #18 — now promotable).
- mb1/dr1/dr2 re-green on wasm post-fix (no collateral).
- x18 stress: two computed picks (in-range + out-of-range at 2n) over a
  500-deep effect-grown rope — 9/9 exact. The scratch-floor float holds at
  depth and on the OOR face.
