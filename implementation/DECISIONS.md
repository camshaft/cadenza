# Implementation Decisions

Durable record of the high-level answers the agent collected during `/build`, so
a later run does not re-ask. This directory is gitignored: the generated code is
a disposable projection of the specification; the specification is the truth.

## Build run — 2026-07-03

### Mode (build-modes.md §"A Build Runs In One Of Two Modes")
- **Attended (`--author`)**. The driver is working on the spec. On a specification
  ambiguity: HALT, surface it, fold the resolution into the spec via `clarify`,
  restart. Recorded here per build-modes.md §"The selected build mode MUST be
  recorded in the build's decision record."

### Phase 1 — user-facing choices (build.md §Phase 1)
1. **Seed host language:** Rust (declared default, `options/bootstrap-strategy/`).
   Environment confirmed: rustc 1.96.0, cargo, `wasm32-unknown-unknown` std present,
   `wasm-tools 1.242`, and cached crates `wit-bindgen`, `wat`, `wasmtime` (46/37),
   `wasm-encoder`/`wasmparser`, `sha2` (0.10.9), `ciborium` (0.2.2). Offline-capable.
   `wasmtime`/`wit-bindgen` CLIs absent — not needed; the runtime is the embeddable
   crate (learnings/2026-07-02-ignition-path-de-risked.md).
2. **Run scope:** straight to the seed toolchain (user chose to skip the throwaway
   Phase-2 spikes; the ignition path was de-risked in a prior session per memory).
   Codegen path is validated incrementally within synthesis, not as a separate phase.
3. **Runtime host:** embeddable component-model runtime = **Wasmtime crate**,
   in-process (`options/execution-model/wasm-component-model.md`).
4. **`options/` posture:** ACCEPT ALL DEFAULTS (the autonomous posture). Every
   decision resolves to its `DEFAULT:` choice:
   ast-encoding=binary-sexpr; hashing-and-encoding=sha256-deterministic-cbor;
   type-mapping=component-model-types; numeric-model=explicit-checked;
   execution-model=wasm-component-model; code-shape=homoiconic-decoupled-display;
   diagnostics-schema=coded-span-record; toolchain=pinned-identity;
   bootstrap-strategy=rust-seed-interpreted-first (native seed + compiled codegen);
   structural-interface=content-addressed-nodes; gate-non-load-bearing=change-process-and-excluded;
   realized-capability-set=seed-ignition-set; bootstrap-interpreter-surface=minimal-reflective-surface.
5. **Optional capabilities:** all declare default **include** and are recorded as
   included (effect-tracking, verification-layers, property-based-testing,
   units-of-measure). NONE are realized by the seed (`options/realized-capability-set/`),
   so their requirements are NOT load-bearing for the seed's ignition gate; they
   re-enter at the full-config gate for a later generation.

### Realized capability set (conformance-gate.md §"…Judged Against The Capabilities It Realizes")
The seed realizes the **seed-ignition-set**: core-semantics (incl. first-class
functions/closures), the mandatory capability-declaration floor, compiler-pipeline,
conformance-gate, self-hosting-and-bootstrap, and the primitive value forms
(Int64 checked, Bool, String, Float64 literal/equality, record, sum, list, map, unit).
The seed is a DYNAMIC interpreter (constitution §VII bootstrap carve-out) — it does
NO static type-checking; where a typed generation would reject, the seed evaluates
or traps at runtime. type-system is deferred to the first post-seed generation.

### Derivation mode (bootstrap.md §"Derivation Modes At Bootstrap")
- **Compiled derivation is the seed's mode**, but — CORRECTED after an attended HALT
  (see "ATTENDED HALT" below) — **the codegen is authored in Cadenza, NOT in Rust**.
  Rust provides ONLY the reference interpreter (the oracle). The Cadenza-authored
  compiler, run by the native seed interpreter over its source, emits the component's
  wasm bytes itself as an ordinary `Bytes` value. Interpreted derivation is the
  optional MAY, deferred.
