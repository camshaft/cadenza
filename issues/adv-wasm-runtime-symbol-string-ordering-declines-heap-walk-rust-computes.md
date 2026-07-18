# wasm gap: genuinely-RUNTIME Symbol/String ordering (<) declines "heap walk not yet built"; rust computes

**Reporter:** breaker (2026-07-18), confirmed by corpus-bugfix on trunk 140834efa. **Severity:** wasm-vs-rust DIVERGENCE (capability gap; not a miscompile). NARROWED to the non-const-foldable operand path.

## Finding
Genuinely-RUNTIME (non-const-foldable) Symbol/String ORDERING (< <= > >=) declines on wasm, computes on rust:
```
(def (mk (: n Int64)) (Symbol.of (if (< n 0) "alpha" "beta")))
(def (run (: a Int64) (: b Int64)) (if (< (mk a) (mk b)) 1 0))   ; run --arg -1 --arg 1
  wasm: "comparison of a compound value needs a heap walk (not yet built)" (compile exit 1)
  rust: computes
```
CONST-foldable Symbol < (e.g. `(< (mk -1) (mk 1))` or the existing 17-symbols const cases) COMPUTE trivially — they fold at compile time and never exercise the runtime heap walk. **The gap only shows with a genuinely-runtime operand (--arg / call boundary).** (corpus-bugfix initially false-greened this by testing a const-foldable shape — the pin must use a runtime operand.)

## Distinct from
- compound EQUALITY (=): works on wasm (ty_heap_walkable / equality heap walk landed).
- LIST ordering: correct uniform decline (no blessed order, v-runtime @329271b89).
- This is compound ORDERING for **Symbol/String specifically**, whose total order the spec **DOES** bless (17-symbols §order) → wasm should grow the runtime compound-ORDERING heap walk (rust already has it), separate from the equality walk.

## Routing
ROUTED to v-runtime (corpus-bugfix 2026-07-18): the wasm compound-compare heap-walk territory (select.rs). Grow the runtime Symbol/String ORDERING walk. Once landed, a runtime-operand (--arg) positive corpus case pins it — breaker will add. Not spawning.

---
ABSORBED + PRIORITIZED (v-runtime, 2026-07-18): confirmed on clean trunk 1453fef7e (wasm hard-errors, rust
computes 0). Distinct from the LIST case — Symbol/String DO have a blessed order (17-symbols §order), so wasm
grows the ordering walk. Folded into v-runtime's operator-GO'd BLESSED HEAP-VALUE ORDERING workstream as the
now-PRIORITIZED SLICE 1 (NEXT, HASH-NEUTRAL): a runtime Symbol/String is a Shape::Str bytes leaf, order =
content-lexicographic, emitted INLINE with exported bytes-get/bytes-len (like String.scalar-len's walk) → a
bytes-lex-compare, NO frozen-hash bump. Resolves this divergence directly. (SLICE 2, after: compound
list/tuple/sum ordering — new runtime value_cmp op + a hash bump, announced via design note.) v-runtime pings
when slice 1 lands so breaker adds the runtime-operand positive corpus pin (passes on both backends then).
No separate fixer — absorbed. Verify+close when slice 1 lands.
