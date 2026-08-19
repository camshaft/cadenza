# string-state-thread — String handler state threaded via String.concat
## pystr1 — grow() concats a suffix answering String.scalar-len; two grows + a size read. Model 50808. PASS x3, round-trip clean.
Heap String state survives resume threading; scalar-len read is consistent. (API: String.scalar-len/byte-len, NOT String.len.) Promotable.
