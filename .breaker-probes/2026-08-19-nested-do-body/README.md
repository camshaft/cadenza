# nested-do-body — nested do-block as a handler body (regression pin for the printer fix 6345bd197)
## pynd1 — (handle E .. (do A (do B C))), 4 dispatches, value = final expr. Model 3040/2030. PASS x3, real-harness round-trip CLEAN.
This is the EXACT shape that broke the ML round-trip before v-syntax's printer fix 6345bd197 (the
finding breaker isolated + drove). Now round-trips clean; promoted as a REGRESSION PIN — if the
greedy-body nested-do printer path regresses, this corpus case fails corpus_roundtrip. Discarded
dispatches still advance the handler state (2030/3040 confirms). Promotable pass-witness.
