# The Cadenza compiler, written in Cadenza (ML surface)

A from-scratch port of the compiler into Cadenza itself, written in the **ML surface**, in *ideal
form* — the compiler you would write if the language were finished. The Rust reference compiler
(`implementation/seed/crates/rcdzc`) is the structural **guide**; this is not a transliteration but a
re-derivation in idiomatic Cadenza.

This is a deliberate **stress test of the language**. Where Cadenza cannot express something cleanly,
the rule is to **report the issue so it gets fixed** — either a fix landed in the seed `rcdzc`, or a
crisp repro filed — rather than contorting the code around a limitation. Friction found is a
deliverable.

## Toolchain

- Author `.cdz` files (ML surface). When unsure of syntax, generate the canonical form with
  `cdz convert <file>.sexp --from sexpr --to ml` — do not hand-transcribe nested `match`/patterns.
- **`cdz check file.cdz`** is the primary loop: every well-formedness fault as
  `file:line:col: severity [CODE]: message`, exit ≠ 0 on error. `--json` for structured output.
- To exercise the backend: `cdz convert file.cdz --to binary > file.bin && cdz compile file.bin -t wasm
  -o out.wasm` (compile is the full type-check + lowering).

## Project manifest + tests

`Project.cdz` is the project manifest, **written in Cadenza itself** — well-known top-level `def`s the
`cdz` binary reads (a def is the manifest; no new syntax, no per-command flags). A file-list entry may
be a literal name OR a **glob** (`*.cdz`, `src/*.cdz`, `**/x.cdz`):

```
def name    = "compiler-ml"
def modules = ["src/*.cdz"]      // library modules — a wildcard, so a new pass just drops into src/
def tests   = ["src/*.cdz"]      // modules whose @test defs form the suite
def exclude = []                 // files removed from the globs above (a demo/fixture to skip)
```

Tests use the built-in **`@test`** workflow (`TESTING.md`): mark a nullary def `@test`; it PASSES by
returning `unit`, FAILS by trapping (`trap("…")`, or the `assert`/`assert_eq`/`assert_ne` helpers,
carries the message). Run them:

```
cdz test                   # NO arg: search up from the cwd for the nearest Project.cdz (like cargo), run it
cdz test .                 # reads Project.cdz here, runs every @test in the declared `tests` modules
cdz test Project.cdz       # same, naming the manifest directly
cdz test src/infer-db.cdz  # one file's @test defs
cdz test --filter head     # only tests whose name contains "head"
```

`cdz test` with no argument walks UP from the current directory to the nearest `Project.cdz`. A
`tests`/`modules` glob expands against the manifest dir (path-sorted, deduped, `Project.cdz` never
matched); a matched file that also matches an `exclude` pattern is dropped. `cdz test <dir>` with no
manifest walks every source file under the dir. A `@test` never burdens a normal `cdz compile` (the
test defs are unexported → dead → dropped).

**`cdz test` FOLLOWS the import closure** (mirrors `cdz check`): a module whose `@test` imports a
sibling type/function links against it and runs — so a test can reuse another module's `Ty` etc. A
directory run runs each file's OWN tests (the entry-file filter keeps a shared imported library's tests
to that library's own run, never double-counted through an importer). Tests still live SAME-FILE with
the code they test (a cross-file test cannot yet construct a type whose variant shadows a *prelude* name
— see the `mlrepro-import-prelude-collision` queue item), so each module tests itself, but a test may now
freely IMPORT non-colliding names from a sibling.

## Structure (mirrors the rcdzc stages)

Source modules live under `src/`; `Project.cdz`, `README.md`, and `TESTING.md` sit at the top. (Language
issues this port finds are filed in the shared queue — see "Language issues found" below — not a private
`repros/` dir.) Every `src/` module carries same-file `@test`s (run the whole suite with `cdz test .`).

