# grh1 — greenhouse auto-vent (2026-08-16, tick 1656)

Attack: a LATCHING side-effect in a compound condition — the sun arm's
boolean-AND-as-nested-if `(if (> (+ t d) 30) (= v 0) false)` trips ONCE
(vents latch open, no re-alarm on later crossings since v==1 kills the AND),
and the mist arm's behavior FORKS on that latched field (net +2 with a
9-tag vs net +3 plain). Cross-op state coupling through the latch, with the
alarm answer using a different encoding (800 + t%100) than the plain row
(t*10 + v).

Differential: starting warmth 28 vs 24: n=10 trips the auto-vent on sun #1
(832 alarm) so BOTH mists bleed (79, 99); n=0 trips on sun #2 (831) so mist
#1 is plain (80) and only mist #2 bleeds (109). Read 3591 vs 3201.

Hand model: n=10 → 8320793510993591; n=0 → 2800808311093201 (mixed base:
4 rows base-1000 + read base-10000; 5-op draft overflowed, trimmed + vent
op dropped).

Pass ×3 wasm + rust + rust-async on trunk bc7437703.
