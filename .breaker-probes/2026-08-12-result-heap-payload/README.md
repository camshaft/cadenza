# 2026-08-12 Result-with-heap-payload (tick 1331, base post-238 trunk)

- `rsl1.sexp` — op result `(Result (List Int64) Int64)`: the arm Errs on the empty
  state and Oks the LIVE list snapshot once grown; both variants cross resume and a
  helper scores them (10*len + head via nested Option-match, or negated Err code).
  No prior Result-with-heap-payload through a dispatch anywhere in 14* (rsw1 et al
  are all scalar payloads). Explicit `(: (Err 7) ...)` annotation grounds the Ok arm's
  type at the Err site. Seeds shift the head digit (-677 / -630). PASS ×3.
