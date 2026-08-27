# Design — a Lean reference interpreter for Cadenza as an independent differential oracle

**Author:** design agent (`design-lean-differential`).
**Audience:** the new `vertical` agent that builds this (suggested `v-lean-oracle`, `area=oracle-lean`),
plus the `cdz-smith`/fuzzer owner (L2 differential wiring), `v-nix` (flake + per-case derivation split),
and `breaker`/`corpus-bugfix` (findings intake).
**Status:** DESIGN — all major forks DECIDED by the operator on 2026-08-27 (§6); remaining choices are
implementation-local with chosen defaults (§7).
**Subsystem:** a new Lean/Lake project (`implementation/oracle-lean/`), a new `xtask` conformance
subcommand, `cdz-smith` (differential + binary-AST generator), and the Nix flake. Anchored on two frozen
contracts — `spec/contracts/ast-encoding.md` and `spec/contracts/deterministic-value-form.md`.

## 0. The principle — READ FIRST

The spec already reserves this exact niche (`spec/bootstrap.md`, §"A Reference Interpreter Is An
Optional Independent Oracle"):

> The toolchain MAY additionally realize a reference interpreter that evaluates a program's canonical
> representation directly, as an independent oracle whose observable behavior can be cross-checked
> against a compiled component. … A reference interpreter, if realized, MUST agree with the executable
> semantics on **every case it realizes**, so that offering it adds an independent check rather than a
> second definition of behavior. … MUST NOT be relied on as the runtime for any promoted generation.

Two phrases are load-bearing. **"canonical representation directly"** → the oracle reads the frozen
**binary AST**, not source text (§6). **"every case it realizes"** → the oracle is allowed to *decline*
a case it does not yet model; a first-class `Unsupported` verdict is what lets partial coverage integrate
on day one and grow monotonically (§6).

Operator's north star (verbatim, condensed): *"build an interpreter for the Cadenza language … take a
single corpus input with an optional input/output assertion and return if it holds … get to differential
testing … generate inputs that disagree under rcdzc and the Lean interpreter … massively scale up the
rate we find bugs."* And on scope: *"ultimately I want to assert that everything matches, down to the
diagnostics and error codes — a full model of the language in Lean that's verifiable but doesn't include
all of the wasm/multi-backend/portability constraints."*

**Why an independent oracle, and why now.** `cdz-smith` already runs a differential (`differential.rs`),
but it compares rcdzc's **wasm** backend against rcdzc's **rust** backend — both share the entire frontend
(read → resolve → typecheck → lower). The operator's read is that most real bugs are in the frontend, and
that this same-frontend oracle "hasn't really caught bugs." A Lean interpreter written from a fresh reading
of the spec shares **zero code** with rcdzc, so it can catch frontend miscompiles (resolve / typecheck /
lower / eval) that no rcdzc-vs-rcdzc check ever will. That independence is the whole value.

## 1. The oracle's shape (DECIDED)

### 1.1 Two stages: const-evaluate to minimal form, THEN execute an input

The oracle mirrors the two things rcdzc actually does — a **compile-time const-evaluation** that reduces
a program to its minimal form (rcdzc's `Meta.apply` / constant-folding, `eval.rs`), and a **runtime
execution** of an input against that reduced program. Modeling them as *separate* stages is what lets the
oracle assert the compiler matches behavior at each (operator directive, PR #4120 review):

```
reduce  : BinaryAstModule → Reduced                 -- compile-time: const-fold to minimal/normal form
execute : Reduced × Trial → Outcome                 -- runtime: run an input against the reduced program

oracle  : (modules : List BinaryAstModule) × (trials : List Trial) → List TrialVerdict

Trial        = { entry : ExportName, args : List Value, hostResponses : List HostResponse }
TrialVerdict = { outcome : Outcome, hostCalls : List HostCall }   -- hostCalls: ordered, verified

Outcome =
  | Value  (canonical-value-form bytes)   -- normal termination
  | Trap   (kind)                         -- div-by-zero | out-of-bounds | overflow | unreachable
  | Error  (code)                         -- compile-time rejection w/ diagnostic  [Phase L4]
  | Diverges                              -- fuel budget exhausted (loop guard)     [soundness: skip]
  | Unsupported (reason)                  -- oracle declines: feature not yet modeled [skip]
```

The corpus already draws this exact line, so the oracle grades each case against the stage rcdzc uses:
- A **bare `(input E)`** (no `(call …)`) is graded on the **const-eval** result — rcdzc folds it to a
  constant at compile time, so the oracle compares `reduce`'s residual value to `(output …)`.
- A **`(call entry args)`** trial supplies *runtime* argument values that **defeat constant-folding**
  (corpus README), so the oracle compares `execute`'s result. A case may interleave several `(call …)`
  trials over one reduced program.

Separating the stages buys a second, independent check on top of the rcdzc cross-check: **stage parity** —
const-evaluating a closed program MUST equal executing it. The oracle can hold itself to that (and rcdzc
to it too: a fold-vs-run divergence in the compiler is a miscompile the oracle surfaces directly).

Both stages are **pure and deterministic** in `(modules, args, hostResponses)`: no IO, no clock, no wasm.
Host effects are modeled by feeding the fixed `hostResponses` in call order (exactly the corpus
`(host-responses …)` fixture) and recording the `hostCalls` made (exactly `(host-calls …)`). That is what
makes a program with effects a pure function of its inputs — and what lets a disagreement be reproduced
deterministically. (Const-evaluation is effect-free by construction; only `execute` can perform host
ops, consuming `hostResponses` in order.)

### 1.2 The verdict algebra aligns 1:1 with the corpus grader

Comparison against rcdzc reuses the exact taxonomy `xtask`'s `grade_trial` already uses
(`xtask/src/main.rs:4042`), so "agree/disagree" means the same thing everywhere:

| Oracle outcome        | Compared to rcdzc / recorded expectation                                   |
|-----------------------|----------------------------------------------------------------------------|
| `Value(bytes)`        | **byte-equality** of the canonical value form (`deterministic-value-form.md`) |
| `Trap(kind)`          | canonical **kind** match (`trap_kind`, `main.rs:4182`) — div0/oob/overflow/unreachable |
| `Error(code)`         | diagnostic **code** match (+ optional message substring) — Phase L4        |
| `hostCalls`           | **ordered** host-call sequence match                                       |
| `Unsupported` / `Diverges` | **SKIP** — never a mismatch (soundness: coverage-gap, not a finding)  |

The `Unsupported`/`Diverges` = never-a-mismatch rule is the same soundness invariant the existing
differential already uses for `Declined` (`differential.rs:26-51`): the oracle only ever reports a
*positive* disagreement on outcomes it fully realizes, so growing coverage can only *add* checks, never
create false alarms.

### 1.3 The wire boundary is two frozen byte formats and nothing else

The Lean binary is a `lake exe cdz-oracle` that reads a request frame on stdin and writes verdicts on
stdout (mirroring `cdz run`'s stdout=value / stderr=trap convention). The only things that cross the
boundary are the **two frozen contracts**:

- **Input** — modules as raw `ast-encoding.md` bytes; a thin length-prefixed envelope for the trial list
  (entry symbol, arg values and host responses as `deterministic-value-form.md` bytes).
- **Output** — per trial: a tag byte + (canonical value-form bytes | trap kind | code | diverge |
  unsupported) + the ordered host-calls.

Keeping the boundary to exactly the two frozen formats is deliberate: the Lean side implements *those two
byte formats to the letter* and owes nothing to rcdzc's internal IRs. Both formats are versioned and
additive-only, so the boundary is stable.

## 2. The increments (top-to-bottom, the way a vertical lands them)

### Phase L0 — toolchain, skeleton, wire boundary

- **L0.1 — Lean/Lake project + Nix + a declining skeleton.** Create `implementation/oracle-lean/` as its
  own Lake project (mirrors `cdz-smith` being its own workspace). Add a `cdz-oracle` exe that parses the
  request frame and returns `Unsupported` for every trial. Flake integration so `nix develop` has Lean and
  `.#oracle-lean` builds. **Gate:** builds under Nix; a smoke request (one module, one trial) round-trips
  and yields `Unsupported`.
- **L0.2 — binary-AST decoder in Lean (`ast-encoding.md`).** Decode module bytes → a Lean `Ast` (leaf pool
  + `Struct = Atom | List`, symbol prelude by index). **Gate:** decode a fixture set of real
  corpus-derived module blobs, re-encode, assert **byte-identical** (bijective round-trip is the contract).
- **L0.3 — canonical value form in Lean (`deterministic-value-form.md`), scalars.** Encoder + total
  decoder for scalar values (ints of each width, Bool, Unit, String, Char). **Gate:** encode a table of
  scalar values **byte-identically** to `cdz-run`'s `render_val` output; decode refuses trailing/invalid
  bytes.

### Phase L1 — the pure-total-core evaluator + FIRST integration (corpus conformance)

- **L1.1 — the pure-total-core semantics as `reduce` + `execute` (§1.1).** From the binary AST: resolve
  names, then define the shared evaluation over Int64 / width ints / Bool, arithmetic with overflow +
  div-by-zero traps, comparisons/ordering, `let`, `if`, curried `fn`/closures + application, tuple/record/
  sum construction, `match` (first-match; non-exhaustive or unmodeled shape → `Unsupported`), and a minimal
  prelude (`Option`/`Result`/`Ordering`). Expose it as the **two stages**: `reduce` (const-fold a closed
  program to minimal form — grades bare `(input E)`) and `execute` (run a `(call …)` trial against the
  reduced program). Anything outside the covered subset → `Unsupported(reason)`. A **fuel budget** bounds
  evaluation → `Diverges`, in both stages. **Gate:** Lean unit tests (incl. a stage-parity check —
  `reduce` of a closed term equals `execute` with no args) + the L1.2 harness green on files `01-literals`,
  `02-binding-and-control`, `06-numeric-model` (integer subset), `09-functions`, and the tuple/sum subset
  of `05-compound-types`.
- **L1.2 — the corpus-conformance harness (`cargo xtask oracle-check`).** Shred each `spec/semantics/*.sexp`
  case via `cdz-corpus` into `(modules, trials, hostResponses)` (reusing `normalize_program`), invoke
  `cdz-oracle`, and compare each verdict to the **recorded expectation** using the `grade_trial` taxonomy
  (§1.2). `Unsupported`/`Diverges` → Todo (skip). Emit a `.oracle-baseline` (additive-only, like
  `.gate-baseline`) and report realized-coverage counts. **Gate:** runs green — **0 Fail** across all
  realized cases; coverage count reported and non-zero. This is the first shipped value: the oracle is now
  cross-checked against the whole executable semantics, and every realized case must agree.

### Phase L2 — cdz-smith differential (Lean as a third Side)

- **L2.1 — a binary-AST generator/mutator + shrinker.** Seed the fuzz corpus from the **real corpus
  programs with assertions stripped** (per operator), decode to binary AST, mutate **at the AST level**
  (no s-expr text — avoids wasting cycles balancing parens), and synthesize several call-arg tuples per
  program. A binary-AST shrinker for findings. **Gate:** the generator emits only decodable modules; the
  shrinker reduces a seeded finding.
- **L2.2 — wire Lean as a third `Side`.** Extend `differential.rs`'s `Side`/`compare` matrix: rcdzc-wasm
  `Value` vs Lean `Value` → byte compare; either side `Unsupported`/`Diverges`/`Declined` → agree
  (coverage-gap); a realized disagreement → shrink + file to `.claude/fleet/queue/` as an `issue`.
  **Gate:** a synthetically injected disagreement is caught, shrunk, and filed; a clean run over the corpus
  seeds reports 0 (untriaged) findings.

### Phase L3+ — grow coverage (each slice self-contained; the conformance baseline grows additively)

Collections (`List`/`Map`/`Set`/`String`/`Char`/`Bytes` ops) → `BigInt`/`Rational`/`Float64` (incl. the
float canonical form: `-0.0` distinct, single NaN) → algebraic **effects + handlers + host routing** (via
the `hostResponses` input and `hostCalls` output) → generics/dictionaries. Each slice flips a batch of
corpus cases from `Unsupported` to `Pass` and is gated by the additive `.oracle-baseline` diff.

### Phase L4 — diagnostics parity (the north star's final rung)

The Lean model does enough typechecking to emit `Error(code)` verdicts and match rcdzc's diagnostic codes
(and pinned message substrings). This turns `error`-expecting corpus cases from `Unsupported` into `Pass`
and makes Lean a **full verifiable model of the language** — values + traps + diagnostics — modulo
wasm/backend/portability. Land per diagnostic family.

## 3. Seams / file anchors

*(Line numbers are landmarks at 2026-08-27, not promises.)*

| What | Where |
|------|-------|
| **Lean oracle project (new)** | `implementation/oracle-lean/` — own Lake project. `lakefile`, `Oracle/Ast.lean` (binary-AST decode), `Oracle/Value.lean` (canonical value form), `Oracle/Eval.lean` (resolve + evaluate), `Oracle/Main.lean` (`cdz-oracle` exe + request/response frame) |
| **AST wire contract** | `spec/contracts/ast-encoding.md` — module input format the Lean decoder implements |
| **Value wire contract** | `spec/contracts/deterministic-value-form.md` — values in/out; the byte form Lean must reproduce |
| **AST shape reference** | `implementation/seed/crates/cadenza-ast/src/ast.rs:173-181` (`Struct = Atom(LeafId) \| List`), `codec.rs`/`leb128.rs` |
| **Corpus shredding (reused)** | `implementation/seed/crates/cdz-corpus/src/lib.rs` — `read` `:246`, `normalize_program` `:1080`, `Record/Trial/Call/Expect` `:31-123` |
| **Grading taxonomy (reused)** | `xtask/src/main.rs` — `grade_trial` `:4042`, `trap_kind` `:4182`; `cdz run` render `cdz-run/src/render.rs:11-34`, compound decode `cdz-run/src/lib.rs:2955` |
| **Conformance harness (new)** | new `xtask` subcommand alongside `gate` (`xtask/src/main.rs`) |
| **Differential (extend)** | `cdz-smith/src/differential.rs` — `Side`/`compare`/matrix `:26-101`; generator `src/generator.rs`; CLI `src/bin/cdz-smith.rs`; fleet loop `cdz-smith/fuzz-cycle.sh` + `fleet/loops/fuzzer.md` |
| **Nix** | the flake — the per-case derivation split the operator referenced; add `.#oracle-lean` + an oracle-conformance run |

## 4. The gate that protects it

- `cargo xtask oracle-check` (new) — green, **0 Fail**; `.oracle-baseline` diff is **additive-only**
  (`Todo→Fail` = a real oracle/compiler disagreement, never allowed to land); coverage count non-zero.
- Lean side — `lake build` + `lake test`: decode round-trip byte-identity (L0.2), value-form byte-identity
  (L0.3), and evaluator case tests (L1.1+).
- `cargo xtask dev-gate` for touched Rust crates (`xtask`, `cdz-smith`) — test + clippy + pinned fmt.
- Nix — `.#oracle-lean` builds; the flake's existing `harness-runs` stays green.
- **Do NOT** touch `cdz-runtime` `//` comments or `wit/runtime.wit` (frozen `REQUIRED_RUNTIME_HASH`); this
  work does not need to.

## 5. Ownership / hand-off

A new `vertical` agent (`v-lean-oracle`, `area=oracle-lean`) owns this top-to-bottom. It coordinates via
`fleet send` with: the **`cdz-smith`/fuzzer owner** (L2 differential + generator), **`v-nix`** (Lean
toolchain packaging + per-case derivation), and **`breaker`/`corpus-bugfix`** (findings intake). The design
agent (`design-lean-differential`) hands off after this doc lands and the queue brief is filed, then stands
down.

**Trust / triage model.** Lean is a from-scratch reading of the spec (`spec/capabilities/*` +
`spec/semantics/*`). A confirmed disagreement is exactly one of: **(a) rcdzc bug** — minimize + file to the
queue (the win); **(b) oracle bug** — fix the Lean model; **(c) spec ambiguity** — escalate to the operator
(via concierge `ask`), resolve the spec, then encode. The recorded corpus `(output …)` is the tie-breaker
where it exists; the spec is the ultimate arbiter. Every disagreement that resolves to a definite expected
outcome SHOULD be minimized and added to the corpus as a new `(case …)` — so the oracle's bug-finding
permanently grows the executable semantics.

## 6. Resolved (operator DECISIONS, 2026-08-27) — do NOT re-litigate

- **Language:** Lean 4. (Chosen for maximal independence — zero shared code with rcdzc — and proof
  potential; toolchain cost accepted.)
- **Input boundary:** the **binary AST**, NOT source text — the textual parser is explicitly out of scope
  ("I'm not super concerned with testing the text parser"; also avoids fuzzing wasting cycles on balanced
  parens). Input is a **list of modules + input/output trials**; **host-effect responses are part of the
  input** and **host-calls part of the output**, so the oracle is a pure deterministic function.
- **Subset:** pure total core first, with a first-class **`Unsupported`/decline** verdict so partial
  coverage integrates and yields value immediately, then grow.
- **Generation:** **extend `cdz-smith`** (not a standalone harness). Seed the fuzz corpus from the real
  corpus (assertions stripped); mutate/generate **at the binary-AST level**; the fuzzer generates several
  call-inputs per program; Lean is the **third differential Side**.
- **Two stages (PR #4120 review):** the oracle must **separate compile-time const-evaluation of the
  program to its minimal form from runtime execution of an input** — `reduce` mirrors rcdzc's const-fold
  (grades bare `(input E)`), `execute` runs a `(call …)` trial. This is how the oracle matches what the
  compiler does at each stage and asserts fold-vs-run parity (§1.1).
- **Loop guard:** a fuel-based **`Diverges`** verdict is required (random programs will loop).
- **North star:** assert **everything** matches — values, traps, **diagnostics + error codes** — a
  verifiable Lean model of the language, **excluding** wasm/multi-backend/portability constraints.

## 7. Open decisions (each with a chosen default; the vertical picks, escalate only a genuine fork)

- **OQ-A — the exact request/response frame.** *Default:* a length-prefixed frame — modules as raw
  `ast-encoding.md` bytes, a thin LEB envelope for the trial list, values as `deterministic-value-form.md`
  bytes, verdicts as tag-byte + payload. Cheap to revise.
- **OQ-B — Lean version pin + Nix packaging.** *Default:* pin a fixed Lean toolchain via the flake; the
  vertical + `v-nix` choose the mechanism (elan-pin vs a toolchain derivation).
- **OQ-C — fuel budget + whether `Diverges`-vs-`Value` is ever a finding.** *Default:* a generous fixed
  step budget; `Diverges` is a coverage-gap (skip), **never** a finding — matching the `Declined`-soundness
  rule. Revisit only if a real infinite-loop miscompile is suspected.
- **OQ-D — where conformance runs.** *Default:* `cargo xtask oracle-check` for the inner loop **plus** a Nix
  run for CI, sharing the shredding logic (`cdz-corpus`). `v-nix` + the vertical align on the derivation.
- **OQ-E — process model.** *Default:* one `cdz-oracle` invocation per request for conformance (simple,
  matches Nix per-case isolation); add a batched/streaming mode in L2 if fuzzing throughput needs it.
- **OQ-F — resolve vs typecheck depth before L4.** *Default:* L1–L3 do name-resolution + evaluation and
  treat any program that would be a compile *error* as `Unsupported`; full typechecking/diagnostics arrives
  in L4. So `error`-expecting corpus cases are skipped until then.
