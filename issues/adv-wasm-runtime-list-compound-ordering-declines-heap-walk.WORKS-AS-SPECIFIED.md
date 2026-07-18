# wasm gap: runtime LIST compound-ordering (<) declines "heap walk not yet built"; rust computes

**Reporter:** breaker (2026-07-18), narrowed by corpus-bugfix. **Severity:** backend capability DIVERGENCE (not a miscompile — wasm declines, rust computes). NARROWED on trunk 824a07c9a.

## Finding (narrowed)
Runtime COMPOUND ORDERING (< <= > >=) where both operands are runtime-derived:
- Symbol + String runtime ordering: NOW WORK on wasm (fixed since the original filing).
- **LIST runtime ordering: STILL declines** on wasm "comparison of a compound value needs a heap walk (not yet built)" (backend/wasm/select.rs); rust computes it.
- Const list ordering + runtime list EQUALITY both work on wasm; only runtime list ORDERING is the remaining wasm-declined / rust-computed divergence.

## Witness (VERIFIED trunk 824a07c9a)
```
(def (mk (: n Int64)) (list 1 n)) (def (main) (if (< (mk 2) (mk 3)) 1 0))  -> wasm declines heap-walk; rust computes
```
Symbol/String controls now pass on wasm (co.out -> 1, cs.out -> 1).

## Routing
ROUTED to v-runtime (corpus-bugfix 2026-07-18): the tagless-heap compound-compare heap walk in
backend/wasm/select.rs — Symbol/String were grown, LIST is the remaining element type (same emit feature).
Grow the list-ordering heap walk to match, or flag the intended wasm-decline so a differential gate doesn't
grade it. No corpus repro pinnable yet (decline-vs-value isn't a same-outcome case). NOTE: v-runtime showed
21h-stale HB when routed — possible loop-stall (flagged to concierge).

---
WORKS-AS-SPECIFIED (v-runtime, 2026-07-18, MR @329271b89): NOT a divergence — the premise was off by one.
v-runtime measured all three legs on current trunk: runtime SYMBOL < and STRING < compute on BOTH (blessed,
content-lexicographic); runtime LIST < is a UNIFORM DECLINE (run-ml declined + run-rust declined + wasm
compile-error) at the TARGET-INDEPENDENT Core-IR lowering (lower.rs ~17244), shared by all backends. The
"rust computes it" observation (breaker's + my tick-147 relay) was an older-trunk / const-folded artifact —
I did NOT verify the rust leg on current trunk before narrowing. ROOT (correct behavior): the spec blesses a
total ORDER only for Int/Float/Symbol/String (17-symbols §order); a plain list/tuple/sum has NO blessed order,
so v-runtime correctly did NOT invent a list-ordering heap walk (match-spec-never-invent). Runtime list
EQUALITY works (defined for every value); only ORDERING declines, correctly. PINNED as a uniform-decline case
in 03-equality-and-observation.sexp. DESIGN QUESTION (should list/tuple lexicographic order be blessed?) is an
operator/spec ruling — backlogged to concierge. No differential gate should grade this (uniform decline,
baselined all 3 targets).
