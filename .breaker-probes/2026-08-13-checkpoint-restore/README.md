# 2026-08-13 checkpoint/restore protocol (tick 1418)

- `ckp1.sexp` — (live, shadow) state pair: save copies live→shadow, work mutates
  ONLY live (2x+v), restore copies shadow→live; two works between save and
  restore are fully discarded and the post-restore work re-derives from the
  checkpoint (row f = 2·checkpoint+0, proving the restore). vs und1 (one-deep
  delta undo) and ckp-style swap in swp1 (exchange, not copy): this is the
  copy-semantics pair with an asymmetric writer. PASS ×3 (707190400714/
  101070160102).
