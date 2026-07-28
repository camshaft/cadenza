# TRACKING (corpus-bugfix, 2026-07-27) — multi-field sum payload (2-elem) COMPILED traps wasm-unreachable

Origin: v-compiler-ml FYI/queue-tracking (inbox issue 000000017099, ref 3cd92ae34).
Repro filed at queue/mlrepro-multifield-sum-payload-2elem-list-compiled-traps-wasm-unreachable.md.

**Status: ALREADY OWNED** — v-compiler-ml filed + routed to v-rust-backend / v-wasm-opt for the
compiled backtrace (they can't local-repro: reduction limit). NOT re-routing (avoid double-assign).

**Symptom**: a 2-field ctor `(P Int64 Int64)` matched `((P x y) (+ x y))` TRAPS wasm-unreachable
when COMPILED (interpreter OK). Narrowed by v-compiler-ml: isolated 2-elem List + SumStore-cell
reads both compile GREEN; only the full construct→store→match→bind-both integration traps.

**Reference oracle (for the corpus pin when fixed)**:
  - `(P 3 4)` → `((P x y) (+ x y))` → **7** COMPILED.
  - runtime-boxed twin: `(if (> n 0) (P 3 4) (P 10 20))` matched → **7** (n>0).

**corpus-bugfix action ON FIX**: when v-rust-backend/v-wasm-opt land the fix, gate the graded case
x3 → 7 (compiled + runtime-boxed twin) and pin into 05-compound-types.sexp beside the other
multi-payload ctor pins. Watch for their land ping / trunk advance touching sum-payload emit.

---
**UPDATE 2026-07-27 (v-compiler-ml + v-wasm-opt joint localization)**:
- v-wasm-opt REFRAMED: it is a **VALUE MISCOMPILE, not a crash**. func 60 (backtrace frame) is
  the TEST WRAPPER; its tail is `if run-src(...) == 7 then unit else unreachable`. Compiled
  run-src RUNS TO COMPLETION and returns Some(v != 7); the assert traps. Single-frame backtrace
  (no unreachable in any eval-db callee).
- v-wasm-opt ruled out the simple "dropped 2nd field" hypothesis: **v != 3** (probed `if v==3`
  compiled → still fails). Wrong value is 0 / 4 / 34 / garbage — UNDETERMINED.
- v-compiler-ml NEW DATA: the direct-Core hand-built test `ev-cctor-two-field-payload-binds-both`
  (eval-db.cdz:779) builds the SAME round-trip — CCtor(0,[3,4]) → CMatchSum(scrut,0,[100,101],
  CBin(+,CVar100,CVar101),CNum0) → a+b — and **PASSES COMPILED locally (=7)**. So the raw SumStore
  2-field store-alloc/store-payload/bind-payload EMIT is CORRECT compiled. The bug is therefore
  **NOT the data-path emit** — it is SCALE/SLOT-dependent (width-disjoint-slot FINDING family,
  separate site from br_table `4f9658803`) OR specific to the **lower-db-PRODUCED Core shape**
  for the run-src source at self-host scale (which may differ from the hand-built Core).
- Value-capture CANNOT happen in v-compiler-ml's tree: instrumenting eval-db + run-src there runs
  the INTERPRETER (=7 correct); the miscompile lives only in rcdzc emit, and the compiled run-src
  path can't run locally (CDZ0999 db-lower/db-infer). Capture belongs to v-wasm-opt's tree.
- Suggested localization: v-wasm-opt read the inner run-src main export return directly off the
  dumped component (zero recompile), OR diff the lower-db Core shape for the run-src source vs the
  hand-built direct-Core test. v-wasm-opt still drives the emit fix; v-compiler-ml HOLDS
  f91d6a116+455ada256 and co-verifies → 7 on land.
