# Decline-cliff discriminator matrix: MIXED-OP requirement (2026-08-14, tick 1468)

Isolated the tsq4 decline factor by walking a same-arm ladder at 4 straight-line
dispatches. Every probe carries the dual-use let (bound list feeds BOTH the
resume value and the next state). All verdicts uniform wasm/rust/rust-async.

| probe | shape | verdict |
|-------|-------|---------|
| lstD | dual-use List.push let, no branch, 1 op | PASS |
| lstB | + capacity branch around the let | PASS |
| lst2 | + tuple-of-two-lists state | PASS |
| lstR | let built by recursive copy def | PASS |
| lstS | let feeds resume + SECOND recursive call (dropl) | PASS |
| lstQ | full tsq deq arm (rev + dropl refill), 4x deq | PASS |
| lstU | lstQ + enq arm DECLARED+HANDLED, never performed | PASS |
| lstM | lstU with dispatches enq,enq,deq,deq (MIXED) | DECLINE ×3 backends |

**Conclusion: the cliff = dual-use-let arm × ≥4 straight-line dispatches ×
≥2 DISTINCT ops actually performed.** Same-op streams of any of these arm
shapes compile. This subsumes the tsq4 datapoint (mixed enq/deq) and explains
odf1/bid1/cbk1 passing at 6-7 dispatches: their multi-op arms are let-free.
pid1/pidL/pidW/pidT (P+I controller, single-op step× five + integ readout —
note integ IS a second op but its arm is let-free): all PASS, consistent —
the SECOND op's arm must matter only when the let-arm op is interleaved with
another op? lstM's enq arm is let-free yet declines — so the requirement is
mixed dispatches where AT LEAST ONE dispatched op has the dual-use-let arm.
pidL passes with 5 step + 1 integ mixed... BUT pidL's let chain feeds pv2 to
both slots — dual-use — and it PASSES with mixed ops. Distinction vs lstM: the
dual-use let in pidL is scalar-tuple, in lstM it is a LIST (heap value).
Refined claim: mixed-op × dual-use let ON A HEAP (List) VALUE × ≥4 dispatches.
pidT (tuple-valued dual-use let, mixed ops) PASSES → tuple ≠ List confirms the
heap-list factor.

## Round 2 (tick 1470): heap-type and writer sweep — all PASS except lstM stands
| probe | shape | verdict |
|-------|-------|---------|
| tplM | String-valued dual-use let, mixed ops | PASS |
| mapM | Map-valued dual-use let (Map.insert), mixed ops | PASS |
| lstX | List.push dual-use let, mixed w/ passive reader | PASS |
| lstY | recursive-copyapp dual-use let, mixed w/ passive reader | PASS |
| lstZ | FULL tsq deq arm (rev+dropl), mixed w/ passive reader | PASS |
| lstW | dual-use-let writer × dropl writer, single list | PASS |
| lstP | two dual-use-let writers, tuple-of-two-lists | PASS |
| lstF | dual-use-let writer × both-slots SWAP arm | PASS |

**lstM remains the ONLY mixed-op decline.** Sharpened conjunction: the decline
needs the tsq deq arm's refill branch — which writes BOTH tuple slots through
TWO recursive calls (rev then dropl) downstream of one dual-use let — mixed
with a second op that WRITES the same tuple state (enq). The same arm beside a
passive reader (lstZ) compiles; every simpler write-write mix compiles. This is
now a single-shape residual, not a class: parked as a narrow decline witness,
flip-watch alongside plt1/fac1/xcl1/tsq4.
