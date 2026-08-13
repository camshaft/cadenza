# 2026-08-13 debounce gate (tick 1429)

- `dbn1.sexp` — (last-fire, pending) state: emit fires only when t-last >= 10
  (records t, CLEARS pending), suppressed emits stash v in pending; flush
  releases the stash. The interesting interplay: a FIRING emit discards any
  pending value (n=5: the suppressed 8 is then overwritten... no — the t=15
  fire clears it, so flush answers 0); n=12: both later emits fire/suppress
  the OTHER way and flush answers the stashed 9. Time-gap gate + side-slot
  stash + clear-on-fire coupling. vs rlm1 (window list): scalar-pair debounce.
  PASS ×3 (7000900/7080009).