Current `src/` modules (34), grouped by role. The compiler is a QUERY-DB pipeline (rcdzc's `db.rs` model):
source → parse → resolve → infer → lower → emit, each a memoized COLUMN on a shared `Db`.

**Query-DB substrate**
- `db.cdz` — THE QUERY DB (Tier-2 rcdzc port of `db.rs`): the memoized-column store the pipeline threads.
- `db-state.cdz` — the Db STATE-EFFECT carrier (Db threaded as an effect, the operator-ruled form).
- `db-demand.cdz` — P1c the MEMOIZING PRODUCER (demand a column, fill-once) — the item-4 memo model.

**Pipeline columns (source → wasm)**
- `sread.cdz` — the S-EXPR SOURCE READER: program text → a `Tree` (the `run-src`/W4-differential front end).
- `parse-db.cdz` — the PARSE column: `Tok` token type + tokenizer/grammar → the arena `Tree`.
- `resolve-db.cdz` — the RESOLVE column: name → binding (the resolved fact per node).
- `infer-db.cdz` — the INFER column: the monomorphic HM type fact per node (deferred-int unify + `TFn` arrows).
- `lower-db.cdz` — the LOWER column: the target-neutral `Core` IR (11 variants: CNum/CVar/CBin/CLet/CIf/CCall/
  CFnRef/CFnRefVar/CCtor/CMatchSum/CMatchEnum).
- `eval-db.cdz` — the INTERPRETER: eval a `Core` term to a value (the oracle for the emit≡interpret differential).
- `emit-db.cdz` — the BACKEND SEAM + backends: `type Target` / `target-emit` / `emit-src-for`; the Wasm arm
  (`emit-wasm-module`) and the Text arm (`render-core` — a readable Core dump). `emit-src`/`emit-text` entries.
- `emit-rec-db.cdz` — the RECURSIVE-MODULE ASSEMBLER (multi-function wasm module emit), split from emit-db.
- `db-resolve.cdz` / `db-infer.cdz` / `db-lower.cdz` / `db-eval.cdz` — the P1d–P1g producer wiring that runs
  each column ON the Db (resolve/infer/lower/run-driver), reading the memoized upstream columns.

**Type layer (the live monomorphic HM)**
- `typed.cdz` — the pipeline's `Typed` type as a zero-import LEAF (the infer column's per-node type fact).
  Extracted here so ty-bridge + infer-db share it without a cycle.
- `ty.cdz` — Tier-1 the SOLVED-TYPE UNIVERSE (ported from rcdzc `ty.rs`): `Ty` with the Sign/Width lattice.
- `unify-ty.cdz` — unification over `ty.cdz`'s shared `Ty` (deferred-int grounds to a concrete width sibling).
- `ty-bridge.cdz` — the `Typed` ↔ `Ty` bridge (`typed.cdz`'s `Typed` ↔ the ty.cdz universe); one home for the conversion.

**Integer-width foundation (hardening item 2)**
- `int-type.cdz` — recognize an integer type NAME → `(signed, width)` (the `(: v Int8)` annotation front).
- `int-width.cdz` — width SEMANTICS: fits-width / wrap-to / checked-{add,sub,mul} / pow2 (rcdzc-faithful).

**Sum types (M2)**
- `sum-store.cdz` — the SUM-VALUE STORE: the heap representation for payload-carrying user sum constructors.

**End-to-end run pins (`run-src` / emit tests — the in-Cadenza dogfood suites)**
- `sread-eval.cdz` — the core SOURCE-IN pipeline integration test (source → value); `run-src`/`run-src-typed`.
- `sread-eval-fns.cdz` — user-function (compositional) e2e. `sread-eval-ho.cdz` — higher-order fns (HO-3).
- `sread-eval-match.cdz` — integer match. `sread-eval-sum.cdz` / `sread-eval-sum-payload.cdz` — user sum
  types (nullary + payload/deconstruction). `sread-eval-ann.cdz` — width-annotated literals.
  `sread-eval-params.cdz` — parameter shapes. `sread-eval-nonrec.cdz` — non-recursive frontier regressions.

**Conformance scoreboard (hard-coded — retirement proposed, see the queue report)**
- `conformance-db.cdz` (+ `conformance-db-cx.cdz` composition, `conformance-db-rel.cdz` relational) — a hand-maintained in-Cadenza scoreboard of
  ~100 integer programs with hard-coded expected values. REDUNDANT with the gate's differential-vs-rcdzc
  harness (`report_ml_conformance`, which runs the full shared corpus); retirement plan filed for operator go.


## Language issues found

Language issues found by this port live in the shared issue **queue**
(`.claude/fleet/queue/`, archived to `issues/` when resolved) as `mlrepro-*` entries. File a NEW finding
there (write `.claude/fleet/queue/mlrepro-<slug>.<ext>` and send `corpus-bugfix` an `issue`, or the owning
vertical a `note`) — do NOT add it here and do NOT create a file under a private `repros/` directory. One
pipeline for every repro: the queue for open findings, `issues/` for resolved ones.
