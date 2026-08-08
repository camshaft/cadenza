# Design — cadenza-clippy: idiomatic lints (warn) + separately-applied fixes, wasm-usable

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

The operator's framing — "think of this as a cadenza clippy: emit WARNINGS, execute fixes SEPARATELY,
all usable inside wasm, lean on the compiler sidecar infrastructure" — lands on machinery that is
**~70% already built**. cadenza-clippy is largely *assembly + productization + a catalog*, not a from-
scratch engine. The pieces:

| Piece | Where | What it gives clippy |
|---|---|---|
| Structural lint engine | `cadenza-syntax/src/query.rs` `pub mod lint` (~:3021) | `(lint PATTERN "message" [severity])` rules with metavars `,x`/`,@xs`, `Severity = Error\|Warning\|Info`, runs over the AST `Tree`, attaches spans. Purely syntactic (no scope/type). |
| Structural rewrite engine | `cadenza-syntax/src/query.rs` (`Pattern`/`Template`, `rewrite`/`rewrite_fixpoint`, `RuleSet`, `Strategy::BottomUp`) | the autofix substrate: a matched pattern → a template-built replacement subtree. |
| Machine-actionable fixes | `rcdzc/src/diag.rs:506` `struct Fix { label, edit, applicability }`, `:519` `enum Edit { ReplaceNode, InsertArms, Wrap, Delete }` (keyed on `StructId`; `Applicability` = Verified vs heuristic) | a diagnostic can carry a structural fix, projected over the ABI as `DiagnosticFix` (`abi.rs:76`). |
| Format-preserving applier | `cdz/src/fix.rs:24` `apply_fix_to_source` → `cadenza_syntax::query::textedit::rewrite_preserving` | one engine applies fixes for `cdz fix`, `cdz check --json`, and the LSP code-action — edits only the changed subtree, preserves layout/comments. |
| Diagnostic code registry | `rcdzc/src/diag.rs:53` `enum Code` → `CDZ####` | already ships code-quality warnings `UnusedBinding` (CDZ0306), `DiscardedValue` (CDZ0307), `UnreachableBranch` (CDZ0308) — the closest existing thing to clippy lints inside the compiler. |
| Type / use columns | `rcdzc/src/sidecar.rs:60` (`KIND_SIDECAR`); `Query::TypeOf` (infer's type column), `Query::UsesOf` (resolve's use column) landed | the type-directed rung reads these; `Query::Rewrite` is designed but **not built** — that is the seam a sidecar-driven autofix would extend. |
| CLI surface | `cadenza-syntax/src/cli.rs:63` `enum Cmd` (Query/Rewrite/Diff/Lint/Clones/Normalize), flattened into `cdz` (`cdz/src/main.rs`) | `cdz lint` already exists (exits non-zero on any `error`); clippy adds a `cdz clippy` sibling. |
| Wasm execution | `rcdzc-wasm/src/lib.rs` (whole compiler as a wasm artifact); `cdz-wasm/src/lib.rs` (`cadenza_syntax` + `rcdzc` in a Web Worker) | the analysis already runs inside wasm today — the hard constraint is met, not open. |

**The two genuine gaps** cadenza-clippy fills: (1) there is no idiomatic-lint *pack* and no convention
pairing a warning with its fix; (2) there is **no lint-level control** — no `allow`/`warn`/`deny`
anywhere (Rust-attribute-style suppression does not exist). Everything else is extension of shipped
machinery.

---

## 1. TL;DR — the win, the seam, the one insight

**The win.** A `cdz clippy` that reports idiomatic-code WARNINGS (e.g. `if b then true else false`,
deeply nested single-scrutinee `match`, `let x = e in x`), each optionally carrying a *Verified*
structural fix that `--fix` applies format-preservingly — usable inside wasm, and eventually authored
as ordinary Cadenza programs.

