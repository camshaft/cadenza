# Cadenza-backend wasm-parity checklist (`--target cadenza` round-trip)

**Goal (operator ruling, 2026-09-01):** FULL wasm parity — the cadenza backend must re-emit EVERY
form the wasm backend emits, and the emitted surface must recompile. Close ALL real gaps, in ANY
order, until the forms-diff is empty (excluding genuine true-parity / both-decline forms). This file
is the DURABLE tracked checklist so the list is not forgotten across ticks/compaction.

## How this list was produced (re-runnable)
Every `spec/semantics/*.sexp` corpus program is run through BOTH `cdz compile -t wasm` and
`cdz compile -t cadenza`. A case where **wasm OK + cadenza declines = a real PARITY GAP** (this
backend's work). A case where **both decline = SHARED** (a language/lowering limit, not a
cadenza-backend gap — do NOT chase). Scan scripts: `/tmp/parity_scan.sh` (declines) and
`/tmp/parity_xref.sh` (GAP-vs-SHARED classification). Re-run when closing a category to re-measure.

## Tally (2026-09-01 baseline scan): **968 GAP-cases** vs 1919 SHARED.

## Parity-gap categories (ranked by case count) — the checklist

- [~] **HostCall / effects — ~203** (biggest). PARTIALLY CLOSED #7268 (1d97ceeae3): the simple
  host-delegated perform now round-trips — per-node `((. E op) args)` + `(effect E (op o (-> ..)))` decl
  preamble + a `(host (E..) body)` wrapper on each performing def; perform ⇔ decl coupling declines when the
  decl is not re-emittable. Verified 43 host cases BYTE-IDENTICAL + recompile, 0 breaks. STILL DECLINING
  (later slices, safe CDZ0900): (a) an effect op whose ARROW TYPE has a non-copyable payload (e.g. a
  Qty-return `(-> Unit (Qty Int64 ((. Unit base) #"meter")))` — `emit_type_surface` can't copy the `#"meter"`
  bytes-unit; extend the type-surface copier or use `type_ast`); (b) multi-effect / handled / peer-bound
  (`effect_bindings`) shapes. Re-scan to measure the program-wide HostCall-decline drop.
  ⚠️ FINDING (2026-09-01): the Qty-return-op sub-case (a) is COUPLED to the Qty-value gap — making
  `emit_type_surface` copy the bytes-unit leaf DOES emit the decl+perform, but the perform's `(Qty.value
  <hostcall>)` context then fails to recompile (CDZ0900 "quantity value from construction site") = a
  round-trip BREAK, WORSE than the clean decline. Tried + REVERTED. So a Qty-return effect must wait for the
  Qty-value category; do NOT enable its decl via a leaf-copy alone.
- [ ] **fn-typed `(-> ..)` parameters — ~170** (higher-order). An INTERNAL fn param works (lam2);
  a specific higher-order shape declines while wasm compiles it. Investigate the declining shape
  (likely exported/boundary or a specific fn-type position).
- [ ] **quantity VALUE from certain construction sites — 106.** A simple `Qty.of` round-trips, but
  specific construction sites decline (`re-emitting a quantity value from this construction site`).
  Coordinate value-facts / units owner.
- [ ] **binary-matching reads — ~106** (`BinIntRead` 98 / `BinRestRead` 6 / `BinSizedRead` 2). Bit-pattern
  matching not re-emitted. Coordinate the binary-matching owner.
- [ ] **nested-match sub-pattern with a sum/list-rest step — 67** (MY lane). The SumPayload
  nested-projection fallback declines a refutable sum/list-rest step under a list/tuple pattern
  (breaker's family; the codeless-ness is fixed CDZ0900 #7189, the FACE still declines). Needs
  nested-match reconstruction inside `emit_match_list` / the projection walker.
- [ ] **disc-FOLDED / Leaf-ROOT sum-match root — 65** (MY lane). A single-arm irrefutable
  destructure `(match p ((Mk #tuple(a b)) body))` (root cont = Leaf) hits the `_ => decline` in
  emit_match_sum. Needs Leaf-root routing through emit_switch_tree with a full destructure (the
  early-session reverted area — do it carefully with body-read-driven slot binding).
- [x] **closure-capture resolution — 42.** CLOSED #7257 (6e65587241): hoist an un-resolvable value-capture
  into a `(let ((cN <cap-value>)) (fn ..))`. Re-scan CONFIRMED: captured-var declines 42 → **0**.
- [ ] **newtype VALUE from certain construction sites — 32** (`re-emitting a newtype value from this
  construction site`). Investigate the declining construction site.
- [x] **AST metaprogramming nodes — ~26** (`AstEncode`/`AstPrint`/`AstDecode` over RUNTIME Ast). CLOSED
  #7259 (5635966d4b): three member-access emit arms `((. Ast encode/print/decode) operand)`; the
  `discs`/`disc_ok`/`disc_err` re-derive from the operand's solved type on recompile. Case (b) confirmed
  (runtime Ast, not a fold fix). Co-owned w/ v-metaprogramming (they own the codec). Metaprog-file declines 13 → 0.
- [ ] **`Core::Seq` — 41.** Entangled with effects (Seq ⟺ observable side-effects ⟺ HostCall); likely
  RESOLVED as a side effect of the HostCall build (emit `(do stmts… tail)`). Re-measure after HostCall.
- [ ] **`ConstFloatNan` 30 + `ConstFloatInf` 8.** An un-folded INTERNAL NaN/Inf node declines; emit the
  `Float64.nan` / inf surface. VERIFY not shared (a NaN/Inf VALUE crossing the boundary is shared —
  "no written value form"). Only the internal-node case is a gap.
- [ ] **`ConstBytes` — 14.** A bytes constant value not re-emitted (`b"…"` literal). Emit the bytes literal.
- [x] **Str/Char literal-at-slot — (LitTest).** CLOSED #7242 (49e1d2a1a0). (Bytes/ListLen/MapHasKeys slot
  probes still decline, coded CDZ0900.)

## VERIFY (counted as GAP but likely true-parity / mis-counted) — confirm before chasing
- [ ] **under-determined sum type — 13.** Likely SHARED (wasm needs concrete types too); confirm direct also declines.
- [ ] **`Poison` 28 / `TrapDivZero` 7 / `TrapOverflow` 5 / other Trap — ~40.** A TRAP may BE the case's expected
  outcome → true-parity, not a gap. Verify against each case's expected outcome; reclassify as SHARED if so.

## Already landed toward parity (compound-slot family + declines-coded, pre-full-parity-ruling)
variant-at-slot #7074 · literal-at-slot + empty-sum #7153 · newtype-peel (map-key) #7181 · guarded-slot
#7188 · projection declines coded CDZ0900 #7189 · newtype-list-element peel #7219 · Str/Char-at-slot #7242 ·
closure-capture hoist #7257.

## Coordination (peer lanes — do NOT solo-reinvent)
effects/HostCall → v-effects (recipe received) · binary-matching → owner · AST nodes → v-metaprogramming ·
quantity → value-facts/units.
