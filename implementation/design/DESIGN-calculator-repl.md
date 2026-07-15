# DESIGN — A Cadenza calculator: REPL, CLI, GUI, and a Command+Space experience

*2026-07-14. Operator directive: "now that we have dimensional analysis and rationals and bigints I want
to build a calculator CLI and GUI … a REPL with forced rational numbers by default … ideally you'd be
able to assign values to variables and recall them … also cool if we had this on GitHub Pages … can you
survey everything, write a design/plan doc?"*

> **STATUS (2026-07-14) — operator decisions locked in:**
> 1. **Forced rationals go through the COMPILER, not a front-end shim.** Lead with a proper
>    `default-fraction`/`default-real Rational` directive (§5 Option B / increment C6), coordinated with
>    the numeric track — NOT the front-end literal-rewrite (Option A / C3). So Rational-by-default is a
>    **prerequisite the calculator waits on**, and C3 is dropped from the near-term path (kept only as a
>    documented fallback). The calculator engine (C1/C2/C4) can still be built without it; it just won't
>    default literals to Rational until the pragma lands.
> 2. **Land this design doc now; do not build yet.** Implementation (C1→C6) is queued, not started.

This is the survey + design + increment plan. It leans hard on one finding: **the evaluation engine
already exists** (the guide's browser playground compiles-and-runs Cadenza entirely client-side, and the
native `cdz` + `cdz-run` do the same on the desktop). A calculator is not a new evaluator — it is a
**thin state + input layer** wrapped around the existing "accumulated definitions + one expression →
compile → run → render" primitive, plus honest work on two gaps (forced-rational defaulting, and the
macOS launcher story).

---

## §0 — Vision, and what it unblocks

A calculator that is *the language*, not a fake calculator grammar:

- You type `1/3 + 1/3 + 1/3` and get **`1`**, exactly — not `0.9999…` and not integer-division `0`.
  (This is the marquee: exact rationals are the reason to build it.)
- You type `1 km + 500 m` and get **`1500 m`** — dimensional analysis, checked, in the same box.
- You type `2^100` and get the full 31-digit integer — BigInt, no overflow.
- You assign `x = 5`, then later `x * x` gives `25`; `ans` recalls the last result.
- It runs in three places off **one engine**: a terminal REPL (`cdz calc`), the GitHub-Pages guide (a
  `/calculator` page reusing the playground's compile+run path), and a Command+Space-style launcher on
  macOS (a Raycast/Alfred extension — see §2 for why *not* Spotlight itself).

Everything above except "forced rationals by default" and the launcher packaging is **runnable today**
through the existing pipeline. The two exceptions are the real work; the rest is UI + a state layer.

---

## §1 — The one evaluation primitive (already built — this is the leverage)

There is exactly one thing a calculator needs from the language: *given a set of definitions the user has
built up, evaluate one more expression and render its value.* That primitive **already exists in two
forms**, both proven end-to-end:

### 1a. Browser — `cdz-wasm::repl_eval` (`implementation/seed/crates/cdz-wasm/src/lib.rs:439`)

```
repl_eval(buffer: &str, expr: &str, from: Surface) -> { component, diagnostics }
```

It parses `buffer` (the user's definitions — a `(do …)`/`(module …)`/bare form), drops its exports,
appends a synthesized nullary entry `(def (cdz-repl-eval) <expr>)`, exports *that*, compiles the whole
thing to a wasm **component**, and hands it back. The guide's playground already drives it:
`PlaygroundPage.replCall` (`guide/src/playground/PlaygroundPage.tsx:212`) calls `replEval(text, expr, srf)`
then runs the component through the **same jco path the Run button uses** (`guide/src/runner/client.ts`
→ `runWorker.ts`: `jco transpile` → instantiate → `make()`/`encode()` for compound results → `renderValue`
→ `renderSyntax` into the reader's surface). A scalar renders directly; a compound value (tuple, list,
**Rational**, Qty) rides the `value-bytes` path and renders as canonical text (`1/2`, `1500 m`, `(list …)`).

The wire is already surfaced to TS: `guide/src/compiler/client.ts` exports `replEval`, `definedNames`,
`renderValue`, `renderSyntax`, `compile`, `diagnostics`, `typeAt`, `semanticTokens`. The disposable run
worker has a 5s watchdog (`runner/client.ts:13`) so a runaway expression can't wedge the UI.

**Consequence:** the browser calculator is a *new page* that reuses `replEval` + the run worker verbatim.
No compiler change, no new wasm export, for the core loop.

### 1b. Native — `rcdzc::compile` + `cdz-run` (`implementation/seed/crates/cdz-run/src/lib.rs`)

`cdz-run::run_capturing(component_bytes, &RunOpts) -> (Outcome, observed_host_calls)` runs a compiled
component against the content-addressed runtime store and returns `Outcome::Value(text)` /
`Outcome::Trap(msg)`. `rcdzc::compile(&[Artifact], &[Target::Wasm])` produces the component. So the exact
same "assemble module → compile → run → render" flow `repl_eval` does in wasm can be done **in-process in
Rust** — and there is currently **no native REPL** to do it (confirmed: `cdz`'s subcommands are
convert/query/compile/type/check/fix/… with no interactive mode; `cdz-run` runs a finished `.wasm`).

**The one duplication to avoid:** `repl_eval`'s module-assembly logic (unwrap the buffer's shell, drop
exports, append `(def (cdz-repl-eval) <expr>)`, export it) lives in `cdz-wasm`. The native REPL needs the
identical assembly. **Extract it once** into a surface-agnostic helper (see §6, C1) that both `cdz-wasm`
and the native `cdz calc` call — so the two REPLs can never drift in how they build the program.

---

## §2 — The three surfaces, and the honest macOS finding

### 2a. Native terminal REPL — `cdz calc` (recommended first build)

A new subcommand on the unified `cdz` binary (`implementation/seed/crates/cdz/src/main.rs`). It owns a
read-eval-print loop: read a line, classify it (assignment vs expression — §4), synthesize the module via
the shared assembler (§1b/C1), `rcdzc::compile`, `cdz-run::run`, print the rendered value. This is the
**engine everything else shells out to or mirrors**, and it's the cheapest to build and test (no
browser, no jco — native wasmtime via `cdz-run`). Ship this first.

