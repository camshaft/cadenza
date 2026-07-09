# Asks index

One line per ask across all three lifecycle directories. See `README.md` for the process.
Regenerate after moving files. Sorted: open (by priority) → pending-validation → done (by ID).

## 🔧 Open (priority-ordered)

- `P021` **ask-81** — 🧭 DESIGN + HANDOFF (rcdzc): CLOSURES as compile-time lambda β-reduction — the SAME `eval` fold tier that will do type-functions/monomorphization (#150). A `(fn …)` is one more TRANSIENT compile-time value (FuncRef/Ctor/Intrinsic family): new IR leaf `Lambda` (Hir/Typed/Mir), one fold rule `Apply(Lambda,args)`→β-reduce, one `select` decline for a survivor. **Increment A (compile-time lambdas)** closes the ENTIRE core `09-functions.sexp` witness (immediate/let-bound/named-HOF/returned/in-place-capture) + **CURRIED application that const-folds back into a regular `Call`** (`((add 3) 4)`→`Call{add,[3,4]}` via fold SPINE-COLLAPSE; the eager under-arity decline at `infer.rs:187` MOVES to the reduction-aware fold) — needs α-renaming on inline/β-reduce (mandatory; hardens the const-inline path too) + `is_transient`/`try_inline` guard widening. **Increment B (runtime closures = `call_indirect`+heap env+`Ty::Fn`→I32)** DEFERRED, declined honestly. Type functions fall out: a type fn is a lambda over type values, always reduces at compile time (pure A subset) — build the β-reduction tier once  
  `open/P021-ask-81-closures-design-compile-time-lambda-beta-reduction-shared-with-type-functions.md`
- `P005` **ask-68** — 🟢 READY TO IMPLEMENT — Perceus precise drop insertion (M2 Phase D / "task #9"), the FULL spelled-out plan. Reframe: leak-freedom (`live-objects → 0`, the actual ask) needs NO liveness analysis — it's mechanical, lands at end of Phase 2 via an Owned/Borrowed context flag threaded through `emit` + scope-end drops. Phase 1 = drop materialized heap locals at fn-end (balances the dup-on-read) + fix `gen_do` `op::DROP`→`himport::DROP` leak. Phase 2 = stop dup'ing in borrow positions (accessors: arr-get/sum-payload/vec-get/map-lookup all return BORROWED handles — the UAF class), dup escaping accessor results. Phases 3 (last-use, the hard part, needs ask-64 UAF trap) + 4 (FBIP reset/reuse — runtime ALREADY built these, WIT 26–28, just not in HEAP_ALLOWLIST) are OPTIONAL optimization. Verified against tree; runtime's own tests prescribe `dup(kept);drop(parent)`  
  `open/P005-ask-68-perceus-precise-drop-insertion-the-full-plan.md`
- `P015` **ask-30** — 🟢 CLOSED (rejection seam exhausted) The self-hosted compiler's CODED effect-diagnostics `compile` keeps the **byte-gate GREEN (component-check 123 agree / 0 disagree — PASS)** and catches every PROVABLE rejection reachable without compound types: arith/cmp/`if`-branch kind mismatch → CDZ0201, non-exhaustive BOOL match → CDZ0210, out-of-range int literal → CDZ0201, dup record/map field → CDZ0201, malformed `let` → CDZ0201, int-vs-float → CDZ0301/CDZ0201, scalar-receiver `.`/`tuple.N` → CDZ0201. The gap-independent reader-REJECTION seam is now EXHAUSTED (analysis in ask-57); every remaining native-rejects/mine-declines is SHAPE-DEPENDENT compound work (needs the seed heap — ask-13/ask-57), all honest `decline`  
  `open/P015-ask-30-the-self-hosted-compiler-has-no-type-checker-compiles-ill-typed-programs-native-rejects.md`
- `P016` **ask-58** — 🟢 DESIGN (operator direction): make ALL built-in modules (`Bytes`/`Int64`/`String`/`List`/`Set`/`Map`/`Char`/… — every one, not just `Bytes`) REAL prelude RECORDS of builtin-function refs, so `Bytes.len` = `(. Bytes len)` const-folded — delete per-module name-specialization from the reader/resolver; realizes the `member-access-and-modules-as-records` decision. Key new primitive = a first-class BUILTIN-FUNCTION REF value (folds from projection, lowers on apply, declines elsewhere). SEED+SPEC first; compiler.cdz's projection-fold (ask-57) is ready, just needs apply-a-builtin-ref lowering  
  `open/P016-ask-58-builtin-modules-as-records-of-builtin-function-refs-const-folded.md`
- `P028` **ask-48** — 🟡 The new `diagnostics.md` capability sets the diagnostics bar well above the seed — error recovery (maximal independent set), primary/derived, and a machine-branchable rejection/decline/trap kind  
  `open/P028-ask-48-diagnostics-capability-sets-bar-above-seed-error-recovery-maximal-set-primary-derived-kind.md`
- `P030` **ask-20** — ⚪ The self-inclusion frontier: what the compiler's emit path must grow to compile its own source  
  `open/P030-ask-20-the-self-inclusion-frontier-what-the-compiler-s-emit-path-mu.md`
- `P050` **ask-02** — 🔴 M-ordering tension: effects are the #1 self-host blocker but scheduled M6  
  `open/P050-ask-02-m-ordering-tension-effects-are-the-1-self-host-blocker-but-s.md`
- `P070` **ask-09** — 🔴 Should a provable-certain trap be a compile-time rejection? (fold stays meaning-preserving either way)  
  `open/P070-ask-09-should-a-provable-certain-trap-be-a-compile-time-rejection-f.md`
- `P100` **ask-39** — 🟡 Runtime AST construction / `Ast.encode` / `Ast.decode` decline — `Ast` is a compile-time-only value, not a registered RUNTIME sum type  
  `open/P100-ask-39-runtime-ast-construction-and-encode-decode-not-supported-ast-not-a-runtime-sum-type.md`
- `P110` **ask-06** — ⚪ Byte-identity target must account for optimization depth (cdz-rustc needs DCE)  
  `open/P110-ask-06-byte-identity-target-must-account-for-optimization-depth-cdz.md`
- `P120` **ask-07** — ⚪ Runtime `String` is the keystone front-end blocker (spec is fine; seed + realized-set work)  
  `open/P120-ask-07-runtime-string-is-the-keystone-front-end-blocker-spec-is-fin.md`
- `P110` **ask-59** — 🟡 (compiler.cdz) Bool-typed PARAMETERS decline — the i64-parameter model can't compile a function whose param is used as a Bool (`if`-cond / bool-`match` / `not`) nor a call passing a Bool arg; native infers per-param valtype. Fix = per-parameter kind inference + Bool-aware calling convention (a Bool param = i32, a Bool arg passes direct). LARGEST remaining compiler.cdz-ownable scalar cluster; NOT seed-gated. SUBSUMES ask-35 (return-kind is the pass-through case). Deferred — a real inference+ABI subsystem, not a loop patch  
  `open/P110-ask-59-bool-parameter-kind-inference-and-calling-convention.md`
- `P120` **ask-35** — ⚪ Polymorphic return-kind specialization (a function whose return kind is its argument's) — the `agree` follow-on to ask-34's decline; SUBSUMED by ask-59 (per-parameter kind inference)  
  `open/P120-ask-35-polymorphic-return-kind-specialization-for-byte-identity-follow-on-to-ask-34.md`
- `P130` **ask-08** — ⚪ No tail-call optimization / bounded wasm stack (self-host ceiling, not a blocker)  
  `open/P130-ask-08-no-tail-call-optimization-bounded-wasm-stack-self-host-ceili.md`
- `P130` **ask-43** — ⚪ Right-shift over-declares one scratch local (3 vs native's 2) — a byte-fidelity gap keeping `>>` `soft` not `agree`  
  `open/P130-ask-43-right-shift-over-declares-a-scratch-local-soft-not-agree.md`
- `P140` **ask-10** — 🟡 A spike's "verified byte-correct" claim must become a corpus case, not stay a probe  
  `open/P140-ask-10-a-spike-s-verified-byte-correct-claim-must-become-a-corpus-c.md`

## ⏳ Pending validation

- **ask-55** — ✅ float crash FIXED (compiler.cdz 19:03) — loop-verified: bare `4.5` → decline (was trap), 0 `run error` traps in the byte gate (was 22), int/float mix now rejects. Awaiting stable refresh + four-gate confirm. Follow-on ask-56 (wrong code on the mix).  
  `pending-validation/ask-55-shape-check-regressed-float-from-decline-to-crash.md`

## ✅ Done

- **ask-13** — 🟢 PARTIAL: the built-in `list` pattern-matching surface — spec clause + STATIC desugar landed (element patterns); the RUNTIME half rides the compound frontier (ask-57)  
  `done/ask-13-the-built-in-list-has-no-pattern-matching-surface-spec-addit.md`
- **ask-01** — 🟢 Pattern binders must compose (nest) — SEED BEHAVIOR NOW LANDED; only the spec MUST remains  
  `done/ask-01-pattern-binders-must-compose-nest-seed-behavior-now-landed-o.md`
- **ask-03** — 🟢 Typed instruction sum for the backend (not string-tagged quasiquote)  
  `done/ask-03-typed-instruction-sum-for-the-backend-not-string-tagged-quas.md`
- **ask-04** — 🟢 Boolean connectives (`and`/`or`/`not`) — the spec had none  
  `done/ask-04-boolean-connectives-and-or-not-the-spec-had-none.md`
- **ask-05** — 🟢 Effect-declaration surface + capability routing at the entrypoint  
  `done/ask-05-effect-declaration-surface-capability-routing-at-the-entrypo.md`
- **ask-11** — 🟢 The front end's unknown-head path needs a real diagnostic, not a placeholder trap — RESOLVED (honest trap) 2026-07-07  
  `done/ask-11-the-front-end-s-unknown-head-path-needs-a-real-diagnostic-no.md`
- **ask-12** — 🟢 The built-in Option/Result loses its payload kind across a function boundary (the reader gate) — RESOLVED 2026-07-07  
  `done/ask-12-the-built-in-option-result-loses-its-payload-kind-across-a-f.md`
- **ask-14** — 🟢 Kind inference is branch-order-dependent for a recursive Bool return — FIXED 2026-07-07  
  `done/ask-14-kind-inference-is-branch-order-dependent-for-a-recursive-boo.md`
- **ask-15** — 🟢 `tuple.N` on a named-def's runtime-tuple result (no `let`) emitted an INVALID component — FIXED 2026-07-07  
  `done/ask-15-tuple-n-on-a-named-def-s-runtime-tuple-result-no-let-emitted.md`
- **ask-16** — 🟢 The real `resolve` on a runtime-built `Node` declined "cannot box" — RESOLVED (was MIS-FRAMED as a seed scale limit) 2026-07-07  
  `done/ask-16-the-real-resolve-on-a-runtime-built-node-declined-cannot-box.md`
- **ask-17** — 🟢 `List.at` on a list bound from a sum payload declines (blocks the natural multi-arg-call rep) — FIXED 2026-07-07  
  `done/ask-17-list-at-on-a-list-bound-from-a-sum-payload-declines-blocks-t.md`
- **ask-18** — 🟢 A recursive `List.push`-accumulator loses its list return kind — FIXED 2026-07-07  
  `done/ask-18-a-recursive-list-push-accumulator-loses-its-list-return-kind.md`
- **ask-19** — 🟢 A nested constructor pattern under `Some` declines when the matched list is a parameter — FIXED (seed) — awaiting loop re-probe  
  `done/ask-19-a-nested-constructor-pattern-under-some-declines-when-the-ma.md`
- **ask-21** — 🟢 Over-applying a user function declines as "needs closures", not the CDZ0201 the corpus says it mirrors — and head-position name classification is fragile  
  `done/ask-21-over-applying-a-user-function-declines-as-needs-closures-not.md`
- **ask-22** — 🟢 Seed gap 3l: emit a `compile : list<u8> → list<u8>` component, not only nullary `run` — RESOLVED (seed side) 2026-07-07  
  `done/ask-22-seed-gap-3l-emit-a-compile-list-u8-list-u8-component-not-onl.md`
- **ask-23** — 🟢 The self-hosted reader miscompiles unsupported constructs instead of declining — RESOLVED (compiler side) 2026-07-07  
  `done/ask-23-the-self-hosted-reader-miscompiles-unsupported-constructs-in.md`
- **ask-24** — 🟢 A monotone fixpoint loop OOMs the seed when a fresh-re-seeded list parameter is consumed as a list — RESOLVED (seed side) 2026-07-07  
  `done/ask-24-a-monotone-fixpoint-loop-ooms-the-seed-when-a-fresh-re-seede.md`
- **ask-25** — 🟢 The self-hosted compiler selects the module entry by POSITION (first def), not by the name `main` — LANDED (gap 3m fixed) — awaiting loop re-probe  
  `done/ask-25-the-self-hosted-compiler-selects-the-module-entry-by-positio.md`
- **ask-26** — 🟢 🟠 The differential gate needs a trap-CAUSE discriminator — a decline and a semantic trap are indistinguishable by value alone (measurement gap, not a compiler bug)  
  `done/ask-26-the-differential-gate-needs-a-trap-cause-discriminator-a-dec.md`
- **ask-27** — 🟢 Seed gap 3n: the `compile`-component RETURN trips "return pointer not aligned" — RESOLVED (seed side) 2026-07-07  
  `done/ask-27-seed-gap-3n-the-compile-component-return-trips-return-pointe.md`
- **ask-28** — 🟢 Adopt `component-check` as the byte-level self-hosting gate — WIRING DONE (`--emit-component` landed); gate now RUNS, but its `disagree` count needs a decline discriminator (→ #29)  
  `done/ask-28-adopt-component-check-as-the-byte-level-self-hosting-gate-wi.md`
- **ask-29** — 🟢 `component-check` scores an honest DECLINE as a DISAGREE — DONE (decline discriminator landed) 2026-07-07  
  `done/ask-29-component-check-scores-an-honest-decline-as-a-disagree-the-b.md`
- **ask-31** — 🟢 The language needs non-trapping `checked` arithmetic — LANDED (seed) — awaiting loop re-probe  
  `done/ask-31-language-needs-non-trapping-checked-arithmetic-returning-option.md`
- **ask-32** — 🟢 `Option.expect` declines on a RUNTIME Option (const-only) — while `match` on the same runtime Option works  
  `done/ask-32-option-expect-declines-on-a-runtime-option-const-only.md`
- **ask-33** — 🟢 🟠 `component-check`'s decline discriminator is too narrow — it models "decline = bare `unreachable` entry", but a decline is "traps at runtime" (77 hidden declines still counted as disagree)  
  `done/ask-33-component-check-decline-discriminator-too-narrow-misses-77-hidden-declines.md`
- **ask-34** — 🟢 ✅→⏳ `compiler.cdz` MISCOMPILES a polymorphic identity applied to a Bool — FIXED via fix (2) DECLINE — awaiting loop re-probe  
  `done/ask-34-polymorphic-identity-loses-its-bool-return-a-real-wrong-value-miscompile.md`
- **ask-36** — 🟢 `compiler.cdz` emitted an INVALID component for a `let`-bound Bool — now DECLINES (compiler side) — awaiting loop re-probe  
  `done/ask-36-let-bound-bool-emitted-invalid-now-declines.md`
- **ask-37** — 🟢 `compiler.cdz` emits bare `i64.add/sub/mul` — runtime `+ - *` WRAP silently on overflow instead of trapping (a wrong-value miscompile class)  
  `done/ask-37-runtime-add-sub-mul-wrap-silently-on-overflow-instead-of-trapping-miscompile.md`
- **ask-38** — 🟢 `Ast.decode` must be TOTAL (return a Result/Option), not trap — input can come from an external source; and it must reject trailing bytes into the error case  
  `done/ask-38-ast-decode-silently-ignores-trailing-bytes-violating-the-new-inverting-decode-contract.md`
- **ask-40** — 🟢 `compile` has no diagnostics channel — a rejected program TRAPS instead of returning a coded diagnostic (blocks ~30 ask-30 rejections from `decline → agree`)  
  `done/ask-40-compile-has-no-diagnostics-channel-rejections-trap-instead-of-returning-a-coded-diagnostic.md`
- **ask-41** — 🟢 Realize the kinded-artifact build-tool interface (Amendment 0.8.0) — `compile: list<artifact> → {artifacts, diagnostics}`  
  `done/ask-41-realize-the-kinded-artifact-build-tool-interface-amendment-0-8-0.md`
- **ask-42** — 🟢 A `Result`-returning `compile` declines on the DEEP sum-matches in its call graph — blocking the diagnostics-channel wiring (ask-40's payoff)  
  `done/ask-42-result-returning-compile-declines-deep-sum-match-in-its-call-graph-blocking-diagnostics-wiring.md`
- **ask-44** — 🟢 Stray `DBG` `eprintln!` left in the seed's ctor-arm-match codegen (debug noise on the self-hosting path)  
  `done/ask-44-stray-dbg-eprintln-in-seed-ctor-arm-match-codegen.md`
- **ask-45** — 🟢 ✅ FIXED (seed, re-probed 2026-07-07) — a recursive effectful function on the runtime-compound path now lowers  
  `done/ask-45-recursive-effectful-function-on-runtime-compound-path-now-fixed-effects-in-the-compiler.md`
- **ask-46** — 🟢 ✅ FIXED (seed 15:59, re-probed 2026-07-07) — a recursive effectful `handle` under the `compile` ENTRY now lowers  
  `done/ask-46-recursive-effectful-function-under-the-compile-entry-declines.md`
- **ask-47** — 🟢 🟠 A stray `DBG` eprintln fires on EVERY `compile-run` — output pollution from the ask-41 artifact-detection WIP  
  `done/ask-47-stray-dbg-eprintln-on-every-compile-run-pollutes-output.md`
- **ask-49** — 🟢 ✅ FIXED (seed 16:31, re-probed 2026-07-07) — a recursive-effectful `handle` returning a compound now lowers on the run/emit path  
  `done/ask-49-recursive-effectful-handle-returning-a-compound-declines-on-the-run-emit-path.md`
- **ask-50** — 🟢 Add OPTIONAL `tracing` to the Rust seed compiler for compilation decisions — feature-gated OFF so the wasm build is untouched  
  `done/ask-50-optional-tracing-in-the-rust-seed-for-compilation-decisions.md`
- **ask-51** — 🟢 The `compile-output` ABI detection doesn't look through a `handle` — blocks EFFECT-based diagnostics (the operator's direction)  
  `done/ask-51-compile-output-abi-detection-does-not-see-through-a-handle.md`
- **ask-52** — 🟢 `Option.expect` doesn't carry the unwrapped record's Shape to a field projection — the per-binding-form tail of the runtime-record-field-access landing  
  `done/ask-52-option-expect-does-not-carry-the-record-shape-to-a-field-projection.md`
- **ask-53** — 🟢 (compiler.cdz — MINE) The diagnostics `check` pass conflated declines with rejections — RESOLVED 2026-07-07 via a THREE-VALUED conservative check kind (`CKind = CKi64|CKBool|CKUnk`; `ck-of` proves a kind only from the node, param/call/KError = CKUnk; emit only on a PROVABLE mismatch). The effect-diagnostics `compile` is now the SHIPPED gate-safe entry: 0 false-rejects (was 9), 79→95 agree, value-gate 0 hard/0 error  
  `done/ask-53-compiler-check-pass-conflates-declines-with-type-rejections.md`
- **ask-54** — 🟢 (compiler.cdz — MINE) No float-literal representation blocked int-vs-float type rejection — RESOLVED 2026-07-07: added `NFloat`→`Core.KFloat` (check-only; `lower` declines it) + `CKFloat` in the lattice; wired through all 12 Core matches. Int-vs-float now rejects with native's code. **Took the byte-gate GREEN (component-check 120 agree / 0 disagree — PASS)**  
  `done/ask-54-no-float-literal-representation-blocks-int-vs-float-type-rejection.md`
- **ask-55** — 🟢 (compiler.cdz — MINE) A transient mid-edit float crash (decline→trap) — RESOLVED by completing the `KFloat` wiring across all exhaustive Core matches (`lower` emits `unreachable` decline); 0 crashes, byte-gate GREEN  
  `done/ask-55-shape-check-regressed-float-from-decline-to-crash.md`
- **ask-56** — 🟢 (compiler.cdz — MINE) Int/float mix rejected with wrong code (CDZ0201 not CDZ0301) — RESOLVED 2026-07-07: `numeric-mismatch-code` (float→301) at every numeric position AND the ROOT fix — `code-string` silently collapsed every non-210 code to CDZ0201 (added the 301 case). The 14 CDZ0301 cases → agree  
  `done/ask-56-int-float-mix-rejects-with-wrong-code-cdz0201-not-cdz0301.md`
