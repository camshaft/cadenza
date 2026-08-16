# flk ladder — flock quorum sensor (2026-08-16, tick 1622)

Attack: a helper def (`sgn`, itself a nested-if returning -1/0/1) called TWICE
in the aligned branch — once in the answer, once in the rebuild — with the
SAME compound argument `(- h heading)`; plus a band test built from two
comparisons as nested-if boolean AND (`(if (<= (- h heading) 2) (<= (- heading h) 2) false)`),
the trf1 condition shape with a symmetric-difference twist.

## Envelope
- flk1 (4 pings + quorum, helper x2 per branch): scratch-locals clean decline.
  Consistent with the compound-count law: sgn(d) + the band compounds count as
  multiple distinct shared subtrees.
- flk2 (3 pings): PASSES x3 all backends. Differential: seeds disagree on
  WHICH scout is the rogue (4 rejected at heading 7+ vs 9 rejected at heading
  6-) so the drifted headings differ and the sub-quorum read packs different
  everything (821 vs 521).

flk2 hand model: n=10 [71,904,82] quorum 821 → 71904082821;
n=0 [61,52,909] quorum 521 → 61052909521.

Pass ×3 wasm + rust + rust-async on trunk 91603aadc. flk1 held for (b).
Ops note: swm1 id was ALREADY TAKEN (sliding-window max, batch 276) — my
free-id grep ran before writing but I searched the PROBE dir output after the
corpus hit scrolled by; renamed flk. Grep result must be READ, not just run.
