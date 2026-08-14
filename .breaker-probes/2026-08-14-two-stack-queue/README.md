# tsq — two-stack amortized FIFO queue (2026-08-14, tick 1460)

Classic two-stack queue driven through handler state: `enq` pushes the in-stack;
`deq` pops the out-stack, reverse-refilling from the in-stack ONLY when the
out-stack is empty. Pins FIFO order surviving an interleaved enq between refills
(the mid-stream enq must NOT overtake the already-staged element).

- `tsqS.sexp` — 3-dispatch face (enq enq deq): PASS ×3 wasm. Exercises one full
  reverse-refill through `rev` + prefix-copy `dropl`. Seed-differentiated
  (n=10 → 111211, n=0 → 10201). **Pool candidate.**
- `tsq1.sexp` — 6-dispatch original (enq enq deq enq deq deq): DECLINES (todo)
  uniformly on wasm/rust/rust-async. NOT a bug — plateau-cliff class.
- `tsq4-declines.sexp` — 4-dispatch minimal decline witness (enq enq deq deq),
  uniform ×3 backends.

## Cliff datapoint (refines the plateau class)
Boundary here is dispatches ≥4 with this arm shape. The dual-use chained let
(`r` feeds BOTH the resume value via `lastv r` AND the next-state via `dropl`)
sits inside one BRANCH of the deq arm — branch-local dual-use still trips the
cliff. shrB2/shrC controls (in /tmp, 2 dispatches): dual-use let AND single-use
let both PASS at 2 dispatches, confirming the arm shape alone is fine; it is
the shape × straight-line dispatch count product that declines.
Flip-watch alongside plt1/fac1/xcl1.
