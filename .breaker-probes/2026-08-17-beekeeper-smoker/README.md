# bkp1 — beekeeper's smoker (2026-08-17, tick 1708)

Attack: a RESOURCE-GATED action where the success cost is a different
constant than the gate (needs 4, costs 3 — the gap means calm 4 succeeds
into calm 1, immediately below the NEXT gate: a success that disables its
own repeat). The sting branch resumes with only the counter bumped (calm
frozen below the gate — an absorbing-ish failure recoverable only by the
other op). Puff cap answers a bare constant 99.

Differential: calm 5 vs 2: n=10 pulls clean (12 — calm 2), puffs to 6,
pulls (23 — calm 3), then STUNG (901); n=0 stung first (901), puffs to 6,
pulls (13), stung again (902). Reads 231 vs 132 — frames and stings swap
counts exactly.

Hand model: n=10 → 120600239010231; n=0 → 9010600139020132 (mixed base).

Ops: first id smk1 TAKEN (string-keyed buckets, batch ~245 era) — grep read,
renamed bkp1.

Pass ×3 wasm + rust + rust-async on trunk 0657b816d.
