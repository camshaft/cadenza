# 2026-08-13 publish fan-out (tick 1423)

- `fan1.sexp` — observer-pattern fan-out: publish enumerates the subscriber map
  (Map.to-list) and REBUILDS THE WHOLE MAP bumping every value (per-dispatch
  full-map rewrite through a fresh Map.empty accumulator), answering the new
  value-sum via a second enumeration walk; a mid-run subscribe grows the fan so
  later publishes reach 3 subscribers where earlier ones reached 2. Two to-list
  walks + full rebuild per publish dispatch. Seeds shift the first publish
  (n=3 → sum 6; n=0 → 0) and everything downstream. PASS ×3 (6312052/306022).