- The seed's Rust modules are therefore: AST reader, value form + canonical bytes,
  the interpreter (oracle), diagnostics, the corpus/behavior-gate runner, a `Bytes`
  primitive, a minimal host that runs a derived component (wasmtime crate), and the
  driver that runs the Cadenza compiler's source through the interpreter. NO Rust
  codegen module.
- **The native Rust reference interpreter is the oracle** (not compiled to wasm);
  it realizes the full seed set and runs the behavior gate. The compiled component's
  observable behavior is checked against it.
- **Ignition scope:** clear the ignition bar = one real derived-and-run component,
  byte-identical re-derivation, imports mirror manifest, agreement with the oracle
  on the derived program(s). Full oracle-agreement across every realized case is a
  promotion obligation (higher than the ignition bar) and is reported honestly, not
  faked. At least two structurally-different programs are derived to demonstrate the
  codegen compiles logic (not a per-program transcript) — the anti-modeling guard
  (learnings/2026-07-02-decouple-interpreter-wasm-from-host.md,
  2026-07-03-real-components-not-a-bespoke-module-model.md).

## Spec defects / drift found during this run (attended — to fold back)
- **`commands/ignite.md` step 1** still prescribes interpreted-derivation-first
  ("a WebAssembly component embedding the interpreter"), contradicting the reshaped
  normative `spec/bootstrap.md` (compiled derivation is the MUST; native oracle) and
  `options/bootstrap-strategy/` + `options/execution-model/`. Command prose lags the
  2026-07-03 reshaping. Spec is authoritative; command prose to be realigned.
- **`commands/gate.md` step 5** cites `bootstrap.md §"Compiled Derivation Is An
  Oracle-Checked Optimization"`; the actual section is now §"Compiled Derivation
  Produces The Component And Agrees With The Oracle". Stale section reference.
These are command/spec drift (the commands are the build loop, not gated spec), not
a specification ambiguity — the spec itself is internally consistent — so they are
recorded and realigned rather than halted on.

## ATTENDED HALT — the seed↔compiler codegen seam (2026-07-03)
The operator halted synthesis when the build was about to author codegen IN RUST,
baking the compiler into the seed. Real underlying spec ambiguity: bootstrap.md
required the toolchain to "generate a component" (wasm bytes) while
options/realized-capability-set/ DEFERRED byte primitives — so a Cadenza program had
no byte type to emit wasm with, and nothing normative forbade putting codegen in the
seed. Resolution (operator choice): **Rust = interpreter/oracle ONLY; the compiler,
including codegen, is authored in Cadenza; the seed realizes a `Bytes` value form so
the Cadenza compiler emits component bytes as an ordinary value.**

Folded into the spec via /clarify (attended halt-and-harden):
- `spec/bootstrap.md` §"The Compiler Is Authored In Cadenza, Not In The Seed" (3 new
  requirements: bytes produced by evaluating the Cadenza compiler; seed MUST NOT
  contain an AST→bytes translation; compiler constructs bytes as a seed-realized
  byte-sequence value form pinned in options/).
- `spec/capabilities/self-hosting-and-bootstrap.md` §"Each Generation Is Derived By
  The Previous" (2 new requirements: translation authored in Cadenza not the seed;
  seed MUST realize a byte-sequence value form).
