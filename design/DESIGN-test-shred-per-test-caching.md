# Per-`@test` nix caching for the Cadenza test gate (test-shred)

Status: LANDED / EVOLVING (v-test-shred, 2026-08-29). The matrix is live on main (iterators standalone,
360/360); cad/choreography standalone + the arm-drops are in flight (contention-gated). See "Status
(LANDED)" at the bottom for the current state and "Scale + the per-suite mode split" for how the original
single-model idea below evolved into the shipped standalone/two-stage per-suite modes. Goal: replace the coarse per-PROJECT
`cdz test` gate derivations (`testCadenzaProject` / the `cad-tests` aggregate over
`implementation/{cad,compiler-ml,choreography,iterators}`) with a per-`@test` content-addressed matrix,
so an unrelated change is a cache hit and each `@test` runs in parallel — **without any persistent
in-process JIT daemon**. This is the last blocker to dropping the `cdz-run` library dependency from `cdz`
(so wasmtime leaves the seedCompiler fan-out — the headline "wasmtime-out-of-seedCompiler" win). It
mirrors the corpus per-case caching graph (`DESIGN-corpus-nix-per-case-caching.md`).

## Why

Today each of the 4 in-tree Cadenza projects runs as ONE nix derivation that shells `cdz test <project>`,
which compiles a test component from the project's `@test`-marked defs and runs each **in-process** via
the `cdz-run` library (a JIT-provider cache reused across trials — the operator's exact "no persistent
daemon" concern). Two problems: (1) `cdz` links `cdz-run` (hence wasmtime) for this; (2) any change to a
project re-runs its WHOLE suite. Shredding to per-`@test` content-addressed derivations fixes both: the
run goes through the `cdz-run` BINARY (exec, not link), and a `@test` re-runs only when its own emitted
wasm changes.

## The idea — the compiler shreds via a query

> **NOTE (superseded shape):** this section describes the original single model — a shared MAIN target +
> per-test CONSUMERS linked via a COMPONENT `--peer` compose. What SHIPPED generalized this into a per-suite
> `mode` (see "Scale + the per-suite mode split"): **standalone** emits self-contained per-test wasm (no
> main, no `--peer`) and **two-stage** keeps the shared-closure + per-test-fragment idea but splices via
> `cdz-compile … --export` rather than a runtime `--peer` compose. The manifest + `cdz-run`-binary exec +
> aggregate structure below are accurate; the "main + `--peer` consumer" specifics are the two-stage lineage.

Per operator refinement (2026-08-29): **the compiler does the shredding.** Given a project, `cdz-compile`
(driven by a query/mode) emits a **MAIN-target artifact + one target per `@test`**, where each per-test
target LINKS against the main target and CALLS its test function. This is compiler-driven shredding, not
nix-orchestrated-from-a-manifest.

```
project ─shred(query)─▶ main.wasm            ─exec─▶ pass/fail ─▶ per-project ─▶ cad-tests
        (cdz-compile)   + test-<k>.wasm/@test  (cdz-run BINARY,   aggregate      aggregate
        COMPILE-only     + manifest (binary)    NO compiler,
        wasmtime-FREE                           NO daemon)
```

- **shred** — ONE command, one content-addressed derivation per project; closure = `cdz` + `cdz-compile`
  + `CDZ_COMPILE_BIN` (the emit compiles in-process in `cdz-compile`, which `cdz` spawns); wasmtime-FREE
  (compile-only, no `--store`). `cdz test --emit-shred <project> --out-dir D` emits, flat:
  - `D/main.wasm` — the shared-closure PROVIDER (the project's library; the `@test` functions live here).
    The expensive shared-closure codegen (~215s for compiler-ml's ~1360-def closure) happens ONCE here.
    (= the Option-C `compute_shared_closure_provider`.)
  - `D/test-<k>.wasm` — one THIN per-`@test` CONSUMER that imports `main`'s interface and calls `@test_k`.
    (= `compute_tests_consumer(db, &[test_k], …)` — a per-`@test` bucketing of the proven `EmitTestsComposed`
    machinery, which today buckets per-FILE.) A change to one `@test` re-emits only its target; `main` is
    the shared CA dependency.
  - `D/manifest.bin` — cadenza-ast BINARY (operator directive "no json, cadenza-ast binary everywhere"):
    per `@test` `{ name (raw db def name = the `--list` identity + the derivation-name/drift key),
    export (the wasm boundary export symbol the grader `--call`s — may be kebab-cased, so ≠ raw name),
    target (test-`<k>.wasm`), main-iface (the `--peer` interface name), is_property }`.
  (`--list` is a SEPARATE lighter mode — enumeration only, no wasm — for the eval-time name index.)
- **exec** (one derivation per `@test`; closure = the COMPILER-FREE `cdz-run` binary + the runtime store):
  `cdz-run <test-<k>.wasm> --peer <main-iface>=<main.wasm> --store <store>` — a COMPONENT `--peer` compose
  (the consumer binds `main`'s exported interface) — → grade by **EXIT CODE** (clean return = PASS, trap =
  FAIL — a `@test` has no expected value, matching the in-process `cdz test` contract). `is_property` tests
  are SKIPPED in v1 (see below).
- **aggregate**: per-project (all its `@test` execs) → the `cad-tests` required context (name unchanged so
  the branch ruleset needs no edit), plus the compiler-ml aggregate folded into `localGate` (the
  Core-shape spine guard).

Caching: the shred is `__contentAddressed`, so a COMPILER change that leaves `main.wasm`/`test-<k>.wasm`
byte-identical yields the same output paths and every exec cache-hits — the nix CA cache REPLACES the
in-process JIT/provider cache. That is the operator's "heavily cached on no changes to the emitted wasm."

## Enumeration — compiler-informed discovery via a scoped, cached IFD (LANDED)

Per-`@test` **content-addressed exec derivations require the `@test` list at EVAL time** (to `genList`/map
N derivations). Two hard constraints collide: nix **cannot decode cadenza-ast binary at eval** (there is
no `builtins.fromJSON` equivalent for binary), and the global **no-IFD convention** means nix eval must not
trigger builds. An earlier resolution shipped a committed PLAIN-TEXT index — but the operator VETOED any
committed hard-coded test list ("what if someone adds a test and forgets to include it — massive pain"):
discovery must be COMPILER-INFORMED (`db.test_defs`), not a committed artifact or a source text-scan.

Landed resolution (operator OK'd IFD for this one scoped use; concierge greenlit scoped-cached-IFD): a
`testDiscovery` derivation runs `cdz test --list --format nix <proj>` (#5461) → `$out` = a SORTED, PURE,
importable nix list `[{ name; is_property; file } …]`. The flake **`import`s `$out` at EVAL** to fan out
the per-`@test` derivations (`testShredIndexEntries = import (testDiscovery …)`). This is IFD, but SCOPED
to discovery ONLY (the global no-IFD convention otherwise stands); nix caches the drv output, so eval
re-reads only when the suite source changes (rotates the drv), not on every eval. It is compiler-
authoritative (no committed index, no text-scan) and eval-readable. Reversible to a pure dynamic-derivation
on a future nix upgrade. The committed `tests-shred-index.txt` was built (#5298) then REMOVED (#5477) once
discovery landed (#5473). Keyed by `(file-stem, name)` — an `@test` name repeats across a suite's files
(iterators has 20 such), and the stem disambiguates + matches the manifest's `file` field.

## Property `@tests` — v1 deferral (safe for the spine)

A property `@test` takes parameters (or is a synthesized `-gen` `Test.gen` wrapper) and runs many trials
with generated inputs. v1 SKIPS these (`is_property` in the manifest). This is SAFE for the `localGate`
compiler-ml spine: the only property `@tests` in the 4 gate suites live in `compiler-ml/tests-example.cdz`,
which is OUTSIDE `src/` and therefore NOT in the suite (`def tests = ["src/*.cdz"]`). cad/choreography/
iterators have none. v2 will feed generated inputs via `cdz-run --host-response` (moving trial generation
to emit/build time).

## Division of labor

- **v-test-shred**: the shred DESIGN + coverage AUDITING (per-suite standalone-vs-two-stage coverage
  measurement, the hollow-green ritual = emit-N == authoritative + zero-skip), and the ARM-DROP specs (per
  suite, rewire the required `cad-tests` aggregate to depend on `test-shred-<suite>` instead of the coarse
  `cad-test-<suite>`, keeping the aggregate NAME so no ruleset edit). Additive-then-retire, per suite.
- **v-cdz-crate-split**: the `cdz-compile` query/subcommand surface — the emit-shred modes (`--standalone`
  monomorphized-per-test, `--two-stage` shared-closure + per-test fragment splice) + the binary manifest +
  the `--list --format nix` discovery projection, and (downstream) dropping `cdz-run`'s lib dep from `cdz`.
- **v-nix**: the flake mechanism (single-writer of the shred matrix — `mkTestShred`/`mkTestExec`/
  `testShredSuiteChecks`/`mkTestShredSuiteAgg`, per-suite `mode`, the `testDiscovery` scoped-cached-IFD,
  and the `cad-tests` aggregate rewiring); builds the emitted targets; owns the CA-build patterns.

## Scale + the per-suite mode split (standalone vs two-stage)

Authoritative `@test` counts (from `db.test_defs`, not a source regex — #5196): compiler-ml 854, cad 138,
choreography 177, iterators 360 = **~1529 `@tests`**, all `is_property = false`.

Two emit modes, chosen PER SUITE (`testShredSuites.<suite>.mode`):
- **standalone** — monomorphize the WHOLE closure per `@test` → each `test-<name>.wasm` is self-contained;
  `mkTestExec` runs it directly via the `cdz-run` binary (no splice). FULL coverage (no `emit_fragment`
  gaps), and a manifest-MISSING entry HARD-FAILS (the full suite emits, so absence = real drift — this
  structurally kills the hollow-green trap: a green aggregate genuinely means every `@test` ran). Right for
  SMALL-closure suites where per-test monomorphization is cheap. Measured full: **iterators 360/360, cad
  138/138, choreography 177/177** — all retire-ready, collision-safe (same-name cross-file tests get a
  numeric-suffix target basename).
- **two-stage** — emit the shared closure ONCE (`emit_fragment`, CA-cached) + a thin per-test fragment,
  spliced via `cdz-compile … --export`. O(closure + tests×body) instead of O(tests×closure) — required for
  HEAVY suites (compiler-ml's ~1360-def / ~215s closure makes standalone cost-prohibitive). BUT
  `emit_fragment` has cadenza-backend re-emit gaps (higher-order params, nested sum projections), so its
  coverage is PARTIAL and a decliner SKIPS. compiler-ml is a LAYERED PEEL: each backend fix lets defs
  progress and exposes deeper classes (1392 payload-projection → newtype-read → generic/open `(type …)`
  emission …); coverage climbs incrementally (64 → 80 → …), not in one jump. compiler-ml stays COARSE
  (its `cad-test-compiler-ml` arm) until two-stage coverage is ~full.

## Status (LANDED — 2026-08-29)

The matrix is LIVE on main and the discovery mechanism is settled:
1. ✅ **Compiler-informed discovery** — `cdz test --list --format nix` (#5461) + the `testDiscovery`
   scoped-cached-IFD (#5473); the committed `tests-shred-index.txt` was built (#5298) then REMOVED (#5477).
   Closes the operator's banned-committed-list concern for iterators.
2. ✅ **iterators shred matrix GREEN on main** — first as two-stage (#5473; discovered to be a hollow 56/360
   — two-stage `emit_fragment` can't lower higher-order params), then corrected to `mode=standalone`
   (#5530) → 360/360 REAL, zero-skip, hard-fail-on-missing. The audit lesson: a green shred aggregate does
   NOT prove coverage unless emit-N == authoritative AND zero-skip (standalone enforces this structurally).
3. ✅ **Per-suite mode** — `mkTestShred`/`mkTestExec`/… take `mode = standalone | two-stage` (#5530);
   `testShredSuites` entries are `{ dir; mode }`.

Remaining (in flight, contention-gated on nix builder starvation):
- (a) **iterators arm-drop** — rewire `cad-tests` to depend on `test-shred-iterators` instead of the coarse
  `cad-test-iterators` (spec delivered + re-audited GREEN; v-nix executes as single-writer in a calm window).
- (b) **cad + choreography standalone** — wired `mode=standalone` (v-nix branch, gate-pending), then their
  arm-drops (each 138/138, 177/177, retire-ready). After these, in-process `cdz test` runs ONLY compiler-ml.
- (c) **compiler-ml** — stays COARSE; two-stage coverage climbs as v-cadenza peels the backend re-emit
  layers on general merits (not a test-shred blocker). Retiring its coarse arm (→ dropping `cdz-run`'s lib
  dep from `cdz`, the headline win) waits on ~full two-stage coverage.
