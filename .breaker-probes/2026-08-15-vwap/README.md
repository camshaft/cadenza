# vwp1 — volume-weighted average price tracker (2026-08-15, tick 1499)

(notional, volume) state: `trade` accrues (p*q, q) answering the running
notional; `vwap` answers truncated notional/volume, or -1 before any trade.
The LEADING -1 row drives the entire *1000-packed total NEGATIVE on both
seeds (-969945989885989 / -989965993905991) — the largest-magnitude negative
pins in the family, exercising negative-total packing arithmetic end-to-end.
Seed shifts one trade's price (15 vs 5), rippling through both later vwaps.

F24-safe: 6 dispatches but 2-branch × 2-tuple (the safe product).

PASS ×3 wasm. **Pool — fills the lhn1/bkf1/vwp1 trio.**
