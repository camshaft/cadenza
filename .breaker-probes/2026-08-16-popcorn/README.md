# pop1 — popcorn kettle with threshold differencing (2026-08-16, tick 1598)

(temperature, popped) state with a let-free threshold-count callee (totpop:
0 below base, else (temp−base)/3 + 1): each `heat` raises the temperature 5°
answering how many NEW kernels popped — derived by DIFFERENCING the running
total against the stored count (binder-over-call at 2 consumers, boundary-
safe); `bowl` reads the total.

Bases 28 vs 18: the cool kettle pops a single late kernel (0,0,0,0,0,1 —
total 1) while the hot one accelerates (0,0,0,1,2,2 — total 5). The packed
totals differ in MAGNITUDE by 4 orders (101 vs 1020205) — extreme
leading-zero asymmetry, the widest packing-length spread in the pool.

PASS ×3. **Pool — fills drm1/grw1/pop1 (13th trio ready).**
