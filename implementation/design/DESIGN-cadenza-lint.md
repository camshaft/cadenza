# Design — cadenza-lint: idiomatic lints (warn) + separately-applied fixes, wasm-usable

**Author:** design (interactive, with the operator). **Audience:** the vertical that builds the
idiomatic-lint tool, and anyone extending the structural lint / fix machinery.
**Status:** **DESIGN — scope DECIDED with the operator; nothing landed.** Every seam this builds on
is real *today* (the structural lint engine `cadenza-syntax/src/query.rs mod lint`; the machine-
actionable fix model `rcdzc/src/diag.rs` `Fix`/`Edit` + the format-preserving applier `cdz/src/fix.rs`;
the type/use columns exposed by `rcdzc/src/sidecar.rs`). The wasm constraint is already satisfied on
two independent paths (see §0). Line/module references are landmarks at this commit, not promises they
won't drift.

---

## 0. What already exists (the honest baseline — measure before building)

The operator's framing — a Cadenza lint tool that emits idiomatic-code WARNINGS, executes fixes
SEPARATELY, is all usable inside wasm, and leans on the compiler sidecar infrastructure (the operator
was explicit the command is a plain `cdz lint`, not a separately-branded tool) — lands on machinery that is
**~70% already built**. cadenza-lint is largely *assembly + productization + a catalog*, not a from-
scratch engine. The pieces:

| Piece | Where | What it gives the lint tool |
|---|---|---|
| Structural lint engine | `cadenza-syntax/src/query.rs` `pub mod lint` (~:3021) | `(lint PATTERN "message" [severity])` rules with metavars `,x`/`,@xs`, `Severity = Error\|Warning\|Info`, runs over the AST `Tree`, attaches spans. Purely syntactic (no scope/type). |
| Structural rewrite engine | `cadenza-syntax/src/query.rs` (`Pattern`/`Template`, `rewrite`/`rewrite_fixpoint`, `RuleSet`, `Strategy::BottomUp`) | the autofix substrate: a matched pattern → a template-built replacement subtree. |
| Machine-actionable fixes | `rcdzc/src/diag.rs:506` `struct Fix { label, edit, applicability }`, `:519` `enum Edit { ReplaceNode, InsertArms, Wrap, Delete }` (keyed on `StructId`; `Applicability` = Verified vs heuristic) | a diagnostic can carry a structural fix, projected over the ABI as `DiagnosticFix` (`abi.rs:76`). |
| Format-preserving applier | `cdz/src/fix.rs:24` `apply_fix_to_source` → `cadenza_syntax::query::textedit::rewrite_preserving` | one engine applies fixes for `cdz fix`, `cdz check --json`, and the LSP code-action — edits only the changed subtree, preserves layout/comments. |
| Diagnostic code registry | `rcdzc/src/diag.rs:53` `enum Code` → `CDZ####` | already ships code-quality warnings `UnusedBinding` (CDZ0306), `DiscardedValue` (CDZ0307), `UnreachableBranch` (CDZ0308) — the closest existing thing to idiomatic lint rules inside the compiler. |
| Type / use columns | `rcdzc/src/sidecar.rs:60` (`KIND_SIDECAR`); `Query::TypeOf` (infer's type column), `Query::UsesOf` (resolve's use column) landed | the type-directed rung reads these; `Query::Rewrite` is designed but **not built** — that is the seam a sidecar-driven autofix would extend. |
| CLI surface | `cadenza-syntax/src/cli.rs:63` `enum Cmd` (Query/Rewrite/Diff/Lint/Clones/Normalize), flattened into `cdz` (`cdz/src/main.rs`) | `cdz lint` already exists (exits non-zero on any `error`); this work extends it with named, fixable idiomatic lints. |
| Wasm execution | `rcdzc-wasm/src/lib.rs` (whole compiler as a wasm artifact); `cdz-wasm/src/lib.rs` (`cadenza_syntax` + `rcdzc` in a Web Worker) | the analysis already runs inside wasm today — the hard constraint is met, not open. |

