# State-binder guards in arm matches (2026-08-11)

Angle: guards in arm matches are pinned over PAYLOAD structure (gg/gp
families) and performing guards reject (CDZ0407); a PURE guard comparing the
payload to the STATE binder (routing that changes as state advances) was
uncovered.

GREEN x3:
- sg1: (guard x (< x s)) — the SAME payload routes differently as the state
  walks past it — 300300/300003
- sg2: the guard calls a PURE def over payload AND state (allowed per
  CDZ0407's pure-def-call carve-out); nearness verdict flips per dispatch —
  111/0

Pin candidates: 247 pool.
