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
## Re-measured (2026-09-01, post #7268/#7257/#7259/#7278): **340 GAP** / 1543 SHARED (1883 total declines,
down from 2887 — ~1000 more cases now compile to cadenza as the landed slices closed). Top remaining GAPs:
generic `Core node … : NODE` 72 · nested-match non-tuple sub-pattern 49 · fn-typed param `(-> ..)` 46+20+… ·
newtype-value-from-construction-site 28 · payload-projection-over-non-tuple/record 22 · literal-payload-test
non-scalar 16 · deep-match const destructure 15 · under-determined sum 12 · Qty-value-from-construction-site 11.
### generic-node bucket IDENTIFIED (node_ident scan, un-normalized): BinIntRead 96 / BinRestRead 6 / BinSizedRead 2
(binary-matching — coordinate owner) · ConstFloatNan 30 · ConstBytes 16 · Seq 15 · ConstFloatInf 8 · TrapDivZero 7
· TrapOverflow 5 · Poison 3. ✅ #7298 CLOSED ConstBytes + ConstFloatNan(F64) + ConstFloatInf(F64) = ~54; ✅ #7303
CLOSED Seq = 15. Left in this bucket (re-measured post-#7303): BinIntRead 96 / BinRestRead 6 / BinSizedRead 2 =
binary-matching 104 (coordinate owner) · ✅ TrapDivZero 7 CLOSED #7313 · ✅ TrapOverflow 5 CLOSED #7319 (both
kind-preserving) · Poison 3 VERIFIED → 27 SHARED + 3 type-value GAPs ROUTED to v-metaprogramming (reflection).
(ConstFloatNan non-Float64 residual CLOSED by #7309.) So the generic-node bucket is now ONLY binary-matching 104
(owner-coordinate) + the 3 type-value Poison GAPs (v-metaprogramming's reflection lane) — every solo my-lane
generic node is CLOSED or ROUTED.

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
- [~] **disc-FOLDED / Leaf-ROOT sum-match root — 65** (MY lane). PARTIALLY CLOSED #7278 (8eba1dfc2d):
  a Leaf-root MatchSum whose body reads the scrutinee by position (`SumPayload{s,[Elem(i)]}`, no `Payload`
  step) now reconstructs the ONE irrefutable arm — `#tuple(b…)` for a `Ty::Tuple` scrutinee, `(Ctor b…)` for
  a single-variant Sum/Nominal (erased newtype) — binding slot i at `(s,[Elem(i)])` so body reads resolve off
  the once-evaluated scrutinee (no double-eval). VALIDATED: value A/B on 05-compound-types = 0 mismatch;
  full-corpus hop2 recompile = 0 breaks. STILL DECLINING (`sum match (a disc-folded / nested / …)` ~8): the
  MULTI-variant-Sum root reads `[Elem(i)]` at a PROVEN variant — needs the folded variant RECORDED (re-engage
  v-core-opt) + a bind at `[Elem(i)]` not `[Payload,Elem]`. Next slice.
- [x] **closure-capture resolution — 42.** CLOSED #7257 (6e65587241): hoist an un-resolvable value-capture
  into a `(let ((cN <cap-value>)) (fn ..))`. Re-scan CONFIRMED: captured-var declines 42 → **0**.
- [ ] **newtype VALUE from certain construction sites — 32** (`re-emitting a newtype value from this
  construction site`). Investigate the declining construction site.
- [x] **AST metaprogramming nodes — ~26** (`AstEncode`/`AstPrint`/`AstDecode` over RUNTIME Ast). CLOSED
  #7259 (5635966d4b): three member-access emit arms `((. Ast encode/print/decode) operand)`; the
  `discs`/`disc_ok`/`disc_err` re-derive from the operand's solved type on recompile. Case (b) confirmed
  (runtime Ast, not a fold fix). Co-owned w/ v-metaprogramming (they own the codec). Metaprog-file declines 13 → 0.
  ✅ v-metaprog CONFIRMED SOUND (2026-09-01): discs are TYPE-derived (Ast sum by name) not operand-derived, so
  never carry them on the surface; empirically byte-clean on corpus-cadenza-12 + -30. ONE exotic caveat (NOT a
  blocker): a USER-SHADOWED `(type Ast …)` + encode of that value + the user decl NOT re-emitted → recompile
  would derive the BUILT-IN discs (mismatch). Safe here — `emit` re-emits ALL user type decls, so a shadowed
  Ast decl rides along and discs re-derive consistently; the dangerous combo (decl dropped) is not reachable.
- [x] **`Core::Seq` — 15** (node_ident count; the 41 was the pre-HostCall estimate). CLOSED #7303 (d516929b77):
  re-emit `(do <stmt>… <tail>)`. A Seq is built ONLY when a non-final stmt reaches a host call (compute.rs
  §needs_seq) and surface `do` re-lowers to a Seq under the SAME condition, so the re-emitted stmts re-form an
  equivalent Seq (round-trips); composes with the #7268 `(host …)` wrapper + perform⇔decl coupling. Seq declines
  15 → 0; full-corpus hop2 = 0 breaks. Additive-by-construction.
- [x] **`ConstFloatNan` 30 + `ConstFloatInf` 8.** CLOSED #7298 then FIXED #7309 (53a33b3fc5) — FULLY closed,
  all widths. ⚠️ #7298 first emitted the bare leaves `Leaf::FloatNan`/`Leaf::FloatInf`, which was WRONG: those
  are VALUE-RENDER / `Ast.encode` leaves the FRONT-END POISONS in expression position (`resolve.rs` → CDZ0201
  "non-finite float value has no source literal form"). It broke recompile whenever a non-finite const survives
  to a runtime VALUE position (a runtime-vs-const compare that doesn't fold) — breaker adv-hop2 finding; my
  full-corpus hop2 scan MISSED it because the corpus only has ALL-CONST non-finite compares that fold away
  (🪤 CORPUS-COVERAGE BLIND SPOT: a passing full-corpus hop2 does NOT prove a value-emitting arm round-trips
  unless a case keeps the value LIVE past const-folding — a runtime-vs-const shape). #7309 fix: emit the WRITTEN
  value form `(. Float<width> nan)` / `(. Float<width> Infinity)` (prelude.rs §float module constant fields:
  a `float-nan`/`float-inf` intrinsic annotated with the module width), which fold back to Core::ConstFloatNan/
  Inf on recompile. Per-width module name handles EVERY width — subsumes the old ==64 guard, closing the 5
  residual non-Float64 NaN cases. Validated: breaker repro HOP-2 ok + value 103; Float32 runtime-vs-const ok +
  value 11; full-corpus hop2 = 0 breaks.
- [x] **`ConstBytes` — 14.** CLOSED #7298 (2fcda9acc3): re-emit `Leaf::Bytes` (`b"…"`; shared `Arc<[u8]>`, no
  copy), the twin of the `ConstStr`↔`"…"` path. Value A/B + full-corpus hop2 clean.
- [x] **Str/Char literal-at-slot — (LitTest).** CLOSED #7242 (49e1d2a1a0). (Bytes/ListLen/MapHasKeys slot
  probes still decline, coded CDZ0900.)

## VERIFY (counted as GAP but likely true-parity / mis-counted) — confirm before chasing
- [ ] **under-determined / generic-open sum type — 12 (+2 generic-open) = ~14.** ⚠️ CORRECTED: NOT shared — the xref
  data (reliable exit-code classification) has all 12 as **GAP** (wasm compiles them, cadenza declines); the "likely
  SHARED" guess was WRONG. ROOT CAUSE (read `backend/cadenza/mod.rs:1504` + `lower.rs:1576` type_ast): a `SumNew`
  value re-emits `(: (<V> <payload>) <sum-type>)`, and the ascription's `<sum-type>` is built by
  `crate::lower::type_ast(&ty)`; it returns `None` (→ decline) when the sum type can't be rendered — chiefly
  `ncx.name_of(decl) == None` (an UNNAMED / anonymous / not-registered sum decl) or a nested unrenderable type (a
  type `Var`, a fn type, `Type`). Same family as the "generic / open user sum value" decline (mod.rs:803, 2 cases).
  So it's a TYPE-SURFACE-RENDERING gap in the `(: value Type)` ascription path, not a value-shape gap. ⏭️ NEEDS
  case-level diagnosis at a LOW-LOAD tick: run the 12 cases to see WHICH sum types hit `name_of==None` — if a named
  sum just isn't in `ncx` it's a fixable registration/lookup; if genuinely ANONYMOUS (no surface name) it may be an
  un-writable surface = reclassify SHARED. Likely overlaps the type-surface renderer (render_ty / type_ast) lane.
- [x] **`TrapDivZero` 7.** NOT true-parity — a real GAP, now CLOSED #7313 (c438bc88a0). These are const `(/ x 0)`/
  `(% x 0)` demoted in a conditionally-reached branch (`demote_conditional_trap`); wasm compiles + traps "integer
  divide by zero", cadenza previously declined. Re-emit the kind-preserving source form `(: (/ 1 0) <IntTy>)` — it
  re-demotes to Core::TrapDivZero of the SAME kind + width (operator's 2026-08-27 kind-preservation ruling). A
  generic `(trap "")` traps the WRONG kind ("unreachable") → since a decline is SKIPPED but a mismatched trap FAILS
  the gate, that would be worse than declining. Validated: trap-kind A/B on runtime-vs-const survivors matches;
  targeted hop2 = 0 breaks. BigInt-typed div-zero declines (later).
- [x] **`TrapOverflow` 5.** CLOSED #7319 (05c6912b62) — the overflow twin of #7313. Re-emit `(: (/ <MIN> -1) <IntTy>)`:
  MIN ÷ -1 overflows at every SIGNED width, re-demoting to Core::TrapOverflow of the same kind. KIND-preserving, not
  byte-preserving (a source `(* MAX MAX)` re-emits as `(/ MIN -1)` — different source, same trap kind = correct). Guarded
  to signed ≤64-bit (unsigned / wider defer — unsigned overflow needs a different const shape). Validated: trap-kind A/B
  on runtime-vs-const survivors both trap "integer overflow"; targeted hop2 = 0 breaks.
- [~] **`Poison` 3** — VERIFIED + ROUTED (not a solo close). Full Poison scan: 30 Poison-declining programs = 27
  SHARED (wasm ALSO rejects, e.g. `(const (+ k 1))` → CDZ0201 not-a-const — reclassify SHARED, NOT a gap) + 3 true
  GAPs, ALL TYPE-VALUE programs in 07-type-system: `(let ((t Int64)) t)`, `(let ((t String)) (let ((u t)) u))`,
  `(: Int64 Type)`. wasm compiles+runs these, printing the reified type value `(: Int64 Type)`; the cadenza backend
  gets a `Core::Poison`. This is the TYPE-REFLECTION layer (v-metaprogramming owns Type.ast/-generic). A wasm-accepted
  program carrying a Core::Poison smells like an UPSTREAM lowering gap, not something to paper over in my backend — a
  blanket Poison→type-value emit would misfire on genuine rejections. ROUTED to v-metaprogramming (issue sent): (a) should
  a type-as-value be a Poison at all? (b) if so, is the reified type recoverable for a narrow `Poison-at-Ty::Type` emit
  arm `(: T Type)`? Awaiting their call before any cadenza-side emit.

## Already landed toward parity (compound-slot family + declines-coded, pre-full-parity-ruling)
variant-at-slot #7074 · literal-at-slot + empty-sum #7153 · newtype-peel (map-key) #7181 · guarded-slot
#7188 · projection declines coded CDZ0900 #7189 · newtype-list-element peel #7219 · Str/Char-at-slot #7242 ·
closure-capture hoist #7257.

## Coordination (peer lanes — do NOT solo-reinvent)
effects/HostCall → v-effects (recipe received) · binary-matching → owner · AST nodes → v-metaprogramming ·
quantity → value-facts/units.
