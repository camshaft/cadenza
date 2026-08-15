# bnf — anti-starvation queue: a 3-TUPLE + range-guard decline (2026-08-15, tick 1537)

Teller queue over id ranges: join grows back answering length; serve advances
front (every 3rd from the back), drained answers -1.

| probe | shape | verdict |
|-------|-------|---------|
| bnf1 | (front,back,k), phase check % in arm | DECLINE ×3 |
| bnf1+binder | phase hoisted through match binder | DECLINE ×3 |
| bnf1+flat | drain guards flattened ahead | DECLINE |
| bnfC | phase check CONSTANT (= 0 1) | DECLINE |
| bnfD | bump dropped, single update-site, still 3-tuple | DECLINE |
| bnfE | 2-TUPLE (front,back), no counter | **PASS ×3** |

The frontier here is the 3-TUPLE with the range guard (< front back) — even
with a constant phase and a single update site (bnfD). bnfE's 2-tuple twin
compiles. Same family feel as phs (cross-field guard: the guard reads
front×back while updates write front|back|k) but SHARPER: bnfD has NO third-
field read in any branch, only the k+1 write, and still declines — the mere
PRESENCE of a third written-but-unread field trips it. Flip-watch with phs1.

bnfE is corpus-eligible (plain FIFO face). Noted for the fold-owner context;
not separately routed (phs note already covers the family — this adds the
written-but-unread-field datapoint to the bank).