**The two genuine gaps** cadenza-lint fills: (1) there is no idiomatic-lint *pack* and no convention
pairing a warning with its fix; (2) there is **no lint-level control** — no `allow`/`warn`/`deny`
anywhere (Rust-attribute-style suppression does not exist). Everything else is extension of shipped
machinery.

---

## 1. TL;DR — the win, the seam, the one insight

**The win.** A `cdz lint` that reports idiomatic-code WARNINGS (e.g. `if b then true else false`,
deeply nested single-scrutinee `match`, `let x = e in x`), each optionally carrying a *Verified*
structural fix that `--fix` applies as an equivalence-preserving codemod (§2) — usable inside wasm, and
eventually authored as ordinary Cadenza programs.

**The one insight.** *The lint and the fix are the SAME structural pattern read two ways.* A rule is
`(pattern → message [→ replacement-template])`: running the pattern yields a diagnostic (the warning);
binding the pattern's metavars into the replacement template yields the fix. This is exactly the
`query.rs` `Pattern`/`Template` pair, so warning-and-fix decoupling costs nothing — the fix is an
*optional* second half of one rule, and applying it is a separate action (`--fix` / a code-action),
never coupled to reporting.

**The seam already exists** (§0). cadenza-lint = a curated `LintSet` + an optional paired `Template`/
`diag::Fix` per rule + a lint-level layer + a `cdz lint` driver. No new AST, no new codec, no new
wasm path.

**The rung ladder** (matches `DESIGN-query-engine.md` / `DESIGN-sidecar-api.md`): syntactic lints live
in `cadenza-syntax` (rung 1, ships now); type-directed lints live in `rcdzc` reading the sidecar
columns (rung 2); lints authored as Cadenza `drive : Ast -> List Request` programs are the end state
(rung 3), gated on compiler-ml self-hosting + the unbuilt `Query::Rewrite`.

---

## 2. The catalog (the operator's step 1: enumerate the transforms first)

Each lint has: a **namespaced name** (`idiomatic/if-bool`), a **default level** (warn), a **message**,
and — where a canonical rewrite exists — a **fix** with an `Applicability` (Verified = always meaning-
preserving; Heuristic = offered, not auto-applied). Tier says where it runs.

**Fix semantics — "equivalence-preserving rewrite", NOT "format-preserving edit"** (v-syntax
correction). A `Verified` fix here does not mean a token-level in-place edit. Every idiomatic fix in
this catalog (`if-bool`, `redundant-let`, `single-arm-match`, …) produces a **structurally different
arena** — `(let x = e in x)` and `e` are different trees, not the same tree reprinted. So a lint fix
is a `match_to_let`-style **codemod** (the result re-reads as a different-but-equivalent tree), applied
only when the user explicitly opts in (`--fix` / a code-action), and it is deliberately NOT part of
`cdz fmt` (fmt must preserve the AST exactly for the round-trip guarantee). `Verified` therefore means
*semantically equivalent, structurally different*, licensed by a round-trip apply-and-recheck witness
test (§6). This is the correct bar for a consented autofix — it is not the format-preserving token-edit
path an in-place lint would use.

### Tier A — purely syntactic (rung 1, `cadenza-syntax`, no compile)

