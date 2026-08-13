# 2026-08-13 high-water Bytes state (tick 1398)

- `bhw1.sexp` — byte-lexicographic MAX threading raw Bytes: put keeps the winner
  via `<` on Bytes, both-branch resume (fresh-into-state vs keep). The decisive
  comparison: [0xC3,0xA9] must rank UNSIGNED above [0x7A] ('z') — a signed byte
  compare would keep 'z'. Third put seeded [n,200]: n=50 loses to the 0xC3
  champion, n=250 wins (winner len flips 2→2... both len 2 — len pins which
  SEQUENCE won only via the b/c verdict digits). Bytes `<` exists but had NO
  corpus usage anywhere; scc1 pinned the String face — this is the raw-bytes
  twin incl. >0x7F values. PASS ×3 (1102/1112).
