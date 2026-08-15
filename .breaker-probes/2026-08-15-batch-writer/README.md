# bch1 — batch-coalescing writer (2026-08-15, tick 1562)

(batch-sum, batch-count) state: `write` buffers answering the count until
the THIRD item self-flushes — answering 100+sum and resetting; `sync`
force-flushes the partial answering its sum, or -1 when empty. Note the
self-flush arm answers with the sum INCLUDING the arriving item without
storing it (the (+ bsum v) compound goes to the answer while the state
resets — a consume-on-flush shape).

Two seed-shaped values land in DIFFERENT batches ((n%4)+2 in batch 1,
(n%3)+1 in batch 2), so both the partial-sync row (8 vs 6) and the
self-flush row (116 vs 115) carry the seed; the count rows and the empty
sync (-1) are shared anchors. First draft had one seed value — the sync row
alone diverged (weak); moved a second seed into batch 2.

PASS ×3. **Pool (11th trio seed).**
