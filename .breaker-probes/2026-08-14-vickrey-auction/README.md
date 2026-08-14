# bid1 — second-price (Vickrey) auction tracker (2026-08-14, tick 1463)

4-tuple handler state (high, second, winner-index, bid-count). `bid` answers the
CURRENT leader's index via a 3-way nested-if arm: a beaten high demotes to
second; a middle bid bumps only second; a low bid changes nothing. Closing
draws read the winner and the second-price he pays.

Seed-differentiated structurally: n=10 → bids (15,8,22,17), two lead changes
with demotions, mid-bid 17 bumps only second → 10103030317. n=0 → bids
(5,8,12,17), EVERY bid takes the lead (four lead changes, no mid-bumps)
→ 10203040412. The two seeds walk different branches of the SAME arm.

PASS ×3 wasm. 6 dispatches, zero chained lets in arms (3-way nested-if is
branch-only) — consistent with the odf1 cliff contrast. **Pool (batch-273).**