| Name | Pattern → rewrite | Fix applicability |
|---|---|---|
| `idiomatic/if-bool` | `(if ,c true false)` → `,c`; `(if ,c false true)` → `(not ,c)` | Verified |
| `idiomatic/if-same-branch` | `(if ,c ,e ,e)` → `,e` | Verified (both arms structurally equal) |
| `idiomatic/redundant-let` | `(let ,x ,e ,x)` → `,e` (bind then immediately return it) | Verified |
| `idiomatic/single-arm-match` | `(match ,x (,p ,e))` → `(let ,p ,x ,e)` — **only when `,p` is irrefutable AND unguarded** | Verified (DELEGATES to the existing `match_to_let` codemod, inheriting its precondition — see below) |
| `naming/camel-case` | a variable/binding name in `camelCase` → warn; offer rename to `snake_case` | Heuristic (auto-rename touches every use site; offer, confirm — operator-flagged) |
| `idiomatic/nested-match` | nested single-scrutinee `match` on the results of one `match` → one hoisted `match` (the operator's headline example) | Heuristic first (arm cross-product can change readability); promote to Verified once the tuple-scrutinee form is settled |
| `idiomatic/double-negation` | `(not (not ,e))` → `,e` | Verified |
| `idiomatic/deep-nesting` | call-CHAIN depth deeper than a threshold → warn (offer: hoist inner sub-expressions to `let`-bound named intermediates) | Heuristic (a refactor suggestion, not a mechanical rewrite — the names are the author's) |

> **Struck: `idiomatic/negated-eq`** (`(not (== ,a ,b))` → `(!= ,a ,b)`). This lint is VACUOUS — it
> has no valid rewrite target. Core Cadenza has NO `!=` node: the compiler has no `Prim::Ne` (only
> Lt/Gt/Le/Ge/Eq — see `spec/semantics/02-binding-and-control.sexp`, the "compiler has NO `Prim::Ne`"
> note), there is no `!=` lexer token or `op_str` head (a `!=` appears only in the cedar policy
> sublanguage), and `!=` desugars to `(not (= …))`. So `(not (= a b))` is ALREADY the canonical form
> with nothing to rewrite *to*. A lint that cannot fire on any real program is dead; struck by ruling
> 2026-08-09.

**Motivating real-world case (operator, PR #2790 `hm-collect.cdz`):** deeply-nested arguments in the
compiler-ml codebase, flagged as *not* setting a good example of idiomatic Cadenza. `idiomatic/deep-
nesting` is the lint that catches exactly this. It is a *structural depth* check (count argument/call
nesting depth on a form; warn past a threshold), which is Tier A (syntactic — no types needed). Its fix
is Heuristic: it can *suggest* hoisting inner sub-expressions to `let`-bound intermediates, but the
binding names are a human choice, so it offers the refactor rather than auto-applying it.

**Boundary with the formatter (coordinate with `v-syntax`).** `v-syntax` is separately assessing whether
part of the PR-#2790 nesting reads badly because of a *formatter layout* gap. Keep the two disjoint:
- **Formatter** = how existing structure is *printed* (line breaks, indentation of a given tree). It
  never changes the structure.
- **the `idiomatic/deep-nesting` lint** = flags the *structure itself* as non-idiomatic (too deep),
  independent of how it is printed.
A well-formatted deeply-nested expression is still a `deep-nesting` warning; a shallow expression laid
out badly is a formatter concern, not a lint one. The lint must not fire merely on layout, and the
formatter must not try to "fix" depth by reflowing. **Confirmed disjoint by v-syntax** on the #2790
case: the formatter already breaks the nested `collect(…)` args correctly (width-driven), leaving a
genuine 4-deep structural nest that no line-breaking un-nests — so it is squarely the lint pass
target, and the formatter does not need a structural-depth signal.

**`single-arm-match` — the delegation precondition (v-syntax, load-bearing).** The fix DELEGATES to
`cadenza-syntax/src/match_to_let.rs` rather than reimplementing the rewrite, and thereby inherits its
contract: it fires ONLY on a single arm whose pattern is **irrefutable and unguarded** (`match v { (a,b)
=> body }` → `let (a,b) = v in body`). A refutable or guarded single arm is NOT safe to convert (a
refutable pattern can fail; a `let` cannot), so the lint must apply the same precondition — the lint
rule reuses `match_to_let`'s precondition check and emits only when it passes. Reimplementing risks
dropping the guard.

**`naming/camel-case` (operator-flagged).** A pure syntactic lint: flag a variable/binding name that is
`camelCase` (Cadenza convention is `snake_case`). The optional fix is an auto-rename, which is Heuristic
because a rename must rewrite every use site of the binding (needs the `UsesOf`/resolve information to be
safe and to avoid capturing a distinct name) — so it is offered, not auto-applied by default. The
*warning* is purely syntactic (name shape) and needs no types; only the *rename fix* consults use sites.

### Tier B — type-directed (rung 2, `rcdzc`, reads sidecar columns)

| Name | Rule | Needs |
|---|---|---|
| `idiomatic/bool-compare` | `x == true` → `x`, `x == false` → `(not x)` | `TypeOf(x) = Bool` (guards against overloaded `==`) |
| `idiomatic/option-map` | `(match ,o (Some ,x) (,f ,x) None None)` → `(Option.map ,f ,o)` | `TypeOf(o) = Option _` |
| `idiomatic/redundant-conversion` | an identity/no-op conversion at a call boundary | `TypeOf` of arg vs param |

Tier B lints are the operator's "type-directed refactoring" — reject when the type isn't the one the
pattern assumes, so an overloaded operator or a shadowed name never mis-fires. **Tier A is the first
shipped increment; Tier B is a later increment once the framework is proven.**

**Type-guard predicates (v-inference guidance — read the `Ty` STRUCTURE, never the rendered string).**
`Query::TypeOf` answers via `Ty::render_name()` → a *string*; a lint that parses that string is fragile
(`Option _` prints an unsolved arg as `_`). A Tier-B lint running post-infer in `rcdzc` instead reads
the per-node solved type directly with `infer::type_of(db, node) -> Ty` and matches on the **enum**.
Three reject-not-fire soundness rules, all mandatory:
1. **Unsolved var / `Any` → BAIL.** `type_of` can return `Ty::Var(_)`/`Ty::Any` for a node whose solve
   isn't pinned; never rewrite off it. Guard `matches!(ty, Ty::Var(_) | Ty::Any)` and also reject a
   partially-unsolved compound with `ty::has_free_var(&ty)` (e.g. `(Option ?0)`).
2. **Defaulted numeric is solved-to-default, not unsolved.** A bare integer literal's width is deferred
   and grounds to `Int64`; it matches as `Int64` though the program never said so (and `has_free_var`
   is false, so it slips a naive "is it solved" check). Irrelevant to `bool-compare` (Bool is exact),
   but any width-keyed lint must know an `Int64` may be a defaulted width.
3. **Read final types — run Tier B as a settled post-typecheck pass**, not interleaved with solving
   (`type_of` does not cache a free-var/Any-bearing type; a mid-solve read can see a provisional `Any`).

Applied to the two first lints:
- **`bool-compare`** (`x == true` → `x`): guard `matches!(type_of(x), Ty::Bool)` exactly (Bool is never
  deferred), reject on `Var`/`Any`, AND confirm the `==` is the **primitive** eq, not a user-overloaded
  / member `==` (an overloaded `==` may be side-effect-free but not identity — do not rewrite it).
- **`option-map`** (`match o { Some x => f x; None => None }` → `Option.map f o`): guard the scrutinee's
  type is a `Ty::Sum` whose **decl identity is the prelude `Option`** — match on the decl occurrence /
  prelude identity, NOT the name string `"Option"` (a user can shadow the name). Reject if the scrutinee
  type is `Var`/`Any` or a same-named user sum; and verify the `Some` arm's body is exactly `f` applied
  to the binder (no extra effects), with the `None => None` arm structural.
v-inference offered to review the guard predicates once drafted.

Open catalog: this list grows increment-by-increment. New lints are additive (a new `LintRule` + a
`Code` entry for the type-directed ones); the catalog itself is the vertical's ongoing work.

---

## 3. Architecture — the two-tier rung ladder (DECIDED)

```
                 cdz lint [--fix] [--allow/--warn/--deny NAME]
                                |
        +-----------------------+------------------------+
        |                                                |
   Tier A (rung 1)                                  Tier B (rung 2)
   cadenza-syntax/query.rs                          rcdzc, over sidecar columns
   syntactic LintSet + paired Template              TypeOf / UsesOf + paired diag::Fix
        |                                                |
        +-----------------------+------------------------+
                                |
                    lint-level layer (allow/warn/deny)
                                |
                    diag::Fix / DiagnosticFix  ---->  cdz/fix.rs (apply, format-preserving)
                                                       (cdz fix / check --json / LSP code-action)
```

- **Reporting** always happens (warnings out). **Fixing** is a separate action: `--fix` applies only
  `Verified` fixes; `--fix --heuristic` opts into Heuristic ones; the LSP offers each as a code-action.
  Warning and fix are never coupled (operator's "execute fixes separately").
- **wasm:** Tier A rides `cdz-wasm` (the lint engine is a dependency-free library). Tier B rides
  `rcdzc-wasm` (the whole compiler already runs in wasm; the agent-kernel runs it via wasmtime). No new
  wasm work — this is why the two-tier split is free: neither tier introduces a native-only dependency.

### The paired-rule shape (DECIDED)

Extend the rung-1 rule form so a rule may carry an optional replacement + applicability:

```
(lint PATTERN "message" [severity])                              ; existing — report only, unchanged
(lint NAME PATTERN "message" [level] [=> TEMPLATE app])          ; extended — named + report + optional fix
```

The extended `(lint …)` form is a SUPERSET of the existing one: a leading `NAME` atom marks a named,
fixable idiomatic lint; a bare `(lint PATTERN …)` (no name) still parses as the existing report-only
rule, so `cdz lint`'s current surface is unchanged. `NAME` is the namespaced lint name (level/allow
control keys off it). `=> TEMPLATE app` is the fix:
`TEMPLATE` is an ordinary `query.rs` `Template` over the pattern's metavars; `app` ∈ `verified |
heuristic`. A rule with no `=>` is warn-only. Compiled by extending `LintRule` (`query.rs:3059`) with
`name: String`, `fix: Option<(Template, Applicability)>`.

For Tier B, the rule is a Rust `IdiomaticLint` in `rcdzc` that runs its pattern, consults `TypeOf`/`UsesOf`,
and emits a `diag::Diagnostic` carrying a `diag::Fix` (the existing `ReplaceNode` edit) — projected over
the ABI exactly like today's warnings.

### Lint levels (DECIDED — this is the one net-new mechanism)

Two controls, both additive:
1. **Module directive** `(allow NAME)` / `(warn NAME)` / `(deny NAME)`, validated against the fixed
   directive registry the way `pragma`/`bind` are today (`rcdzc/src/db.rs` `TOP_LEVEL_KEYWORDS` :6200;
   validated in `compile.rs` :767/:1584; unknown key → `UnknownDirective` CDZ0601). An unknown lint
   NAME reuses the same "did you mean?" path. `deny` promotes a warning to an error (fails the run);
   `allow` suppresses it.
2. **CLI override** `cdz lint --allow/--warn/--deny NAME` (repeatable), overriding the module level.

Level resolution order: CLI override > module directive > lint's default level. Names are namespaced
(`idiomatic/…`); a bare group name (`--allow idiomatic`) sets a whole group. This is the only piece
with no existing analogue, so it gets its own increment slice + a reject-test suite.

---

## 4. Increments (top-to-bottom, the way a vertical lands them)

**I1 — framework + level scaffold + first Tier-A lints (thin vertical slice, DECIDED as first drop).**
- Extend `query.rs` `LintRule` with `name` + optional `fix: (Template, Applicability)`; extend
  `compile_form` (`query.rs:3068`) to accept the named `(lint NAME …)` superset form.
- Implement the lint-level layer: the `(allow/warn/deny NAME)` directive (registry + validation +
  CDZ0601 reuse) and the `--allow/--warn/--deny` CLI flags; the resolution order in §3.
- Extend the existing `cdz lint` command (`cadenza-syntax/src/cli.rs` `enum Cmd` :63; `run_lint`;
  flatten into `cdz`) to report the named idiomatic warnings and, with `--fix`, apply Verified fixes.
  The bare `(lint …)` form and current `cdz lint` behavior stay unchanged — this is an additive
  extension of the one lint command, not a new subcommand (v-syntax review). The idiomatic fixes are
  arena-changing codemods (§2), applied via the `match_to_let`-style codemod path, not the token-level
  format-preserving edit path.
- Land the first Tier-A lints WITH fixes: `idiomatic/if-bool`, `idiomatic/redundant-let`,
  `idiomatic/single-arm-match` (delegating to `match_to_let` + its irrefutable/unguarded precondition),
  plus the pure `naming/camel-case` warning (operator-flagged; ship the warn first, the rename fix in
  I2 since it needs use-site rewriting).
- v-syntax owns `query.rs` and will gate the `LintRule` extension MR (lint round-trip + no regression on
  existing lint tests).
- **Gate:** unit tests per lint (a match-and-fix round-trip: source → warning fires → `--fix` →
  expected source; a `--allow` suppresses it; a `--deny` fails). `cdz lint --fix` output re-parses
  and re-lints clean (idempotent). `cargo xtask check` clean. No corpus/gate-baseline flips (the lint
  pass is additive, off the compile path).

**I2 — the rest of the Tier-A catalog.**
- `idiomatic/if-same-branch`, `idiomatic/double-negation`, `idiomatic/nested-
  match` (Heuristic), `idiomatic/deep-nesting` (Heuristic — the operator's PR-#2790 motivating case),
  and the `naming/camel-case` **rename fix** (needs `UsesOf` to rewrite every use site safely).
  (`idiomatic/negated-eq` was STRUCK — vacuous, no `!=` node to rewrite to; see the Tier-A table note.)
  Each: rule + fix + tests. Grow the catalog as coherent per-lint (or small-group) units.
- `idiomatic/deep-nesting` coordinates with `v-syntax`'s formatter-layout assessment (§2 boundary): the
  lint fires on structural depth, never on layout; a shared test asserts a well-formatted deep call still
  warns and a shallow-but-ugly one does not.

**I3 — Tier B (type-directed) lints in `rcdzc`.**
- A `lint` module in `rcdzc` that runs as a **settled post-typecheck pass** (§2 soundness rule 3),
  reads the per-node solved type via `infer::type_of(db, node)` and matches the `Ty` enum, consults
  `UsesOf` where needed, and emits `diag::Diagnostic` + `diag::Fix`.
- **Code band: a NEW `CDZ07xx` band** (v-diagnostics ruling — do NOT extend `CDZ03xx`). `03xx` is
  *provably-defective* (dead-code/reachability the compiler proves); lint rules are *advisory-
  idiomatic* (compiles-and-correct-but-not-idiomatic), a different kind — mixing them breaks the `03xx`
  contract. `07xx` is the next free band (05xx=dimensions, 06xx=directive/rename). Under-allocate and
  grow (codes are pinned forever): one code per Tier-B lint, allocated in order; v-diagnostics owns and
  blesses the exact numbers + lands the `diag.rs` `Code` scaffolds (the CDZ0407-style scaffold-then-
  wire-emit split), given the concrete Tier-B lint list. If Tier A ever wants stable codes too, split
  `0700-0749` type-directed / `0750-0799` syntactic — but only reserve what's used.
- First lints: `idiomatic/bool-compare`, `idiomatic/option-map` (guards per §2).
- **Gate:** reject/fix tests in `rcdzc/src/tests.rs`; the type guard verified (an overloaded `==` does
  NOT fire `bool-compare`; a shadowed user `Option` does NOT fire `option-map`; a `Var`/`Any` type
  bails); ABI `DiagnosticFix` round-trips; `cdz-wasm` surfaces the fix.

**I4 (design, later) — lints as Cadenza `drive` programs (rung 3).**
- Once compiler-ml self-hosts and `Query::Rewrite` lands in `rcdzc/src/sidecar.rs`, a lint becomes an
  ordinary Cadenza `drive : Ast -> List Request` program. The catalog migrates from native rules to
  Cadenza source. **Firmly future** (v-compiler-ml): `Query::Rewrite` is designed-not-built and NOT on
  the near-term compiler-ml queue (which is the ARC-B5 HM-inference stack then ARC-C/D/E); pick-up only
  if the operator prioritizes lint-autofix. The earlier rungs are designed so this is additive: the
  paired-rule shape maps cleanly — `Query`'s arms are already `StructId`-keyed, so a `StructId`-keyed
  `Rewrite` is the same addressing model; the sidecar validates the replacement re-typechecks (that IS
  the "validated transaction"), gating a `Verified` rewrite on a successful re-infer and surfacing a
  `Heuristic` one as advisory. **Caveat for whenever it lands:** the replacement subtree must be built
  as `cdzast` nodes the sidecar consumes (arena-consistent, ids stable) — the `Template` builder emits
  `cdzast`, not a surface re-parse (loop v-metaprog, who owns `Ast`/`Template` lowering).

---

## 5. Seams / file anchors

- `cadenza-syntax/src/query.rs:3021` `mod lint` — extend `LintRule`, extend `compile_form` for the named `(lint NAME …)` form.
- `cadenza-syntax/src/query.rs` `Template`/`rewrite` — the fix substrate.
- `cadenza-syntax/src/cli.rs:63` `enum Cmd` `Lint` + `run_lint` — extend for named/fixable lints + `--fix`.
- `cdz/src/fix.rs:24` `apply_fix_to_source` — the fix applier (reuse verbatim).
- `cdz/src/main.rs` — flatten `cdz lint`.
- `rcdzc/src/diag.rs:53` `enum Code` (add Tier-B codes in a new **CDZ07xx** band), `:506` `struct Fix`,
  `:519` `enum Edit` (reuse `ReplaceNode`/`Wrap`/`InsertArms`/`Delete` — do NOT add a variant without
  telling v-diagnostics; a new `Edit` kind that doesn't render on the ML surface in `cdz/fix.rs`
  silently drops the fix, the CDZ0210-class bug).
- `infer::type_of(db, node) -> Ty` — the structured per-node type read Tier B keys off (NOT
  `Ty::render_name()`); `ty::has_free_var` for the partial-unsolved guard.
- `cadenza-syntax/src/match_to_let.rs` — the codemod `idiomatic/single-arm-match` delegates to
  (irrefutable+unguarded precondition).
- `rcdzc/src/sidecar.rs:60` — `TypeOf`/`UsesOf` columns for Tier B; `Query::Rewrite` seam for rung 3.
- `rcdzc/src/db.rs:6200` `TOP_LEVEL_KEYWORDS` + `compile.rs:767/:1584` — the directive registry the
  `(allow/warn/deny)` control extends; `diag.rs:337` `UnknownDirective` (CDZ0601) reused for bad names.
- `rcdzc-wasm/src/lib.rs`, `cdz-wasm/src/lib.rs` — the wasm execution paths (no change needed).

## 6. Soundness & the gate

- **The round-trip witness licenses `Verified`** (v-diagnostics): a `Verified` fix MUST have a test that
  applies the fix and proves the result is an equivalent, still-compiling program (apply-and-recheck).
  A fix without that witness is `Heuristic`. This is what makes an "equivalence-preserving but arena-
  changing" fix (§2) safe to auto-apply under `--fix`.
- **A fix must be type/well-formedness-consistent in its position** (v-diagnostics, CDZ0302-#1784 rule):
  never emit a fix that, applied, yields ill-typed or ill-formed source — advisory lints are especially
  prone (the "idiomatic" form may not typecheck in every context the naive form does). Every fix needs
  an apply-and-recheck test. `cdz lint --fix` output MUST re-lint clean + re-parse (idempotence).
- **Fix labels** are one-line imperative, naming the idiomatic target ("replace with `foo`"); display-
  only. The machine-actionable part is the `Edit`.
- the lint pass is OFF the compile path: it introduces no corpus/gate-baseline flips. The gate is per-lint
  unit tests + `cargo xtask check` + (Tier B) `rcdzc` reject/fix tests + a `cdz-wasm` surfacing test.
- Tier B lints MUST guard on the exact `Ty` they assume via the enum-structure reads in §2 (reject on
  `Var`/`Any`/`has_free_var`, key sum identity off the decl not the name, confirm primitive `==`), and
  run as a settled post-typecheck pass — so overloading/shadowing/unsolved-types never mis-fire.

## 7. Territory & coordination

- A new `vertical` owns cadenza-lint top-to-bottom (area straddles `cadenza-syntax`/`cdz` for Tier A
  and `rcdzc` for Tier B). Tier A touches only lint-owned surface; Tier B adds `Code` entries + a
  `lint` module in `rcdzc` — coordinate with `v-inference` (owns infer/resolve/the columns; has
  reviewed the type-guard predicates), `v-syntax` (owns `query.rs`; gates the `LintRule` extension MR +
  the `match_to_let` delegation + the formatter boundary), and `v-diagnostics` (owns the diagnostic
  registry; will allocate the CDZ07xx numbers + land the `diag.rs` `Code` scaffolds given the Tier-B
  lint list). `v-compiler-ml` owns the rcdzc sidecar (`Query::Rewrite` for rung 3, firmly future);
  `v-metaprog` owns `Ast`/`Template` lowering (rung-3 caveat).
- Overlap to respect: `cadenza-syntax/src/match_to_let.rs` (the existing normalize codemod) is the
  precedent for `idiomatic/single-arm-match`; reuse it, don't duplicate.

## 8. Decisions (resolved with the operator) & open questions

**Decided:** two-tier rung ladder (syntactic in `cadenza-syntax`, type-directed in `rcdzc`, self-hosted
`drive` later) · paired rule (lint + optional structural fix, applied separately) · lint levels YES
(module directive + CLI override, namespaced names) · first increment = thin vertical slice (framework +
level scaffold + first Tier-A lints end-to-end with fixes).

**Confirmed by owners (peer collaboration, 2026-08-08):**
- *v-syntax* — reuse `query.rs` `mod lint` + a `cdz lint` sibling subcommand (they gate the MR); keep
  `cdz lint` unchanged; `single-arm-match` DELEGATES to `match_to_let` (inherit its irrefutable+unguarded
  precondition); the idiomatic fixes are equivalence-preserving *arena-changing codemods*, NOT format-
  preserving token edits, and stay out of `cdz fmt`. Formatter/lint boundary confirmed disjoint.
- *v-inference* — reading types post-infer is the intended use; read the `Ty` enum via `type_of(node)`,
  never the render string; reject on `Var`/`Any`/`has_free_var`; deferred-numeric is solved-to-default;
  run Tier B as a settled pass; key sum identity off the decl. (Guards in §2.)
- *v-diagnostics* — allocate a NEW **CDZ07xx** band (not `03xx`); `Verified` needs a round-trip witness;
  reuse existing `Edit` variants; they own + bless the exact numbers and land the `Code` scaffolds.
- *v-compiler-ml* — `Query::Rewrite` (rung 3) is firmly future, not near-term; the paired-rule shape maps
  cleanly (`StructId`-keyed, validated-transaction = re-typecheck); replacement must emit `cdzast`.

**Open (chosen defaults):**
- Fix conflict resolution when two lints touch overlapping spans in one `--fix` pass. *Default:* apply
  bottom-up, non-overlapping (the `apply_edits` skip-on-overlap rule at `query.rs:3003` already does
  this); a second `cdz lint --fix` pass catches the rest (idempotence covers it).
- Whether `--deny`-promoted lint warnings should gate CI. *Default:* the lint pass stays advisory (its own
  subcommand, non-zero only under `--deny`); wiring it into the fleet gate is a separate operator call.
- Group-level names (`--allow idiomatic`). *Default:* support a bare group prefix as "all lints under
  it"; finer taxonomy deferred until the catalog is larger.

## 9. Fold into the frozen contract

Additive w.r.t. `spec/contracts/`: lint warnings are ordinary diagnostics (already have stable codes,
severity, and machine-actionable fixes per `spec/capabilities/diagnostics.md` + `constitution.md` §XI).
The `(allow/warn/deny)` directive extends the module-directive registry (`spec/capabilities/modules-
and-namespaces.md`) — a new key, validated like the existing ones. `cdz lint` is a new tooling
subcommand under `spec/capabilities/tooling-and-lsp.md` + `spec/contracts/build-tool-interface.md`. No
change to the compile ABI or the runtime hash.