⚠ `cdz calc` needs the runtime store populated (`cargo xtask build`) whenever a Rational/compound value
crosses the boundary — same content-address dependency `cdz-run` already has.

### 2b. GitHub Pages GUI — a `/calculator` route in the guide

The guide is already a Vite/React SPA deployed to Pages (`.github/workflows/pages.yml`,
`browser-guide-jco-execution-path` memory), with a full `/playground` and a self-contained `ReplPanel`
(input line + history + arrow-up recall + name completion, `guide/src/playground/ReplPanel.tsx`). A
calculator is a **stripped, opinionated playground**: no code editor, a big result display, a running
tape of `expr = result`, a variables panel. It reuses `replEval` + the run worker (§1a) and the
`ReplPanel` interaction model, adding the state layer (§4). New route alongside `/playground` in
`guide/src/main.tsx`. Deploys with zero CI change (Pages workflow already builds the guide + stages the
wasm).

### 2c. macOS "Command+Space" — the honest answer

**Spotlight itself is not extensible for custom computation.** Apple provides no public API to inject a
third-party calculator, evaluator, or custom result into the Command+Space Spotlight panel; the inline
calculator/units there are Apple-private, and the old Spotlight importer plugins only index metadata, they
don't run code. So "extend the Command+Space widget" as literally Spotlight is **not possible**. Don't
promise it.

The realistic, genuinely-good routes (in recommended order):

1. **Raycast extension** — best fit. Raycast is a Command+Space Spotlight *replacement* with a
   first-class extension API in **React + TypeScript/Node** — the same stack as the guide. A calculator
   extension shows a live result as you type in the Raycast bar. Two ways to evaluate:
   - shell out to the native `cdz calc --once "<expr>"` (a one-shot mode — trivial to add), or
   - bundle the `cdz-wasm` pkg and evaluate in-process (Node ≥20.19 for jco; heavier, but no native
     binary to ship).
   This is the closest thing to "your own calculator in Command+Space," and it's low effort once §2a
   exists.
2. **Alfred workflow** — Alfred is the other Command+Space replacement; workflows are shell-driven, so a
   workflow that pipes the query to `cdz calc --once` and shows the result is a ~1-file wrapper. Even
   easier than Raycast, less pretty.
3. **Standalone hotkey app** — a tiny menu-bar / floating-window app bound to a global hotkey (Tauri or
   SwiftUI), embedding the native engine. Most control, most packaging work. Only if 1–2 are too
   constraining.

