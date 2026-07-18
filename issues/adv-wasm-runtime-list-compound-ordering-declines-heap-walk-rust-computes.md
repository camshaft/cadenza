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