- `options/realized-capability-set/seed-ignition-set.md` — added `Bytes` to the
  seed's realized set (the declared default the requirements point to); reconciled the
  `minimal-reflective-surface.md` deferred-byte note (interpreter's AST-decoder bytes
  stay deferred — a distinct concern from the compiler's `Bytes` output).
- Corpus: added `spec/semantics/10-bytes.sexp` (7 `(needs bytes)` cases) witnessing
  the new behavioral requirement; registered in semantics/README; grounded the
  `Bytes.*` symbols in options/code-shape/; pinned two trap reasons in
  options/diagnostics-schema/.
Then RESET implementation/ and RESTARTED synthesis from the corrected spec (attended
mode: after folding a resolution, restart from the corrected spec, not from the halt).

## ANALYZE
- `ANALYZE: PASS` confirmed on the committed spec tree before the first synthesis
  attempt (all 8 checks; 544 requirements extract cleanly across 29 normative files;
  bootstrap ⊆ full; the two uncommitted frozen-contract diffs additive/editorial).
- Re-confirmed after each /clarify hardening. Final counts: 550 full / 351 ignition
  requirements extract cleanly; 66 corpus cases well-formed; 13/13 options defaults
  intact; no banned tokens in normative files.

## THIRD ATTENDED HALT — member access & modules-as-records (2026-07-03)
The interpreter used a lowercase/uppercase HEURISTIC to tell `p.x` (field access) from
`Sign.Neg` (qualified name) — the parse ambiguity the homoiconic principle forbids.
Resolved (operator): `.` is the SOLE record accessor `(. <record> <key>)`; `a.b` is
display sugar the reader expands to `(. a b)`; modules/records/prelude namespaces are all
**records** (fixed named fields), distinct from **maps** (dynamic homogeneous) — a
distinction load-bearing for the type system; module metadata via a `(meta <name>)` key
kept out of the export namespace. Folded into core-semantics.md §"Records, Maps, And
Member Access" + §"Modules" (10 new reqs), options/code-shape/, diagnostics trap table,
corpus 05. Learnings: spec/learnings/2026-07-03-one-accessor-modules-are-records.md (+ two
more this session: seed-realizes-bytes, the-compiler-emits-the-whole-component).

## IGNITION — cleared, real (2026-07-03)
`cadenza-seed ignite` → **IGNITION: PASS**. The seed interprets the Cadenza-authored
compiler (cadenza/compiler.cdz) over a program's binary AST; the Cadenza compiler emits
the complete 89-byte WebAssembly component as a Bytes value; the seed runs it.
- Real derived-and-run component (validated by BOTH the wasmtime crate AND external
  wasm-tools — /tmp/cadenza-derived-A.wasm), output 42.
- Byte-identical re-derivation (same content hash); imports mirror manifest (empty → 0);
  agrees with the reference interpreter (oracle).
- Anti-modeling guard: programs returning 42 vs 7 → components differ in exactly 1 byte
  (the i64.const operand) → the compiler compiles logic, not a transcript; each agrees
  with the oracle.
- Rust seed = interpreter/oracle + Bytes + deterministic binary-AST codec ONLY; NO Rust
  codegen. The Cadenza compiler produced the whole component itself.
- Behavior gate: 60 passed / 6 skipped / 0 failed. Codec round-trip unit tests pass.

## SELF-HOSTING — not reached; honest distance recorded
`cadenza-seed selfhost-probe`: the compiler's own source (1247-byte binary AST) round-trips
(it can receive itself), but gen-1 codegen emits wasm only for an integer-returning `main`.
Self-hosting requires emitting wasm for let/if/+/</=/member-access/def+application/recursion/
Bytes·List·Ast calls — a multi-generation climb via `regen`. NOT claimed as done.

## DIFFERENTIAL GATE + COMPILER GROWTH — 2026-07-03 (post-ignition)
Built a **differential gate** (`cadenza-seed differential-gate`; `src/differential.rs`): for
every realized corpus case, run the SAME program through BOTH the oracle (reference
interpreter) AND the Cadenza compiler → wasm → run, and compare observable behavior. Verdicts:
`agree` / `todo` (compiler declined — the honest backlog) / `skip` (unrealized capability) /
`disagree` (compiled behavior contradicts the oracle — the ONLY failing verdict, + malformed).
This operationalizes self-hosting-and-bootstrap.md §"A Derived Component Agrees With The
Oracle" and §"The Generated Path Is Exercised Before It Is Trusted" directly on the corpus,
and is the live checklist for growing the compiler (flip todo→agree). Refactors: `corpus.rs`
exposes `load_cases`/`Case`/`first_unrealized`/`value_form_expected` (one loader for both
gates); `derive.rs` adds `derive_node` (derive from an in-memory program tree).

Load-bearing invariant: the compiler MUST decline (trap → `todo`) rather than emit a wrong
component, so the gate never shows a false `disagree`. Every emitter guards operand kinds;
`leb-byte` declines outside its correct single-byte range instead of miscompiling.

Grew `cadenza/compiler.cdz` from a constant-extractor (1 agree) to a **recursive stack-machine
expression emitter** (16 agree / 49 todo / 6 skip / 0 disagree). Now emits: int + bool + FLOAT
literals; `(+ a b)`, `(< a b)`, `(= a b)` on ints; `let`/name refs via i64 wasm locals (slot
reuse + shadowing); `(do …)` sequencing; `if` (typed result blocktype); transparent `:`
annotation. A static kind synthesizer (INT/BOOL/FLOAT/OTHER) mirrors the emitter and drives the
component return type (s64 / bool / float64). New seed reflection natives:
`Ast.is-int/is-name/is-bool/is-float`, `Ast.name-value/bool-value/float-bytes`, `List.cons`.
Float literal = f64.const + 8 LE IEEE bytes from `Ast.float-bytes` (NaN canonicalized); `-0.0`
round-trips byte-exact; float `=` stays declined (IEEE eq ≠ canonical-byte eq → would disagree).
New wasm opcodes verified byte-exact against wasm-tools before hand-coding. Regressions clean
throughout: ignition PASS, behavior gate 65/0 (corpus grew to include `do`/module-name-binding,
which the oracle now realizes).

RESOLVED (operator ratified 2026-07-03) — **exhaustion across the compiled seam is a trap**.
Folded into spec via /clarify (attended): `spec/capabilities/self-hosting-and-bootstrap.md`
§"Exhaustion Is Observed As A Trap In A Derived Component" (2 requirements) + learning
`spec/learnings/2026-07-03-exhaustion-is-a-trap-across-the-compiled-seam.md`. Requirements
extract cleanly (file 24→26 reqs); bootstrap gate intact. The differential harness's
`observables_agree` treats oracle `Exhausted` vs component `Trap` as agreement. The frozen
`determinism-and-fuel.md` contract is untouched (it still governs emission: per-call fuel +
deterministic halt point); the compiler's fuel global satisfies it.

## COMPILER ARCHITECTURE + FUNCTIONS — 2026-07-03 (operator design directives)
The operator steered three design changes, all now reflected in `cadenza/compiler.cdz`:
1. **In-Cadenza WAT-like assembler.** Instead of hand-concatenating bytes, the emitter builds
   a structured instruction list (`op-i64-const`, `op-if`, `op-call`, `op-loop`, … — dot-free
   names, since the reader treats `a.b` as member-access sugar) and a Cadenza
   `assemble-seq`/`assemble-instr` folds it to `Bytes`. Raw wasm opcodes live ONLY in
   `assemble-instr`. CRUCIAL SEAM DECISION (operator chose the spec-true reading): the
   assembler runs INSIDE Cadenza, NOT as a Rust `wat`-crate final pass — a Rust pass would put
   a text→bytes translation in the seed and make the compiler emit a partial artifact a tool
   completes, contradicting frozen bootstrap.md L73 + L77. The `wat` crate (1.252, cached) was
   deliberately NOT used.
2. **Functions + recursion + fuel.** `core-module-multi` emits a fuel global + fuel-dec helper
   (func 0) + user defs (funcs 1..N) + a call convention (`call 0` before each `call <f>`), so
   recursion is accounted against the measure and unbounded recursion traps == oracle
   `exhausted`. A lone nullary `main` keeps the minimal single-function envelope (preserves the
   ignition byte layout). New seed reflection natives: `Ast.name-value/bool-value/float-bytes`,
   `Ast.is-int/is-name/is-bool/is-float`, `List.cons`. New `emit` debug subcommand.
3. **Explicit tail recursion (requested, NOT yet built).** `op-loop`/`op-br`/`assemble-loop`
   scaffolding is in place; a self-tail-call → loop+br (no stack growth) is the next increment,
   ideally motivated by a corpus case observing deep bounded tail recursion.
Also requested, queued as Phase C: **AST as a native sum type + `match`-in-the-compiler** to
retire the `Ast.is-*`/`Ast.*-value` accessor natives (carries an open design Q on qualified
variant names like `Ast.int` in patterns).

Differential gate now: **19 agree / 46 todo / 6 skip / 0 disagree**. Regressions clean: ignition
PASS, behavior gate 65/0, codec unit tests pass. Every new wasm encoding verified vs wasm-tools;
one bug (core-module-multi omitted the `\0asm` preamble) was caught by the gate as DISAGREE and
fixed.

## CONSOLIDATED LEARNINGS (2026-07-03) — folded to spec where they drove a requirement
Three findings became spec learnings + requirement edits (self-hosting-and-bootstrap.md now 30
reqs, analyze-clean, bootstrap gate intact):
- **Decline, do not miscompile** (§"An Unsupported Construct Is Declined, Not Miscompiled", 2 reqs;
  learning 2026-07-03-decline-do-not-miscompile.md). A compiler grown incrementally MUST trap/
  decline what it cannot yet compile, never emit divergent bytes nor silently skip — this keeps the
  differential gate's `todo` vs `disagree` meaningful and is what makes the whole climb safe.
- **The corpus is a differential gate** (§"The Generated Path Is Exercised Before It Is Trusted",
  +1 req; learning 2026-07-03-the-corpus-is-a-differential-gate.md). The generated path is exercised
  vs the oracle over every corpus case the compiler compiles — corpus becomes a live regression
  surface as the compiler grows.
- **The assembler lives in Cadenza** (§"Each Generation Is Derived By The Previous", +1 req; learning
  2026-07-03-the-assembler-lives-in-cadenza.md). Even instruction→bytes assembly is authored in
  Cadenza, never a host `wat`-crate pass.
Plus the earlier exhaustion=trap fold (above).

Implementation techniques worth keeping (synthesis knowledge, not spec):
- **Static kind synthesizer** mirrors the emitter (INT=0/BOOL=1/FLOAT=2/OTHER=3) and drives the
  component return type + `if`/`loop` result blocktypes. Kept in lockstep with `emit-*` so the
  predicted kind equals the kind the emitted code leaves on the stack.
- **Dotted names are member-access sugar** — instruction/def names must be dot-free (`op-i64-const`,
  not `i64.const`) or the reader rewrites them to `(. i64 const)`.
- **Verify every new wasm encoding empirically** with `wasm-tools parse <wat> && wasm-tools strip`
  before hand-coding the bytes in Cadenza; the `emit` subcommand dumps a program's derived bytes.
- **`.duvet` extraction is instant** — run analyze checks as direct `duvet extract`/`report`, not a
  slow agent workflow.

## NEXT PHASE — SELF-HOSTING (the north star; NOT yet achieved)
`cadenza-seed emit cadenza/compiler.cdz` → "derive declined: trap: byte value out of range". The
compiler can *receive* its own source (10,591-byte binary AST round-trips) but cannot yet *compile*
it. `selfhost-probe` names the gap. Critical-path clusters, in dependency order:
1. **Member access `(. record field)`** — the compiler's source uses prelude records on nearly every
   line (`List.at`, `Ast.is-list`, `Bytes.concat`, `Sign.*`). Biggest single unlock.
2. **`Bytes` / `List` / `Ast` operations as compiled calls** — the compiler *is* byte/list work.
3. **String values + string `=`** — instruction tags, `Ast.name-is`.
4. **`(list …)` construction + records** — the instruction representation itself.
5. **Multi-byte signed/unsigned LEB128** — the source's ~10 KB of constants exceed the −64..63 range.
6. **Phase C: AST-as-native-sum-type + `match`-in-the-compiler** — retires the ~13 `Ast.is-*`/
   `Ast.*-value` accessor natives, SHRINKING the source the compiler must self-compile. Converges with
   goal (1). Open design Q: how qualified variant names (`Ast.int`) appear in match patterns given
   `Ast.int` reads as member access.
Finish line = the compiler compiles its own source to a component that, run as the compiler,
reproduces byte-identical output (the fixpoint). Method unchanged: grow `compiler.cdz`, flip todo→
agree, keep disagree=0; regression set each step = differential-gate + ignite + behavior-gate + test.

## Build run — 2026-07-05 (AUTONOMOUS)

### Mode (build-modes.md §"A Build Runs In One Of Two Modes")
- **Autonomous** (no `--author`). Driver interactively chose "accept all defaults,
  skip the Phase-2 spikes, straight to the seed toolchain." No specification
  ambiguity was hit (spec is internally consistent; ANALYZE clean), so no
  declared-default fallback was exercised beyond the recorded posture.

### Phase 0 — Orient / ANALYZE
- Re-read README, AGENTS, constitution (0.5.0), overview, bootstrap, all 8 frozen
  contracts, and the ignition-subset capabilities. Post-pivot architecture confirmed:
  two compilers (cdz-rustc Rust seed + compiler.cdz), mandatory static typing
  (VII, Amendment 0.4.0), one wasm runtime.
- ANALYZE: PASS — 741 full / 513 ignition requirements extract cleanly across 31
  normative files (duvet, markdown format); bootstrap ⊆ full (0 bootstrap-only);
  all 17 options carry a DEFAULT pointing to an existing choice; 304 corpus cases
  well-formed; no banned tokens in normative files. `duvet report`'s 64 errors are
  CITATION-SIDE (stale gitignored-impl citations to renamed section slugs), not spec
  defects — clear on regen (memory: duvet-report-errors-are-citation-side).
- Operator-gated points all resolved in committed spec (ast-encoding, hashing,
  type-mapping, execution-model, realized-capability-set) → autonomous build permitted.

### Phase 1 — user-facing choices
Unchanged from the 2026-07-03 run: Rust seed (cdz-rustc), Wasmtime runtime, ACCEPT
ALL options defaults, all optional caps included-but-not-realized-by-seed.

### Phases 3–4 — synthesize / gate / ignition
The seed toolchain already existed from prior sessions. This run BROUGHT THE BEHAVIOR
GATE TO GREEN by fixing the 22 recorded corpus FAILs (all documented in agent memory).
Every fix is in the gitignored implementation/ only; the spec tree is untouched.

The 22 → 0 fixes (all in crates/cdz-compiler/src/), each guarding decline-don't-miscompile:
1. **record/map entry with no value PANIC** (2) — `eval_const` record/map arm now
   bounds-checks `kv.get(1)` → falls through to `check_type_rejections` which rejects
   CDZ0201 (added `malformed_kv_entry`). Never-crash restored.
2. **runtime shift masks & wraps** (2) — new `gen_shift`: emits an inline count guard
   (`(u64)count >= 64` → trap) + a left-shift overflow guard (`(r >> count) != a` → trap),
   so the runtime path enforces #Overflow Is Defined identically to the const fold. Added
   op::I64_GE_U.
3. **modulo INT_MIN by -1 const-fold traps** (1) — `fold_int_op` special-cases `% -1 → 0`
   (matches wasm i64.rem_s; Rust checked_rem over-conservatively trapped).
4. **tuple access on non-tuple not rejected** (2) — new `tuple.N` arm in
   check_type_rejections mirrors the `.`-on-non-record check (CDZ0201).
5. **compound value + scalar annotation not rejected** (2) — `matches_annotation` now
   rejects a compound StaticType against a known scalar type name (CDZ0203); added
   `is_scalar_type_name`.
6. **nominal vs plain-record comparison not rejected** (2) — `=` arm rejects CDZ0202 when
   exactly one operand is nominal and both share coarse type.
7. **list/map homogeneity misses compound shape** (4) — list-element + map-value arms now
   compare per-element SHAPE via shapes_incompatible (not just coarse KIND); added
   `first_const_element`.
8. **nested shape mismatch in equality not rejected** (2) — `shapes_incompatible` made
   RECURSIVE (tuple elements, record/map values, list elements, same-variant sum payloads),
   with a coarse-kind fallback for nested kind mismatch. Guarded so two DIFFERENT variants
   of the SAME sum stay compatible (compare-unequal, not a type error).
9. **unquote extra operand dropped in quasiquote** (1) — `malformed_unquote_arity` walks the
   quasiquote body; `unquote`/`unquote-splicing` with operand-count ≠ 1 → CDZ0201.
10. **strings not Unicode-normalized** (2) — reader NFC-normalizes string literals
    (unicode-normalization crate, pure/wasm-portable); equality + length see one form.
11. **QQ unquote-runtime AST not structurally equal** (2) — `gen_let` now binds a SCALAR
    LITERAL as a compile-time alias (like structural values), so `(unquote x)` with x=1
    folds to `(Ast.Int 1)` not `(Ast.Name "x")`.

### FINAL GATE STATUS (2026-07-05)
- **BEHAVIOR-GATE: PASS** — 265 agree, 0 disagree, 12 todo (honest declines:
  runtime-float-eq ×2, host-capability-lowering ×6, CDZ0401-undeclared-cap ×1,
  runtime-compound-output ×1, named-HOF-lambda-arg ×1, fn-in-tuple ×1, string-op-on-
  runtime-match ×1), 26 skip (unrealized caps: effects 5, sum-type-decl 6, numeric-model
  4, open-sums 4, eval 2, rows 2, type-system 2, self-hosting-surface 1).
- **IGNITION: PASS** — real 89-byte derived-and-run component (→42), byte-identical
  re-derivation, imports mirror manifest (empty→0), A-vs-B differ in 1 byte (compiles
  logic not a transcript).
- **COMPONENT-CHECK: PASS** — cdz-rustc built to wasm32-unknown-unknown component
  (0 imports) agrees byte-identically with the native compiler on 277 programs (the
  two-compilers-agree proof at the tooling level). NB: must target wasm32-unknown-unknown,
  NOT wasm32-wasip1 (the latter links wasi:cli/environment imports → instantiate failure).
- **cargo test: 10/10 pass** (8 seed integration + 2 codec round-trip).

The generation is now PROMOTABLE against its realized capability set (passes both gates).
Self-hosting remains the multi-generation climb (compiler.cdz cannot yet compile its own
source — the todos above are the honest backlog). New dep added to cdz-compiler:
unicode-normalization = "0.1" (pure, wasm-portable).

## M1 — Native sum-type declarations + match (2026-07-05, roadmap milestone)

Realized the `sum-type-declaration` capability: a program declares its own sum type with
`(type Name (V1 payload | V2 | …))` and constructs/matches its variants (`Color.Red`,
`Result.Ok`, `IntList.Cons`), exactly as it already could with the prelude sums (Option /
Result / Sign). This is the first roadmap milestone after the M0 green baseline; it most
shrinks the eventual `compiler.cdz` (retires the `Ast.is-*`/accessor idiom in favor of
native match). Added `sum-type-declaration` to `corpus::REALIZED`, flipping the 6
`(needs sum-type-declaration)` cases from skip → run, all green.

Root cause of all 6 prior skips (had they been run): a single missing wiring. The `(type …)`
declarations sit NESTED inside `main`'s `do` block (a declaration is scoped to the forms that
follow it), but the compiler only collected `(type …)` from a module's TOP-LEVEL forms — so
program-declared variants were unknown constructors, and the `(V | V | …)` declaration body
was misread as an over-applied constructor (CDZ0201). Four surgical edits in
`crates/cdz-compiler/src/codegen.rs`:

1. **`collect_sum_types` recurses** into every non-quoted subtree (was top-level only), so a
   `(type …)` nested in a `def` body's `do`/`let` registers its variants. `quote`/`quasiquote`
   bodies are pruned (a `(type …)` inside quoted data is an AST value, not a declaration).
2. **`gen_do` treats `(type …)` as inert** — skipped like `module`/`def` (compile-time-only,
   no runtime value); a trailing `(type …)` declines (a do block cannot end on a declaration).
3. **`check_tree` prunes the `(type …)` body** — its `(V | …)` variant syntax is not an
   ordinary expression, so a node-local check must not descend and reject it as over-application.
4. **`is_structural` recognizes a QUALIFIED constructor head** via `constructor_of` — a program
   sum's `(IntList.Cons …)` reads as `((. IntList Cons) …)` (a `.`-list head), which the old
   bare-`Name`-only check missed, so a `let`-bound recursive/nested sum was emitted as a runtime
   dotted-application (no lowering → "unsupported dotted-application" decline). Now aliased as a
   compile-time structure like any constructor value. This closed the recursive-`IntList` and
   nested-`Expr.Add (tuple …)` cases.

The **qualified-variant-in-pattern design question** (roadmap M1) needed NO new code: the
existing pattern matcher (`try_match` → `constructor_of`) already resolves both bare `(Some n)`
and qualified `(Status.Ready _)` / `(IntList.Cons (tuple head _))` patterns. Resolution:
qualified `Type.Variant` in a pattern is the canonical form; the matcher strips to the variant
tag via `variant_tag`. Recorded as settled.

