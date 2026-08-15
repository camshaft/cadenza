# pgn1 — pagination cursor over a seed-sized collection (2026-08-15, tick 1553)

(cursor, pages) state over a seed-sized collection (20+n): `next size` serves
up to the page size answering the count actually served — the LAST page runs
short; a drained cursor answers -1; `rewind` resets the cursor answering
pages served (the count survives the rewind). The smaller collection drains
one page earlier: its short page (4) and its -1 land at rows 3/4 where the
larger runs 8,8,8,6 — then both serve a fresh 12 post-rewind.

3-branch arm (drained / short-page / full-page), 2-tuple — envelope-safe.
PASS ×3. **Pool — fills gsc1/vnd1/pgn1 (tenth trio ready).**