**The one insight.** *The lint and the fix are the SAME structural pattern read two ways.* A rule is
`(pattern → message [→ replacement-template])`: running the pattern yields a diagnostic (the warning);
binding the pattern's metavars into the replacement template yields the fix. This is exactly the
`query.rs` `Pattern`/`Template` pair, so warning-and-fix decoupling costs nothing — the fix is an
*optional* second half of one rule, and applying it is a separate action (`--fix` / a code-action),
never coupled to reporting.

**The seam already exists** (§0). cadenza-clippy = a curated `LintSet` + an optional paired `Template`/
`diag::Fix` per rule + a lint-level layer + a `cdz clippy` driver. No new AST, no new codec, no new
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

### Tier A — purely syntactic (rung 1, `cadenza-syntax`, no compile)

| Name | Pattern → rewrite | Fix applicability |
|---|---|---|
| `idiomatic/if-bool` | `(if ,c true false)` → `,c`; `(if ,c false true)` → `(not ,c)` | Verified |
| `idiomatic/if-same-branch` | `(if ,c ,e ,e)` → `,e` | Verified (both arms structurally equal) |
| `idiomatic/redundant-let` | `(let ,x ,e ,x)` → `,e` (bind then immediately return it) | Verified |
| `idiomatic/single-arm-match` | `(match ,x (,p ,e))` → `(let ,p ,x ,e)` | Verified (mirrors the existing `match_to_let` normalize codemod) |
| `idiomatic/nested-match` | nested single-scrutinee `match` on the results of one `match` → one hoisted `match` (the operator's headline example) | Heuristic first (arm cross-product can change readability); promote to Verified once the tuple-scrutinee form is settled |
| `idiomatic/double-negation` | `(not (not ,e))` → `,e` | Verified |
| `idiomatic/negated-eq` | `(not (== ,a ,b))` → `(!= ,a ,b)` | Verified |

### Tier B — type-directed (rung 2, `rcdzc`, reads sidecar columns)

| Name | Rule | Needs |
|---|---|---|
| `idiomatic/bool-compare` | `x == true` → `x`, `x == false` → `(not x)` | `TypeOf(x) = Bool` (guards against overloaded `==`) |
| `idiomatic/option-map` | `(match ,o (Some ,x) (,f ,x) None None)` → `(Option.map ,f ,o)` | `TypeOf(o) = Option _` |
| `idiomatic/redundant-conversion` | an identity/no-op conversion at a call boundary | `TypeOf` of arg vs param |

Tier B lints are the operator's "type-directed refactoring" — reject when the type isn't the one the
pattern assumes, so an overloaded operator or a shadowed name never mis-fires. **Tier A is the first
shipped increment; Tier B is a later increment once the framework is proven.**

Open catalog: this list grows increment-by-increment. New lints are additive (a new `LintRule` + a
`Code` entry for the type-directed ones); the catalog itself is the vertical's ongoing work.

---

## 3. Architecture — the two-tier rung ladder (DECIDED)

```
                 cdz clippy [--fix] [--allow/--warn/--deny NAME]
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
(lint PATTERN "message" [severity])                         ; existing — report only
(clippy NAME PATTERN "message" [level] [=> TEMPLATE app])   ; new — report + optional fix
```

`NAME` is the namespaced lint name (level/allow control keys off it). `=> TEMPLATE app` is the fix:
`TEMPLATE` is an ordinary `query.rs` `Template` over the pattern's metavars; `app` ∈ `verified |
heuristic`. A rule with no `=>` is warn-only. Compiled by extending `LintRule` (`query.rs:3059`) with
`name: String`, `fix: Option<(Template, Applicability)>`.

For Tier B, the rule is a Rust `ClippyLint` in `rcdzc` that runs its pattern, consults `TypeOf`/`UsesOf`,
and emits a `diag::Diagnostic` carrying a `diag::Fix` (the existing `ReplaceNode` edit) — projected over
the ABI exactly like today's warnings.

### Lint levels (DECIDED — this is the one net-new mechanism)

Two controls, both additive:
1. **Module directive** `(allow NAME)` / `(warn NAME)` / `(deny NAME)`, validated against the fixed
   directive registry the way `pragma`/`bind` are today (`rcdzc/src/db.rs` `TOP_LEVEL_KEYWORDS` :6200;
   validated in `compile.rs` :767/:1584; unknown key → `UnknownDirective` CDZ0601). An unknown lint
   NAME reuses the same "did you mean?" path. `deny` promotes a warning to an error (fails the run);
   `allow` suppresses it.
2. **CLI override** `cdz clippy --allow/--warn/--deny NAME` (repeatable), overriding the module level.

Level resolution order: CLI override > module directive > lint's default level. Names are namespaced
(`idiomatic/…`); a bare group name (`--allow idiomatic`) sets a whole group. This is the only piece
with no existing analogue, so it gets its own increment slice + a reject-test suite.

---

## 4. Increments (top-to-bottom, the way a vertical lands them)

**I1 — framework + level scaffold + first Tier-A lints (thin vertical slice, DECIDED as first drop).**
- Extend `query.rs` `LintRule` with `name` + optional `fix: (Template, Applicability)`; add the
  `(clippy …)` form parser next to `compile_form` (`query.rs:3068`).
- Implement the lint-level layer: the `(allow/warn/deny NAME)` directive (registry + validation +
  CDZ0601 reuse) and the `--allow/--warn/--deny` CLI flags; the resolution order in §3.
- Add the `cdz clippy` subcommand (`cadenza-syntax/src/cli.rs` `enum Cmd` :63; `run_clippy` beside
  `run_lint`; flatten into `cdz`), reporting warnings and, with `--fix`, applying Verified fixes via the
  existing `cdz/fix.rs` path.
- Land the first 3 Tier-A lints WITH fixes: `idiomatic/if-bool`, `idiomatic/redundant-let`,
  `idiomatic/single-arm-match`.
- **Gate:** unit tests per lint (a match-and-fix round-trip: source → warning fires → `--fix` →
  expected source; a `--allow` suppresses it; a `--deny` fails). `cdz clippy --fix` output re-parses
  and re-lints clean (idempotent). `cargo xtask check` clean. No corpus/gate-baseline flips (clippy is
  additive, off the compile path).

**I2 — the rest of the Tier-A catalog.**
- `idiomatic/if-same-branch`, `idiomatic/double-negation`, `idiomatic/negated-eq`, `idiomatic/nested-
  match` (Heuristic). Each: rule + fix + tests. Grow the catalog as coherent per-lint (or small-group)
  units.

**I3 — Tier B (type-directed) lints in `rcdzc`.**
- A `clippy` module in `rcdzc` that runs after infer, consults `TypeOf`/`UsesOf` (the sidecar columns),
  and emits `diag::Diagnostic` + `diag::Fix`. New `Code` entries in the CDZ03xx code-quality band
  (alongside `UnusedBinding`/`DiscardedValue`/`UnreachableBranch`).
- First lints: `idiomatic/bool-compare`, `idiomatic/option-map`.
- **Gate:** reject/fix tests in `rcdzc/src/tests.rs`; the type guard verified (an overloaded `==` does
  NOT fire `bool-compare`); ABI `DiagnosticFix` round-trips; `cdz-wasm` surfaces the fix.

**I4 (design, later) — lints as Cadenza `drive` programs (rung 3).**
- Once compiler-ml self-hosts and `Query::Rewrite` lands in `rcdzc/src/sidecar.rs`, a lint becomes an
  ordinary Cadenza `drive : Ast -> List Request` program. The catalog migrates from native rules to
  Cadenza source. Blocked today; recorded so the earlier rungs don't paint into a corner (the paired-
  rule shape maps 1:1 onto a `Rewrite` request).

---

## 5. Seams / file anchors

- `cadenza-syntax/src/query.rs:3021` `mod lint` — extend `LintRule`, add `(clippy …)` form.
- `cadenza-syntax/src/query.rs` `Template`/`rewrite` — the fix substrate.
- `cadenza-syntax/src/cli.rs:63` `enum Cmd`; add `Clippy`; `run_clippy` beside `run_lint`.
- `cdz/src/fix.rs:24` `apply_fix_to_source` — the fix applier (reuse verbatim).
- `cdz/src/main.rs` — flatten `cdz clippy`.
- `rcdzc/src/diag.rs:53` `enum Code` (add Tier-B codes), `:506` `struct Fix`, `:519` `enum Edit`.
- `rcdzc/src/sidecar.rs:60` — `TypeOf`/`UsesOf` columns for Tier B; `Query::Rewrite` seam for rung 3.
- `rcdzc/src/db.rs:6200` `TOP_LEVEL_KEYWORDS` + `compile.rs:767/:1584` — the directive registry the
  `(allow/warn/deny)` control extends; `diag.rs:337` `UnknownDirective` (CDZ0601) reused for bad names.
- `rcdzc-wasm/src/lib.rs`, `cdz-wasm/src/lib.rs` — the wasm execution paths (no change needed).

## 6. Soundness & the gate

- A Verified fix MUST be meaning-preserving for every input it matches; a fix that isn't provably so is
  Heuristic (offered, not auto-applied). `cdz clippy --fix` output MUST re-lint clean and re-parse
  (idempotence is a gate test per lint).
- clippy is OFF the compile path: it introduces no corpus/gate-baseline flips. The gate is per-lint
  unit tests + `cargo xtask check` + (Tier B) `rcdzc` reject/fix tests + a `cdz-wasm` surfacing test.
- Tier B lints MUST guard on the exact type they assume (reject on mismatch), so overloading/shadowing
  never mis-fires — this is the whole point of type-directed.

## 7. Territory & coordination

- A new `vertical` owns cadenza-clippy top-to-bottom (area straddles `cadenza-syntax`/`cdz` for Tier A
  and `rcdzc` for Tier B). Tier A touches only clippy-owned surface; Tier B adds `Code` entries + a
  `clippy` module in `rcdzc` — coordinate with `v-inference` (owns infer/resolve/the columns) before
  reading new columns, and with the diagnostics workstream on the CDZ03xx code allocation.
- Overlap to respect: `cadenza-syntax/src/match_to_let.rs` (the existing normalize codemod) is the
  precedent for `idiomatic/single-arm-match`; reuse it, don't duplicate.

## 8. Decisions (resolved with the operator) & open questions

**Decided:** two-tier rung ladder (syntactic in `cadenza-syntax`, type-directed in `rcdzc`, self-hosted
`drive` later) · paired rule (lint + optional structural fix, applied separately) · lint levels YES
(module directive + CLI override, namespaced names) · first increment = thin vertical slice (framework +
level scaffold + 3 Tier-A lints end-to-end with fixes).

**Open (chosen defaults):**
- Fix conflict resolution when two lints touch overlapping spans in one `--fix` pass. *Default:* apply
  bottom-up, non-overlapping (the `apply_edits` skip-on-overlap rule at `query.rs:3003` already does
  this); a second `cdz clippy --fix` pass catches the rest (idempotence covers it).
- Whether `--deny`-promoted clippy warnings should gate CI. *Default:* clippy stays advisory (its own
  subcommand, non-zero only under `--deny`); wiring it into the fleet gate is a separate operator call.
- Group-level names (`--allow idiomatic`). *Default:* support a bare group prefix as "all lints under
  it"; finer taxonomy deferred until the catalog is larger.

## 9. Fold into the frozen contract

Additive w.r.t. `spec/contracts/`: clippy warnings are ordinary diagnostics (already have stable codes,
severity, and machine-actionable fixes per `spec/capabilities/diagnostics.md` + `constitution.md` §XI).
The `(allow/warn/deny)` directive extends the module-directive registry (`spec/capabilities/modules-
and-namespaces.md`) — a new key, validated like the existing ones. `cdz clippy` is a new tooling
subcommand under `spec/capabilities/tooling-and-lsp.md` + `spec/contracts/build-tool-interface.md`. No
change to the compile ABI or the runtime hash.
