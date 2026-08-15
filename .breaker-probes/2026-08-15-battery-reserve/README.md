# chg1 — battery controller with protected reserve (2026-08-15, tick 1517)

SCALAR level state: `charge` clamps at 100; `draw` refuses any request that
would dip below the seed-shaped reserve (n+10), answering the NEGATED
shortfall with the level untouched — the refused row's level then survives
into the next draw, pinning refusal-leaves-state-intact through subsequent
dispatches. The same draw sequence trips the refusal at DIFFERENT points:
n=10 refuses draw 3 (-7) and completes draw 4; n=0 completes draw 3 and
refuses draw 4 (-27) — refusal positions swap.

2-branch scalar arms, 6 dispatches — envelope-safe. PASS ×3. **Pool.**