**Recommendation:** build §2a (`cdz calc` + a `--once` one-shot), then a **Raycast extension** as the
Command+Space experience, with an Alfred workflow as a trivial alternative. Note the Spotlight limitation
up front so nobody expects native Spotlight integration.

---

## §3 — Decisions

1. **One engine, three shells.** The calculator is the `repl_eval` primitive (§1) + a state layer (§4).
   The native `cdz calc` is the reference engine; the Pages GUI reuses the wasm path; the macOS launcher
   shells out to `cdz calc --once` (or bundles the wasm). No surface reimplements evaluation.

2. **Extract the module-assembler once (C1).** `repl_eval`'s buffer-unwrap + entry-synthesis + export
   logic moves to a shared, surface-agnostic function so `cdz-wasm` and `cdz calc` share it byte-for-byte.
   This is the only refactor of existing code; everything else is additive.

3. **ML surface by default** (operator directive). The REPL reads/prints ML by default; a `--sexpr`
   flag / GUI toggle switches. `=` in ML is *equality* (`==` surface → `=` arena); a **binding** is
   `def x = 5` or `let x = 5 in …` (verified via round-trip). So the calculator's `x = 5` assignment
   sugar (§4) is a *calculator-front-end* convenience that desugars to `def x = 5` — it is **not** a new
   language form. Do NOT add an assignment operator to the language.

4. **Variables are an accumulated buffer of `def`s, keyed by name, last-write-wins** (§4). Recall works
   because every expression is re-compiled against the current buffer — the same reason the playground
   REPL sees the editor's defs. The state layer is pure UI/session state; no compiler or runtime change.

5. **Forced rationals is a real gap, addressed in a defined order** (§5). The landed `default-integer`
   pragma is **integer-only by spec** and will *not* make literals Rational. Ship a working calculator
   with a **front-end literal-rewrite** (annotate bare numerics as Rational) as the near-term mechanism,
   and pursue a proper `default-fraction`/`default-real Rational` directive as the clean long-term fix,
   coordinated with the numeric track.

6. **Rational/compound results render, they don't grade.** A Rational result crosses as a compound
   (`record{numerator,denominator}`) and renders fine in both the browser (`value-bytes` path, verified:
   `1/2`, `5/1`) and native (`cdz-run` render). The known jco *grading* limit (can't `==`-compare a
   compound result) is a test-harness issue, irrelevant to a calculator that only *displays* values.

---

## §4 — State model: variables and recall (the net-new layer)

The current playground REPL is **stateless across entries** — each `replCall` re-evaluates against the
editor buffer and stores nothing (`PlaygroundPage.tsx:212`, `ReplPanel` history is display-only). A
calculator needs accumulation. The model:

- **Session state = an ordered map `name → source-expression`** (the bindings the user has made), plus
  a running `history` of `(input, rendered-result)` for the tape.
- **Classify each input line:**
  - `name = expr` (a single identifier, then `=`/`==`, then an expression) → an **assignment**. Store
    `bindings[name] = expr` (last-write-wins), then evaluate `expr` against the *rest* of the buffer to
    echo its value.
  - anything else → an **expression**. Evaluate against the current buffer, display the result, and set
    the implicit `ans` binding to *this input's source* (so `ans` recalls the last computation and
    re-derives correctly).
- **Assemble the buffer** each turn as the surface program:
  `def a = <expr_a>  def b = <expr_b>  …  def ans = <last>` — i.e. serialize `bindings` to a `(do …)`
  of `def`s (topologically fine because later defs may reference earlier ones; the compiler resolves
  order). Pass that as the `buffer` arg to `repl_eval` / the native assembler, with the typed line as
  the `expr`. **Store the source expression, not the rendered value** — so `x = a + b` stays live if `a`
  changes, and there's never a render→re-parse round-trip (which would be lossy for a Rational rendered
  as `1/2` under integer-division re-parse).
- **Recall / editing affordances** already exist in `ReplPanel` (arrow-up history, name completion off
  `definedNames`). The variables map feeds `onNames`, so `x`/`ans` autocomplete.
- **Errors** (a decline, a trap, a redefinition that no longer type-checks) surface as the REPL's
  existing `error`/`trap` entry kinds; a failed assignment does **not** commit to `bindings`.

