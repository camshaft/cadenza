# qrm1 — weighted quorum with punitive veto (2026-08-14, tick 1483)

3-op handler over (weight, threshold): `vote` accumulates answering the
quorum bit; `veto` zeroes the tally AND raises the threshold by 2 (the
threshold itself is MUTABLE state, seeded at n+6); `tally` packs weight*10
plus the quorum bit.

Seed contrast: n=0 (thr 6) reaches quorum on vote 2, again after the veto
(thr 8, votes 9+8=17≥8 → both post-veto votes answer 1, final tally 171);
n=10 (thr 16) NEVER reaches quorum — every vote answers 0 and both tallies
lack the bit (90, 170). Same vote stream, opposite outcomes end-to-end.

PASS ×3 wasm. **Pool (next-next-next trio).**
