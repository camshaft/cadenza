# Perform-at-scale divergence hunt (2026-08-11) — FINDING #16

GREEN x3 (pin candidate):
- ps1: 100k-iteration tail loop PERFORMING every iteration, unobserved —
  constant stack on all 3 — 4999950000/3

FINDING (filed): observed tail performer loses constant stack on WASM only.
- ps2 grow-then-drain at 50k: wasm trap (stack exhausted), rust PASS,
  rust-async HANG (gate timeout).
- Bisect ladder (all /tmp): ps-scalar (discard-tail, no observer) 50k OK;
  ps-heap (List state, no observer) 50k OK; ps-twoconst (2 ops, const tail)
  50k OK; ps-two / ps-lettail (post-recursion (Acc.size) observer) trap
  between 5k and 8k — scalar and heap alike. The discriminator is the
  OBSERVED out-state (multi-value upgrade), which de-tail-calls the wasm loop.
- rust-async second face: 10k passes, 50k hangs (not traps).
- Queue: adv-observed-tail-performer-loses-constant-stack-wasm.sexp (10k pin:
  rust+rust-async pass, wasm traps). Issue to corpus-bugfix + concierge backlog.

## Value-correctness companions (same tick+1)
- px1: observed loop at depth 100/0/7 — VALUES agree x3 inside the wasm stack
  window (the divergence is stack-depth only, not value corruption) — 100/0/7
- px2: TWO observed walks back-to-back, second re-enters the upgraded loop
  after the first drain — 5100/0, green x3
Both are pin candidates NOW (value faces, depth-safe); the 10k differential
joins on fix-land.
