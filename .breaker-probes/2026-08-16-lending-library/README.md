# lbr1 — lending library with hold queue (2026-08-16, tick 1610)

Attack: three-op protocol where the RETURN branch's answer packs a compound
`(* late (+ (% n 3) 1))` that also feeds the state rebuild (fines) — the seed
compound appears 4x across the 2-branch ret arm (both answers, both rebuilds).
The hold-interception branch mutates a DIFFERENT field than the plain branch
(holds-1 vs avail+1) while both accumulate fines — cross-field asymmetric
rebuild under a shared compound.

Differential: the seed scales ONLY the fine multiplier — circulation rows
(borrow/hold/interception) are byte-identical across seeds; the fine rows and
audit diverge (490/410 vs 290/210). A constant-folder that specializes the
fine compound per-seed must still keep the shared circulation rows identical.

First 3-op + hold-queue-interception theme. Weak-pin lesson re-applied: first
draft had the fine only in the audit row (single-row differential) — repacked
the per-return fine into the ret answers.

Hand model: n=10 rows [11,1,12,490,10,410] → 11001012490010410;
n=0 rows [11,1,12,290,10,210] → 11001012290010210 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 68122fd42.
