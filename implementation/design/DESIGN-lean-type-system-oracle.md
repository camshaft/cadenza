# Design — a Lean type-system oracle: an independent typing judgment that validates rcdzc's REJECTIONS

**Author:** design agent (`design-type-oracle`).
**Audience:** the `vertical` agent that builds this — RECOMMENDED to be `v-lean-oracle` (it already owns
`implementation/oracle-lean/` *and* `cdz-smith/src/lean.rs`; a second vertical editing the same crates
would collide — §5), plus the `cdz-smith`/fuzzer owner (T2 differential), `v-inference` / `v-diagnostics`
(the compiler's type system + the CDZ code taxonomy), `v-nix` (the conformance derivation), and
`breaker`/`corpus-bugfix` (findings intake).
**Status:** DESIGN — the three major forks DECIDED by the operator on 2026-09-01 (§6); remaining choices are
implementation-local with chosen defaults (§7).
**Subsystem:** a new typing module + exe inside the EXISTING Lean project (`implementation/oracle-lean/`),
extensions to `cdz-smith` (a third `BatchItem` kind), and the Nix flake (a new conformance derivation).
Anchored on the frozen `spec/contracts/ast-encoding.md` and the normative `spec/capabilities/type-system.md`.

## 0. The principle — READ FIRST

We already have a Lean **semantics** oracle ([[DESIGN-lean-differential-oracle]],
`implementation/oracle-lean/`): given a program it re-derives the runtime VALUE/trap and asserts rcdzc
matches. That oracle validates **accepted** (well-typed) programs — it catches *miscompiles*.

This oracle is the **complementary direction**: an independent **type checker** that, given a program,
returns a **typing verdict** (well-typed / ill-typed-with-code) and asserts rcdzc's ACCEPT/REJECT decision
matches. The fuzzer drives it on rcdzc's **rejected** programs. Together the two give the operator's
**"oracles in both directions"** (operator, 2026-09-01, verbatim): *"It might be a good idea to have
another lean oracle for the cadenza type system. And the fuzzer would be able to call that on rejected
programs. That way we have oracles in both directions."*

**Why this catches bugs nothing else can — the false-reject blind spot.** `cdz-smith`'s existing
differential (`differential.rs`) compares rcdzc's **wasm** backend against its **rust** backend. Both share
the entire frontend (read → resolve → **typecheck** → lower), so when the frontend REJECTS a program, BOTH
sides decline identically — and the matrix has the explicit arm `(Side::Declined(_), _) | (_,
Side::Declined(_)) => Diff::Agree` (`cdz-smith/src/differential.rs:147`). **A decline is *never* a mismatch
today.** That is correct for the same-frontend differential (a decline there means "not comparable"), but it
means **no check anywhere validates that a rejection was JUSTIFIED.** A from-scratch Lean type checker that
shares zero code with rcdzc is the only way to catch a **false-reject** — rcdzc rejecting a program that is
actually well-typed. That independence is the whole value, and the rejected-program direction is *pure new
signal* the fleet has never had.

The dual — a **false-accept / soundness hole** (rcdzc accepts a program that is actually ill-typed, then
emits code for it) — is real and has bitten repeatedly (`if-branch-type-mismatch-not-rejected`,
`literal-pattern-type-mismatch-not-rejected`, `map-literal-key-type-homogeneity-not-checked`, an ill-typed
immediately-applied inline lambda → invalid wasm). The semantics oracle already catches *some* of these
(an ill-typed accept that miscompiles surfaces as a value/trap mismatch). This design PRIORITIZES the
false-reject direction (§6) and reaches false-accept detection for free at T4 (§2), when the oracle compares
inferred types on accepted programs.

**The niche is already reserved.** [[DESIGN-lean-differential-oracle]] §Phase L4 ("diagnostics parity")
foresaw a typechecking Lean model that emits `Error(code)` verdicts and matches rcdzc's codes. This design
promotes that rung into a first-class, fuzzer-driven, **bidirectional** oracle and sequences it as its own
top-to-bottom vertical, reusing the semantics oracle's binary-AST decoder, wire, and harness.

## 1. The oracle's shape (DECIDED)

### 1.1 The typing verdict algebra

The oracle is a pure, total function from a binary-AST module to a **typing verdict**:

```
infer : BinaryAstModule → TypeVerdict

TypeVerdict =
  | WellTyped  (τ : Ty)            -- accepts; τ = the principal type (compared only at T4, §2)
  | IllTyped   (code : Code)       -- rejects with a specific CDZ diagnostic code
  | Unsupported (reason : String)  -- the oracle declines to model this program — a SOUND coverage gap
```

`infer` does **name resolution + Hindley-Milner inference by unification** over the modeled subset
(`type-system.md:28-38`), and returns `Unsupported` for any construct outside that subset. It is total
(a fuel/size budget bounds it; a program that would not terminate typechecking → `Unsupported`, never a
hang). It has NO IO and shares NO code with rcdzc — it is a fresh reading of `spec/capabilities/type-system.md`.

### 1.2 The differential classification — how a verdict becomes a finding

The oracle's verdict is compared against **rcdzc's `cdz check` verdict** — the frontend-only accept/reject
decision (`cdz/src/main.rs:621 run_check`; exits non-zero on any error-severity CODED fault). rcdzc's
verdict is one of `accept | reject(code) | decline` (a **codeless** `Reject::decline` = "construct not yet
implemented" — `cdz check` exits 0 for it, but `cdz compile` rejects; the corpus grades it as *Todo*, never
a disagreement).

| rcdzc `cdz check`      | Lean oracle          | classification                                              |
|------------------------|----------------------|-------------------------------------------------------------|
| accept                 | `WellTyped`          | **agree** (compare τ only at T4)                            |
| `reject(code)`         | `IllTyped(code)`     | **agree** (T1); `code≠code′` → weak **code-mismatch** (T3)  |
| **`reject(code)`**     | **`WellTyped`**      | **FALSE-REJECT** — a compiler bug (over-strict CODED reject) — the highest-value finding |
| codeless **decline**   | `WellTyped`          | **CAPABILITY-GAP** — "should work, not yet built" (route as a backlog/`(output V)` TODO, NOT a soundness bug) |
| **accept**             | **`IllTyped`**       | **FALSE-ACCEPT / soundness hole** (reached at T4, §2)       |
| any                    | `Unsupported`        | **skip** — sound coverage gap, never a mismatch             |
| `reject`/`decline`     | `IllTyped`           | **agree**                                                   |

The **false-reject vs capability-gap split is load-bearing** (corpus policy, [[operator-corpus-policy-lock-in-idealistic-never-work-around-gaps]]): a Lean-accepts over an rcdzc CODED-reject is a *bug* (rcdzc
wrongly emitted a coded diagnostic for a valid program) → minimize + file as an `issue`. A Lean-accepts over
an rcdzc CODELESS-decline is a *known capability gap* (a should-work-but-unimplemented feature) → route as a
backlog item / corpus `(output V)` TODO. cdz-smith's triage distinguishes them purely by whether rcdzc's
carried verdict was coded. `Unsupported`/`skip` on the oracle side is *always* a coverage gap — growing
coverage can only ADD checks, never create a false alarm (the same soundness invariant `differential.rs`
uses for `Declined`).

### 1.3 The wire is a new `BatchItem` kind — zero verdict-protocol change

The fuzzer↔Lean boundary is the frozen binary AST, and it ALREADY multiplexes item kinds through one batch
frame (`cdz-smith/src/lean.rs`: `BatchItem = Trial | Equiv`, one `(verdicts …)` response, one verdict per
item in order). A typing check is a **third `BatchItem` variant** — no new frame, no verdict-protocol change:

```text
REQUEST  = (batch <item>…)
  <item> = (trial     <program> (args <v>…) (value <v>)|(trap "<r>"))   -- semantics assertion  (existing)
         | (equiv     <orig> <cadenza>)                                  -- symbolic equivalence (existing)
         | (typecheck <program> <rcdzc-verdict>)                         -- TYPING assertion     (NEW)
             <rcdzc-verdict> = (accept) | (reject "<CODE>") | (decline)
RESPONSE = (verdicts <v>…)   <v> = (holds) | (mismatch "<detail>") | (skip "<reason>")   -- UNCHANGED
```

For a `(typecheck <program> <rcdzc-verdict>)` item the oracle runs `infer program`, maps its `TypeVerdict`
against the carried `<rcdzc-verdict>` per §1.2, and emits `holds` (agree) / `mismatch("<detail>")` (a
finding) / `skip("<reason>")` (Unsupported). The `mismatch` detail string names the direction
(`false-reject: oracle infers <τ>`, `capability-gap: …`, `code-mismatch: oracle CDZ… vs rcdzc CDZ…`) so
cdz-smith triages without re-deriving. Reusing the existing `holds/mismatch/skip` protocol means the batch
may freely MIX all three item kinds and the pipeline (judge batch N while compiling batch N+1) is unchanged.

## 2. The increments (top-to-bottom, the way a vertical lands them)

### Phase T0 — verdict algebra, wire, declining skeleton

- **T0.1 — `Oracle/Type.lean` + the `(typecheck …)` wire, all-declining.** Add `TypeVerdict` (§1.1) and
  `infer : Module → TypeVerdict` returning `Unsupported` for every program. Extend `Oracle/Batch.lean`'s
  batch decoder with the `(typecheck <program> <rcdzc-verdict>)` node + a `judgeTypecheck` arm implementing
  the §1.2 comparison (declining ⇒ always `skip`). Rust side: extend `cdz-smith/src/lean.rs` `BatchItem` with
  a `Typecheck { program, rcdzc_verdict }` variant + `build_typecheck` encoder (mirrors `build_trial`);
  `decode_verdicts` is unchanged. **Gate:** a `(typecheck …)` item round-trips through `encode_batch` →
  `--batch-stream` → `(skip …)`; Lean + Rust unit tests (a Rust test mirrors `end_to_end_against_oracle_check`).
- **T0.2 — the corpus-conformance mode.** A conformance run that, per corpus case dir, decodes `program.ast`
  + `oracle-trial.ast` and asserts the oracle's typing verdict against the case's RECORDED expectation:
  `(expect-error CODE)` ⇒ expect `IllTyped` (code matched only at T3); `(expect-value …)`/`(expect-trap …)`
  ⇒ the program is well-typed ⇒ expect `WellTyped`; `Unsupported` ⇒ skip. Default: a new `oracle-typecheck`
  exe reusing `OracleCheck.lean`'s `--manifest` harness (§3). **Gate:** builds under nix; the declining
  skeleton yields all-`skip` (0 mismatch) over the whole corpus — the additive-baseline scaffold is in place.

### Phase T1 — the pure-total-core type checker + FIRST corpus conformance (the shipped value)

- **T1.1 — HM inference over the pure total core.** From the binary AST: resolve names, then infer by
  unification (`type-system.md:28-54`) over scalars (Int of each width / Bool / Unit / String / Char), `let`
  with **generalization** (`:40-44`), `if`, curried `fn`/closures + application, tuple / record / sum
  construction, `match` (first-match; non-exhaustive over a variant set → `IllTyped CDZ0210`), and ascription
  as a *constrain-not-contradict* constraint (`:50-54`; unsolvable → `IllTyped CDZ0203`). Emit the coded
  faults reachable from this subset: unbound name → `CDZ0101`, non-linear binder → `CDZ0102`,
  apply-non-function / arity / general unification failure → `CDZ0201`/`CDZ0203`. Anything outside the subset
  → `Unsupported(reason)`. **Gate:** Lean unit tests + the T0.2 harness green (**0 mismatch**) over the core
  subset of `07-type-system`, plus `01`/`02`/`09` and the tuple/sum subset of `05` — realized coverage
  reported and non-zero. This is the first shipped value: the oracle now independently agrees with rcdzc's
  accept/reject on every realized case.
- **T1.2 — the `.type-oracle-baseline` (additive-only).** Emit a baseline of realized (holds) cases, like
  `.oracle-baseline`/`.gate-baseline`. A `skip→mismatch` flip is a real oracle-vs-rcdzc disagreement, never
  allowed to land; pass/coverage counts drift only upward as slices land. **Gate:** the baseline diff is
  additive-only; coverage count non-zero.

### Phase T2 — the cdz-smith false-reject differential (the operator's CORE ask)

- **T2.1 — feed rcdzc's REJECTED programs to the oracle.** cdz-smith runs `cdz check` on each generated
  program (the frontend-only verdict, cheaper than a full compile), capturing `accept | reject(code) |
  decline`. For the REJECTED bucket — including the programs the existing `lean-differential --declines-dir`
  path ALREADY captures but never re-checks (`cdz-smith/src/bin/cdz-smith.rs`) — it builds `(typecheck
  <program> (reject "<CODE>"|(decline)))` items and streams them to the oracle. Classify per §1.2: oracle
  `WellTyped` + rcdzc **coded** reject → **false-reject** (shrink at the AST level + file to
  `.claude/fleet/queue/` as an `issue`); oracle `WellTyped` + rcdzc **codeless** decline → **capability-gap**
  (route as a backlog/`(output V)` TODO, lower urgency); oracle `IllTyped` → holds; `Unsupported` → skip.
  **Gate:** an injected false-reject (a program the oracle accepts, paired with a stubbed `(reject …)`) is
  caught, shrunk, and filed; a clean corpus-seed run reports 0 UNTRIAGED findings.

### Phase T3 — error-code parity (the "+code" depth — operator's diagnostics north star)

When both sides reject, match the **CDZ code** (`diag.rs:448-488` — `CDZ0203` TypeMismatch, `CDZ0201`
Malformed, `CDZ0301`/`CDZ0302` numeric, `CDZ0210` NonExhaustive, `CDZ0202` NominalMismatch, `CDZ0214`
AbstractCtor, `CDZ0211`/`CDZ0212` rows, `CDZ0401`-`0408` effects, `CDZ0501`/`0502` units, …). The oracle
emits the SPECIFIC code for its modeled faults; a `code≠code′` on an agreed rejection is a weaker
**code-mismatch** finding (diagnostic-quality — route to `v-diagnostics`/the code owner, not a
soundness bug). This flips code-bearing corpus cases from `skip` to `holds`, grown per code family, biggest
first: `CDZ0203`/`CDZ0201` (≈841 corpus cases combined), then `CDZ0301`/`CDZ0302` (numeric, ≈219), `CDZ0210`
(exhaustiveness, 80), `CDZ0101` (104), then `CDZ0202`/`CDZ0214`, `CDZ0211`/`CDZ0212`, effects, units.

### Phase T4 — principal-type agreement + the false-ACCEPT direction (the "+type" stretch)

When both sides ACCEPT, compare the inferred **principal type** up to alpha-renaming. rcdzc renders a
named-var scheme via `cdz type`/`cdz type-at` (`Scheme::render_scheme`/`Ty::render_named_vars`); the oracle
emits its own canonical type form on the wire (a `(type …)` sub-AST, or a rendered scheme string compared
after normalization — OQ-C). This is the strongest rung of the north star ("assert everything matches — down
to the diagnostics and error codes") and it OPENS the false-accept direction: streaming rcdzc's *accepted*
programs, an oracle `IllTyped` verdict is a **soundness finding** (rcdzc accepted an ill-typed program).
Land per type shape.

### Growing the type-system fragment (interleaves with T3/T4)

Each slice extends `infer`'s modeled subset — numeric width/range (`type-system.md`+`numeric-model.md`),
nominal newtypes + abstract types (`:154-204`), open rows + record row-ops (`:86-128`), effect rows via the
same row machinery (`:148-152`), generics as type-valued parameters + explicitly-passed dictionaries
(`:232-268`), units of measure — flipping a batch of corpus cases from `Unsupported` to realized and gated by
the additive `.type-oracle-baseline`. The bidirectional discipline holds throughout: the oracle only ever
reports a POSITIVE disagreement on a construct it fully models.

## 3. Seams / file anchors

*(Line numbers are landmarks at 2026-09-01, not promises.)*

| What | Where |
|------|-------|
| **Lean oracle project (reuse)** | `implementation/oracle-lean/` — its own Lake project; add `Oracle/Type.lean` (the typing judgment) alongside `Oracle/Eval.lean`; add an `oracle-typecheck` exe (or a `--typecheck` mode) alongside `OracleCheck.lean` |
| **AST decoder (reuse as-is)** | `implementation/oracle-lean/Oracle/Ast.lean` — `Ast.decode` (binary-AST → `Module`), already used by `Check.lean`/`Eval.lean` |
| **Batch wire (extend)** | `implementation/oracle-lean/Oracle/Batch.lean` — add the `(typecheck …)` decode arm + `judgeTypecheck`; verdict protocol `holds`/`mismatch`/`skip` UNCHANGED |
| **Corpus-conformance runner (mirror)** | `implementation/oracle-lean/OracleCheck.lean` — `checkCase`/`--manifest`/`Tally` (`:24,50,63-87`); the type runner mirrors this shape, mapping `expect-error`/`expect-value` → the expected `TypeVerdict` |
| **rcdzc verdict surface (reuse)** | `cdz check` (`cdz/src/main.rs:621 run_check`), coded/codeless split at `main.rs:346-359`; `cdz type`/`type-at` (`main.rs:393-429`) for T4 principal types |
| **Diagnostic code table** | `implementation/seed/crates/rcdzc/src/diag.rs:448-488` (`Code` enum → string); per-code semantics in the doc-comments `diag.rs:60-410` |
| **Corpus shredding (reuse)** | `implementation/seed/crates/cdz-corpus/src/cli.rs` — `shred_records` (`:573`) writes `program.ast` + `oracle-trial.ast` per case; the trial's expect clauses (`expect-error`/`expect-value`/`expect-trap`) are what T0.2 maps |
| **Corpus grading taxonomy (align)** | `implementation/seed/crates/cdz-corpus-grade/src/lib.rs` — `(error CODE)` = structured `(Error, code)` match (`:392`); codeless decline = Todo (`:433-450`); `(declines)` is DEPRECATED/removed (`:206-209`) |
| **cdz-smith wire (extend)** | `cdz-smith/src/lean.rs` — `BatchItem` (`:98`), `build_trial`/`build_equiv`/`encode_batch` (`:106-162`), `decode_verdicts` (`:199`); add a `Typecheck` variant + `build_typecheck` |
| **cdz-smith differential + declines feed (extend)** | `cdz-smith/src/differential.rs` (`Side::Declined`⇒Agree, `:147`); `--declines-dir` capture in `cdz-smith/src/bin/cdz-smith.rs`; generator `generator.rs`/`astgen.rs`; loop `cdz-smith/fuzz-cycle.sh` + `fleet/loops/fuzzer.md` |
| **Type-system spec (the model's source)** | `spec/capabilities/type-system.md` (normative; inference `:28-38`, ascription `:50-54`, structural `:70-84`, rows `:86-128`, effect rows `:148-152`, nominal `:154-204`, generics/dicts `:232-268`, soundness/erasure `:276-286`); corpus `spec/semantics/07-type-system.sexp` (+ `05`/`15`/`14*`/`18`) |
| **Nix (new derivation)** | the flake — mirror `oracleLeanCheck` (`flake.nix:5390`)/`oracleLeanShreds` (`:5363`) with an `oracleLeanTypeCheck` derivation running the type runner over the shredded corpus |

## 4. The gate that protects it

- **Corpus conformance** (new nix derivation `oracleLeanTypeCheck`, mirroring `oracleLeanCheck`) — green,
  **0 mismatch** across all realized cases; `.type-oracle-baseline` diff is **additive-only** (a
  `skip→mismatch` flip is a real oracle-vs-rcdzc disagreement, never allowed to land); coverage count non-zero.
- **Lean side** — `lake build` + `lake test`: `infer` unit cases (well-typed accepts, ill-typed rejects with
  the modeled codes, `Unsupported` for out-of-subset), and a `(typecheck …)` round-trip through `Batch.lean`.
- **`cargo xtask dev-gate`** for touched Rust crates (`cdz-smith`) — test + clippy + PINNED fmt.
- **Nix** — the new derivation builds; the existing `oracleLeanCheck`/`oracleLeanAstRoundtrip` stay green
  (this work only ADDS a module + exe to `oracle-lean`, it does not alter the semantics oracle).
- **Do NOT** touch `cdz-runtime` `//` comments or `wit/runtime.wit` (frozen `REQUIRED_RUNTIME_HASH`); this
  work does not need to.
- **Corpus policy** ([[operator-corpus-policy-lock-in-idealistic-never-work-around-gaps]]): a false-reject the
  oracle finds is a WIN — lock in the idealistic behavior (route the compiler bug to `v-inference`, and where
  the expected outcome is definite, add the minimized program to the corpus as a new `(output V)`/`(error
  CODE)` case). Never pin the oracle to match a current compiler gap.

## 5. Ownership / hand-off

**RECOMMENDED owner: `v-lean-oracle`** (the existing semantics-oracle vertical). Rationale — the single-writer
worktree invariant (AGENTS-fleet §1): this work edits `implementation/oracle-lean/` (a new module + exe) AND
`cdz-smith/src/lean.rs` (a new `BatchItem`) — **both are crates `v-lean-oracle` already owns**. A *separate*
vertical editing the same crates would collide on every sync. So the cleanest path is `v-lean-oracle`
expanding its charter to carry the type oracle as a parallel workstream (its harness/wire/decoder are the
exact reuse surface). If the PM/operator instead wants a dedicated `v-type-oracle`, it MUST coordinate tightly
with `v-lean-oracle` on those two crates (hand-off or a clear file-split) to avoid contention. The design
agent hands off after this doc lands + the queue brief is filed, then stands down.

**Coordination:** `cdz-smith`/fuzzer owner (T2 wiring + shrinker), `v-inference`/`v-diagnostics` (a confirmed
false-reject or code-mismatch is theirs to fix; the CDZ taxonomy is the shared contract), `v-nix` (the
conformance derivation), `breaker`/`corpus-bugfix` (findings intake).

**Trust / triage model.** Lean is a from-scratch reading of `spec/capabilities/type-system.md`. A confirmed
disagreement is exactly one of: **(a) rcdzc bug** — a false-reject (coded) or a false-accept (T4) → minimize +
file (the win); **(b) capability gap** — rcdzc codeless-declines a valid program → backlog/`(output V)` TODO;
**(c) oracle bug** — fix the Lean model; **(d) spec ambiguity** — escalate to the operator (concierge `ask`),
resolve the spec, then encode. The spec is the ultimate arbiter; the recorded corpus expectation is the
tie-breaker where it exists.

## 6. Resolved (operator DECISIONS, 2026-09-01) — do NOT re-litigate

- **Direction — FALSE-REJECT first.** v1 targets the rejected-program direction: feed rcdzc's rejections to
  the oracle; a Lean-accepts over a coded reject is the finding. This fills the exact blind spot the
  wasm-vs-rust differential has (decline ⇒ agree), matches the operator's framing, and the `--declines-dir`
  feed already exists. False-accept detection arrives at T4 (comparing types on accepted programs).
- **Verdict depth — STAGED: accept/reject → +code → +type.** T1 = the accept/reject boundary (catches
  false-reject/false-accept). T3 = match the CDZ error code when both reject. T4 = compare the inferred
  principal type when both accept. This realizes the north star ("down to the diagnostics and error codes")
  incrementally, each rung shipping value.
- **Housing + first fragment — REUSE `oracle-lean`, pure-core first.** A new `Oracle/Type.lean` + an
  `oracle-typecheck` exe INSIDE the existing `implementation/oracle-lean/` project (reuse the binary-AST
  decoder, batch wire, and `--manifest` harness). Model the pure total core first
  (scalars/let/if/fn/tuple/record/sum/match/ascription + Option/Result/Ordering), mirroring the semantics
  oracle's L1 subset, then grow (numeric-width → nominal/abstract → rows → effects → generics/dicts → units).
- **Input boundary — the binary AST** (inherited from [[DESIGN-lean-differential-oracle]] §6): the oracle reads
  `program.ast`, not source text; the textual parser is out of scope.
- **`Unsupported` is first-class and always a skip** (inherited): partial coverage integrates day one and grows
  monotonically; the oracle only ever reports a positive disagreement on a construct it fully models.

## 7. Open decisions (each with a chosen default; the vertical picks, escalate only a genuine fork)

- **OQ-A — separate `oracle-typecheck` exe vs a `--typecheck` mode on `oracle-check`.** *Default:* a separate
  `oracle-typecheck` exe for the corpus conformance (a clean second derivation, no risk to the semantics run),
  while the FUZZER path reuses the single `oracle-check --batch-stream` (the `(typecheck …)` `BatchItem` rides
  the existing mixed batch). Cheap to revise.
- **OQ-B — `cdz check` vs the full compile path as the rcdzc verdict.** *Default:* `cdz check` (the
  frontend-only, type-system verdict — the right thing to compare a type checker against; also faster in the
  fuzz loop). NOTE the known check≡compile gap (a codeless decline exits `check` 0 but `compile` rejects, and
  documented flatten/world-effect divergences): carry rcdzc's verdict as `accept|reject(code)|decline` from
  `check`, and treat `decline` as the capability-gap bucket (§1.2), not accept. Revisit only if a check-side
  gap produces noise.
- **OQ-C — the T4 principal-type wire form + comparison.** *Default:* compare rendered named-var SCHEME
  STRINGS after a canonical alpha-normalization (rcdzc `cdz type` already renders `a`,`b`,… via
  `render_named_vars`; the oracle renders the same convention) — cheaper than a structural `(type …)` sub-AST
  and sufficient for agreement. Escalate to a structural type-AST only if string comparison proves brittle.
- **OQ-D — code granularity in T3.** *Default:* match the exact `Code` enum variant (`diag.rs`); a
  `code≠code′` where BOTH are legitimate rejections is a *weak* finding (route to `v-diagnostics`, don't block
  the baseline). Only the accept/reject direction (T1) gates the additive baseline; code-mismatches are
  advisory until a code family is fully modeled.
- **OQ-E — fuzzer feed source in T2.** *Default:* both the live `cdz check` verdict on freshly generated
  programs AND the existing `--declines-dir` captured bucket (re-checked). The captured bucket is a ready
  backlog of real rcdzc rejections to validate on day one; the live path grows coverage continuously.
- **OQ-F — fuel/size budget for `infer`.** *Default:* a generous fixed budget; exceeding it → `Unsupported`
  (a coverage gap), never a hang or a spurious verdict — matching the semantics oracle's `Diverges`-soundness
  rule.
