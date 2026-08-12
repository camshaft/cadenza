# 2026-08-12 string-keyed buckets (tick 1330, base post-238 trunk)

- `smk1.sexp` — (Map String Int64) handler state with keys BUILT IN THE ARM:
  String.concat of the prefix arg + a parity-routed suffix ("-e"/"-o"), then
  accumulate-or-insert answers the bucket's new total. First string-keyed Map
  state in 14b/14c (7 prior uses all in 14 part 1); combines arm-built rope keys
  with the lookup-match accumulate idiom. Seeds route to different buckets
  (n=4: a-e/a-e/b-o = 41005; n=7: a-o/a-o/b-e = 71608). PASS ×3.
