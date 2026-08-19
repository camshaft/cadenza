# set-dedup-state — Set-state handler, dedup + contains across the seam
## pysd1 — add(k) threads Set.insert (dedup, re-add no-grow) answering size; has(k) reads contains. Model 2110. PASS x3.
CHAMP Set dedup + membership survive resume threading. Promotable. (API: Set.of (list ..), Set.insert/len/contains.)

## 🩸→✅ ROUND-TRIP FINDING CORRECTED: it was NESTED-DO, not Set.of(list)
Original suspicion (Set.of(list) breaks round-trip) was WRONG — v-metaprogramming correctly
noted the (list ...) -> ("list" ...) head-flip is cosmetic (structurally_eq collapses it). The
REAL trigger, isolated by structural debug-diff + minimal harness cases: a NESTED do-block
(do A (do B C)) does NOT ML-round-trip — the ML surface FLATTENS it, changing the AST node
count (7160 vs 7161). Minimal repro (FAILS harness): handler body (do (E.tick) (do (E.tick)
(E.tick))). FLAT form (do (E.tick) (E.tick) (E.tick)) round-trips CLEAN (harness ok).
FIX: rewrote pysd1's body to a single flat do — now passes the real corpus_roundtrip harness
x3 + gate x3 (2110). pysd1 is now PROMOTABLE. Filed corrected finding to v-metaprogramming
(nested-do ML-surface flattening, surface round-trip only, not soundness).
LESSON: use the REAL corpus_roundtrip harness to isolate, NOT the cdz convert CLI text-diff
(the CLI always shows the cosmetic list head-flip and misleads).

