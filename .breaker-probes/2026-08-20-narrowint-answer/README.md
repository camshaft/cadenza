# narrowint-answer — a BARE narrow-int op-result / resume-answer type declines the fold

## Generalizes pyu8w1 (narrow-int STATE) to narrow-int ANSWER — and it's broader
pyu8w1 found a UInt8 *handler state* threaded across >=2 dispatches declines. This bank
shows the gap is really a **bare narrow-int OP-RESULT type** anywhere in the fold, and it
declines even at SINGLE dispatch.

## pyu8a1 — canonical witness (UInt8 op result, Int64 state, 3 dispatches)
```
(effect E (op tick (-> UInt8)))
(tick () s (resume (UInt8.wrapping-add 250 (UInt8.of s)) (+ s 1)))
```
Oracle 276453 / 275352 (n=10: answers 251,252,253; n=0: 250,251,252; body 1000d1+100d2+d3).
DECLINES uniformly wasm+rust+rust-async ("not yet reducible by the tail-resumptive fold").
SAFE over-decline (reject, not miscompile) — verified todo x3, never a wrong value.

## Isolation controls (all on trunk 977b17e2f)
- C1 Int64 answer, IDENTICAL multi-dispatch body: COMPILES. -> not the body shape.
- C2 UInt8 answer, SINGLE dispatch: DECLINES. -> not dispatch count.
- C3 UInt8 LITERAL answer (no UInt8 arithmetic), single dispatch: DECLINES. -> not the
  UInt8.of / UInt8.wrapping-add ops; the ANSWER TYPE alone triggers it.
- C4 UInt8 literal answer, body IS the perform (fully tail): DECLINES. -> not two-hole.
- C5 (Option UInt8) answer (BOXED narrow int), single dispatch: COMPILES. -> BOXING dodges
  it; the gap is a BARE (unboxed) narrow-int op-result representation.
- C6 Int64 literal answer single dispatch: COMPILES (baseline).
- C7 UInt16 answer: DECLINES.  C8 Int32 answer: DECLINES. -> ALL bare narrow ints, not
  UInt8-specific.

## Conclusion (filed to v-effects as a pyu8w1 follow-on)
The tail-resumptive fold declines when the OP RESULT TYPE is a bare narrow int
(UInt8/UInt16/Int32) — independent of dispatch count, arm shape, or whether narrow-int
arithmetic appears. Int64 op results fold; boxing the narrow int (Option UInt8) folds.
Consistent with the fold materializing the resume-answer through an i64-width slot
(#16-23 width-alias class) that has no bare-narrow-int representation yet. SAFE decline.
Decline-witness pyu8a1 (oracle 276453/275352) auto-flips to pass when the fold admits
bare narrow-int op results.
