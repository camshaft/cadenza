# 2026-08-14 Markov bigram counter (tick 1455)

- `mkv1.sexp` — the map key is a PAIR BUILT IN THE ARM from state and argument:
  (prev, v) — the same value arriving after different predecessors lands in
  different buckets. State = (prev, tuple-keyed map) with the ascribed
  Map.empty seed (the tk-ann1 workaround face). n=2 makes every feed the (2,2)
  bigram after the first (counts 1,1,2,3,4); n=5 alternates (5,2)/(2,5)
  (1,1,1,2,2). The composite-key-FROM-STATE face — tk-ann1 keys from op args
  alone. PASS ×3 (11234/11122).