**Corpus fix (spec/semantics/05-compound-types.sexp):** the "a sum type is declared with named
variants" case recorded `(output (: Color.Red Color))` — a BARE tag — but its input
`(Color.Red unit)` applies the nullary constructor to unit, yielding the Sum value that renders
`(Color.Red unit)`, the same `(Variant unit)` form every other nullary variant takes in the
corpus (`(None unit)` line ~585, `(Sign.Pos unit)` line 155). Rendering one value class two ways
violates the FROZEN deterministic-value-form contract (#A Value Has One Canonical Byte Form).
The compiler's `(Color.Red unit)` was correct; the recorded output was an authoring slip. Fixed
the output + doc. This was the ONLY behavior-gate FAIL; the compiler needed no change for it.

### GATE STATUS after M1 (2026-07-05)
- **BEHAVIOR-GATE: PASS** — 305 agree, 0 disagree, 12 todo, 20 skip. (The pass count also
  absorbs ~34 corpus cases a concurrent authoring session added to 03/05/06/10/13-*.sexp during
  this run — negative/empty-list-index traps, more numeric/string cases — all green.)
- **IGNITION: PASS** — 89-byte reproducible component (→42), A-vs-B differ in 1 byte.
- **COMPONENT-CHECK: PASS** — 317 agree, 0 disagree (native == wasm32-unknown-unknown component,
  byte-identical). Rebuilt the component (`cargo component build --release --target
  wasm32-unknown-unknown`) so the check proves the CURRENT compiler.
- **cargo test: 10/10** (8 seed integration + 2 codec round-trip).

M1 done. Next roadmap item: M2 — runtime compound heap + typed-result ABI (the big lift).
