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

## 🚨 VALIDATION INTEGRITY (2026-09-01) — the two gates every close MUST pass, and the trap
A cadenza close needs BOTH: (1) the case no longer declines, AND (2) the emitted surface RECOMPILES
(hop2: `-t cadenza` to a `.ast`, then `-t wasm` that `.ast`) AND is value-equivalent. 🪤 TRAP: detect
emit-success by the OUTPUT FILE (`-o X; [ -s X ]`), NEVER by `compile 2>/dev/null | grep -q wrote` — cdz
prints "wrote" to STDERR, so that grep never matches and the hop2 check silently no-ops (a HOLLOW gate that
reports 0 breaks while checking nothing; it hid breaker's #7298 catch). Also hand-probe a RUNTIME-vs-const
survivor for any value-emitting arm (a corpus sweep is blind to value arms whose const folds away). ALSO: a
TRUE round-trip break requires the ORIGINAL to compile to wasm — `original-wasm-OK AND hop2-FAIL`; an original
that itself fails wasm is SHARED, not a cadenza gap (the gate MUST pre-check original-wasm, else it over-counts).
The first corrected sweep reported ~65 but ~half were SHARED (bare-effect host ops returning String/etc — original
also CDZ0900). With the original-wasm precondition: **~22 TRUE round-trip breaks** on clean main — cluster: the
UInt64-literal ascription-drop dominates (10 in 06-numeric-model; the ConstInt arm emits a BARE literal, which
re-grounds to Int64 → CDZ0201 for unsigned/over-i64) · nested/generic user sums (05/07) · a NEWTYPE-unwrap match
`(match u ((Mk n) n))` folding to bare `u` → returns `(: 7 UserId)` not `7` (a VALUE MISCOMPILE in the nominal
fold) · Map-runtime-keys · empty-list ascription `(: #list() (List Int64))`. ✅ RE-VALIDATED #7278 (Leaf-root) +
#7303 (Seq) with the corrected+precondition gate: BOTH HOLD (Leaf-root emissions recompile + value-match; the
effect breaks were all SHARED). ✅ #7346 CLOSED the UInt64-literal cluster (ascribe `(: v <IntTy>)` for
unsigned/over-i64 via `int_module_ast`; 06-numeric true breaks 10→1). REMAINING true breaks: **12** (titled-break scan, corrected+precondition gate, post-#7346; all
surface/recompilability TYPE breaks — none wrong-DATA), by family:
  • GENERIC/NESTED-SUM (CDZ0203 ×5): 05 depth-3 erased-and-boxed nested-sum match (breaker-reproduced) · 07
    annotated-empty-list (undetermined-empty-list control) · 07 gng1 nested generic Box-of-Pair · 09
    recursive-generic producer wrapping in a user sum consumed at one type · 09 borrowed-heap-sum-param in a
    self-recursive fn. (Same root as the under-determined-sum type_ast/generic-sum surface family.)
  • MAP runtime-key (CDZ0201 ×2): 05 two distinct names → same value key · same string key. [breaker minimizing]
  • MUTUAL-RECURSION / SCC (CDZ0101 ×2): 14b mutually-recursive performing pair threading one handler state · 14c
    caller-observed pure-mutual SCC group-wide multi-value fold (a fn in the SCC not re-emitted → unbound name).
  • do-local recursive-fn double-inlining (CDZ0201 ×1, 02) · newtype-from-PERFORM @invariant (CDZ0201 ×1, 14b).
  • UInt64-FROM-CONTEXT (CDZ0301 ×1, 06): `(& x <hugelit>)` — the literal's own type is imprecisely Int64; needs
    the op to thread `expected`=UInt64 to operands AND the ConstInt arm to prefer expected when eff_ty can't hold v
    (MINE, deferred — touches Arith operand emit + the expected-consultation path). Also: newtype-unwrap type-drop
    `(match u ((Mk n) n))`→bare `u` + Int8 narrow-signed type-drop (tuple Int8→Int64) — the "emit drops the precise
    type" family (may overlap the generic-sum ascription work). NO Rational break in the true set (Rational is SHARED/OK). 🪤 value_ab HARNESS CAVEAT: it passes FIXED args regardless of `main` arity, so a
MULTI-arg main gets a malformed invocation → FALSE-POSITIVE mismatch (bit me: gcd "3041 vs 3040" was fake; gcd
MATCHES with correct 2 args). Confirm any value_ab mismatch with arity-correct args before believing it.

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
(ConstFloatNan non-Float64 residual CLOSED by #7309; type-value Poison CLOSED #7330.) So the generic-node bucket is
now ONLY binary-matching 104 (BinIntRead/Rest/Sized — owner-coordinate). EVERY other generic-node GAP is CLOSED.

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
  So it's a TYPE-SURFACE-RENDERING gap in the `(: value Type)` ascription path, not a value-shape gap.
  🔬 DIAGNOSED (2026-09-01): the 12 are variant values with an un-pinned type PARAMETER (a free type ARG) — e.g.
  `(Ok 6)` is `Result Int64 <?E>` (only Ok exercised, Err's param free). An escaping under-determined value is
  CDZ0203-rejected by BOTH backends (SHARED); the GAP cases are consumed INTERNALLY (`=`/`match`).
  ❌ ATTEMPTED + ABANDONED (unsound): defaulting the free arg to `Unit` for the ascription. The corrected hop2 gate
  caught it — the free arg is NOT always unobserved: NON-LOCAL context the node can't see can fix it (a `(None)` fed
  to a param `acc: Option Int64` → my `(Option Unit)` emit → hop2 CDZ0203 payload mismatch); the node-local
  `expected`-fallback doesn't capture it. ⏭️ Needs REAL propagation of the surrounding-context type into the SumNew
  ascription (not node-local defaulting) — a harder slice; STILL OPEN.
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
- [x] **`Poison` type-value 3 — CLOSED #7330 (d3b70e9b6d).** v-metaprogramming (reflection owner) confirmed the recipe:
  the Poison IS the expected erased core of a compile-time type-value (do NOT de-Poison it). A pre-core-match `Ty::Type`
  arm recovers the concrete Ty via `eval::typeval_of` and re-emits `(: <type_ast(concrete)> Type)` (mirrors
  value_form.rs:1999). SAFE discriminator (Ty::Type AND typeval_of success) — a real Poison rejection like
  `(const (+ k 1))` has a non-Type type → arm skipped → still declines CDZ0900 (verified, no misfire). All 3 round-trip
  value-equivalent; full-corpus hop2 = 0 breaks. The other 27 Poison programs stay SHARED (wasm also rejects).
- [~] ~~**`Poison` 3** — VERIFIED + ROUTED~~ (superseded by ↑ CLOSED #7330). Full Poison scan: 30 Poison-declining programs = 27
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