This is entirely a UI/session-state layer — **no compiler or runtime change**. The native REPL keeps the
same map in a `Vec<(String, String)>` / `IndexMap` and serializes identically.

⚠ Edge cases to handle in the classifier: distinguish assignment `x = …` from an equality *expression*
`a == b` (ML `==`); reject re-binding a prelude name shadowing that would confuse (or allow it — it's the
user's session); a bare `def …`/`let …` the user types explicitly should pass through as a binding too.

---

## §5 — "Forced rationals by default": the real gap, and how to close it

This is the one place the language doesn't yet do what the operator wants, so it needs a clear-eyed plan.

**The problem.** Bare integer literals are `Int64` and integer `/` **truncates**: `1/3` = `0`, so
`1/3 + 1/3 + 1/3` = `0`, not `1`. Exact arithmetic requires the operands to be `Rational`. The operator
expects "a pragma another agent is landing" to force this — but the pragma that **did** land
(`default-integer`, commit `498b6726`) is, by spec and implementation, **integer-only**: its domain
predicate is `Ty::Int | Ty::BigInt` (`compile.rs:403`), and `(pragma default-integer Rational)` is
rejected **CDZ0303** ("must name an integer type"). So the existing pragma will **not** give rationals.
That's the key honest finding: *don't assume the incoming pragma solves this.*

**What already works today** (so we can ship without waiting on the compiler):
- `Rational.of n d` and `Rational.of-int n` construct rationals; `+ - * /` on two Rationals are exact and
  `/` is total (`spec/semantics/06-numeric-model.sexp`).
- **Annotation grounds a literal to Rational** (landed `ac51a30f`): `(: 5 Rational)` → `5/1`,
  `(: 0.5 Rational)` → `1/2` exactly (decimal captured as significand·10^exp, no float rounding). That
  commit note also says it is "the grounding the forthcoming **`R` literal suffix** desugars onto" — so a
  `5R`/`1/2R`-style suffix is anticipated.
- Rationals compose with units: `(Qty Rational u)` is both dimensioned and exact
  (`spec/semantics/18-units-of-measure.sexp:386`).

**Option A — front-end literal rewrite (near-term, no compiler change; recommended to ship first).**
The calculator preprocesses the *input string* before handing it to the evaluator: tokenize, and wrap
every bare numeric literal `N` as `(: N Rational)` (s-expr) / annotate on the ML side (or emit the `R`
suffix once it exists). Then `1/3 + 1/3 + 1/3` becomes rational division → `1`. This is a small,
well-scoped pass on the calculator's own input (reusing `cadenza-syntax`'s lexer to find numeric-literal
token spans, so it never rewrites digits inside a symbol/string/identifier). It's a "Rational mode"
toggle: on by default (operator's wish), off to get raw Int64/Float behavior.
- *Risk:* getting the token classification exactly right (don't touch `#"sym3"`, string contents, unit
  names, exponents already handled by the lexer). Mitigate by operating on lexer tokens, not regex.

**Option B — a proper defaulting directive (clean long-term; a compiler increment).**
Add `(pragma default-fraction Rational)` (or `default-real`) — a sibling of `default-integer` — so a
bare, otherwise-unconstrained literal in a module carrying it defaults to `Rational`. The machinery is
mostly present: the literal→Rational grounding exists (`ac51a30f`), and `default-integer` already shows
the pattern (a load-time node→type map consulted by `infer`, commit `498b6726`). The new work is (1) a
spec section admitting a rational default (the existing one says "MUST name an integer type", so a
*separate* directive is the spec-honest move, not loosening `default-integer`), (2) the registry entry +
domain predicate, (3) the defaulting hook for the `Rational` case (including how `/` on two
defaulted-Rational literals selects rational division). Estimate: a contained increment on the numeric
track, gated 0-fail. **Coordinate with whoever owns the numeric pragma** — flag that `default-integer`
does *not* cover this and a `default-fraction` is what the calculator needs.

**Option C — the `R` literal suffix.** If the anticipated `R` suffix (per `ac51a30f`) lands, the
calculator's front-end rewrite (Option A) just appends `R` instead of wrapping in `(: … Rational)` —
cleaner output, same effect. Watch for it; adopt when present.

**Decision (operator, 2026-07-14): lead with Option B.** Forced-rational defaulting is done **in the
compiler** via a `default-fraction`/`default-real Rational` directive — the calculator waits on it rather
than shipping the front-end shim. Option A (the front-end rewrite) is **not** on the near-term path; it
stays documented only as a fallback if the pragma slips and a working fraction-calculator is needed
sooner. Adopt Option C's `R` suffix for cleaner output if it lands. So "Rational mode" is a language
capability, not a calculator-front-end concern — the calculator simply emits a module carrying the
directive once it exists.

---

## §6 — Increment plan (each a landable slice; gate 0-fail per step)

Ordered so a *usable* calculator exists as early as possible, engine-first.

- **C1 — Extract the shared REPL module-assembler (refactor, byte-neutral).** Lift `repl_eval`'s
  buffer-unwrap + `(def (cdz-repl-eval) <expr>)` synthesis + export-drop (`cdz-wasm/src/lib.rs`
  `buffer_items`/assembly, lines 350-517) into a surface-agnostic helper (likely in `cadenza-syntax` or a
  small shared module both `cdz-wasm` and `cdz` depend on), operating on `Arenas`. `cdz-wasm::repl_eval`
  becomes a thin caller. Prove byte-identical output (existing playground REPL behavior unchanged).

- **C2 — Native `cdz calc` REPL.** New subcommand: read a line → classify (§4) → assemble via C1 →
  `rcdzc::compile` → `cdz-run::run` → print. ML surface default, `--sexpr` flag. Variables map +
  `ans` + history in-process. Plus a **`--once "<expr>"`** one-shot mode (compute, print, exit) — the
  hook the macOS launcher and scripts use. This is the reference engine; test it hard (assignment,
  recall, redefine, error non-commit, trap, units, BigInt).

- **C3 — (DROPPED from the near-term path per operator decision — fallback only.)** The front-end
  literal-rewrite (Option A) — a lexer-driven pass wrapping bare numerics as Rational. Not built unless
  the C6 pragma slips and a working fraction-calculator is needed before then. Rational-by-default is
  instead delivered by C6 (below), which the calculator treats as a prerequisite.

- **C4 — Pages GUI: `/calculator` route.** A stripped playground: result display + tape + variables
  panel, reusing `replEval` + the run worker + `ReplPanel`'s interaction model, with the §4 state layer
  and the C3 rewrite (shared with C2 if the rewrite is exposed via a wasm export, or reimplemented small
  in TS against the same lexer via `cdz-wasm`). New route in `main.tsx`; deploys on the existing Pages
  workflow. Verify in a real browser (Playwright, per the guide's recipe): `1/3+1/3+1/3` → `1`,
  `x = 5` then `x*x` → `25`, `2^100` full digits, `1 km + 500 m` → `1500 m`.

- **C5 — macOS Command+Space: Raycast extension.** A React/TS Raycast extension that shells to
  `cdz calc --once` (or bundles the wasm pkg), live result as you type, "copy result" action. Document
  the Spotlight limitation; provide an **Alfred workflow** as a trivial shell alternative in the same PR.

- **C6 — `default-fraction`/`default-real Rational` directive (the forced-rational mechanism, operator's
  chosen approach).** The proper compiler-side rational defaulting (§5 Option B), coordinated with the
  numeric track: a spec section admitting a rational default (a *separate* directive, since
  `default-integer`'s prose says "MUST name an integer type"), the registry entry + domain predicate, and
  the defaulting hook for the Rational case (including how `/` on two defaulted-Rational literals selects
  rational division). This is what makes the calculator a *fraction* calculator; the calculator emits a
  module carrying the directive once it exists. **Prerequisite for the "1/3+1/3+1/3 = 1" behavior** — the
  engine (C1/C2/C4) can ship first, but defaults to Int64 until C6 lands.

**Ordering rationale:** C1→C2 gives a working native units/BigInt calculator with variables and **zero**
browser/macOS dependency. C4 puts it on Pages, C5 is the Command+Space experience. **C6 is the
forced-rational unblock** (operator's compiler-first choice) and can proceed in parallel on the numeric
track — the calculator's core loop doesn't block on it, but the *rational-by-default* feel does. (C3, the
front-end shim, is dropped; kept only as a documented fallback.)

---

## §7 — Reusable vs. net-new (at a glance)

| Piece | Status |
|---|---|
| Compile + run one expression against a buffer of defs | ✅ exists — `repl_eval` (wasm), `rcdzc::compile`+`cdz-run` (native) |
| jco browser instantiation + compound/Rational render | ✅ exists — `runWorker.ts`, `renderValue`/`renderSyntax`, run watchdog |
| REPL input UI: history, arrow-up recall, name completion | ✅ exists — `ReplPanel.tsx` |
| Pages deploy + wasm build/stage | ✅ exists — `pages.yml`, `xtask guide-wasm`, `stage-wasm.mjs` |
| Rational construction + exact `/` + units composition + BigInt | ✅ exists — numeric model, `Qty Rational` |
| Literal→Rational grounding via annotation | ✅ exists — `ac51a30f` |
| Shared REPL module-assembler (one copy) | 🔨 C1 — extract from `cdz-wasm` |
| Native `cdz calc` REPL + `--once` | 🔨 C2 — net-new subcommand |
| Variable/`ans` accumulation + assignment classifier | 🔨 C2/C4 — net-new state layer (no compiler change) |
| Forced-rational default (front-end rewrite) | ⏸ C3 — dropped from near-term path (fallback only) |
| `/calculator` GUI page | 🔨 C4 — net-new route reusing playground pieces |
| Raycast extension + Alfred workflow | 🔨 C5 — net-new packaging |
| `default-fraction`/`default-real` pragma (forced-rational) | 🔨 C6 — compiler increment, numeric track (operator's chosen mechanism) |
| Native Spotlight (Command+Space) integration | ❌ not possible — no Apple API; use Raycast/Alfred |

---

## §8 — Risks / watch-items

- **Don't assume the incoming pragma forces rationals.** `default-integer` is integer-only (CDZ0303 on
  Rational). Coordinate on a `default-fraction` (§5/C6); ship Option A meanwhile.
- **`=` is equality, not assignment** in the language. The `x = 5` calculator sugar is front-end only;
  never add an assignment operator to Cadenza. Classifier must distinguish `x = expr` (assign) from
  `a == b` (compare).
- **Store binding *sources*, not rendered values.** A Rational renders `1/2`, but `1/2` re-parsed under
  integer division is `0`. Re-evaluate from source each turn (also keeps recall correct when a dependency
  changes).
- **Runtime store dependency (native).** `cdz calc` needs `cargo xtask build` to populate the
  content-addressed store before a Rational/compound result can cross; a stale/missing store is the usual
  "no runtime of content address … in the store" refusal.
- **Stale wasm pkg (browser).** The `guide/src/wasm/pkg` artifact is gitignored and easily stale — the
  recurring "errors in the playground" false alarm. Rebuild (`cargo xtask build && cargo xtask
  guide-wasm`) before blaming the calculator; on this box `wasm-pack 0.15.0`/`wasm-opt` has a known
  workaround (see `browser-guide-jco-execution-path` memory).
- **Literal-rewrite precision (C3).** Rewrite over *lexer tokens*, not text/regex, so digits inside
  symbols/strings/unit names/exponents are never touched.
- **Don't reimplement evaluation per surface.** Enforce C1 so `cdz calc` and the browser stay in lockstep
  on how a program is assembled.
- **Land in a worktree, CAS onto spec.** `spec` is checked out in the shared main tree; edit only in a
  `.claude/worktrees/` worktree and land via a guarded `update-ref` (per AGENTS.md).

---

## §9 — Summary

The calculator is not a new evaluator — it is a **state + input layer over an evaluation primitive that
already exists** (`repl_eval` in the browser, `rcdzc::compile`+`cdz-run` natively), plus honest work on
two gaps. Build engine-first: extract the shared module-assembler (C1), a native `cdz calc` REPL with
variables/`ans`/`--once` (C2), a Pages `/calculator` page reusing the playground's compile+run path and
`ReplPanel` (C4), and a **Raycast** extension (with an Alfred alternative) as the Command+Space
experience — because **Spotlight itself exposes no API** for custom computation (C5). Forced rationals
are delivered **in the compiler** (operator decision): a `default-fraction`/`default-real Rational`
directive (C6), coordinated with the numeric track — a *separate* directive, since the landed
`default-integer` pragma is integer-only and rejects Rational (CDZ0303). The front-end literal-rewrite
(C3) is dropped, kept only as a fallback. Every UI/state piece needs no compiler or runtime change; the
forced-rational feel is the one prerequisite the calculator waits on (C6).
