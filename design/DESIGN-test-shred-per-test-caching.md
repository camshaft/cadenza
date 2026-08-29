# Per-`@test` nix caching for the Cadenza test gate (test-shred)

Status: DESIGN (v-test-shred, 2026-08-29, operator-requested). Goal: replace the coarse per-PROJECT
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

## Enumeration — a committed plain-text index (no IFD, no JSON-as-data)

Per-`@test` **content-addressed exec derivations require the `@test` list at EVAL time** (to `genList`/map
N derivations). Two hard constraints collide: nix **cannot decode cadenza-ast binary at eval** (there is
no `builtins.fromJSON` equivalent for binary), and **IFD is banned** (`flake.nix`), so nix can neither
decode the authoritative binary manifest nor read a build output to fan out. The compiler-emits-per-test-
targets refinement is ORTHOGONAL to this — it shapes the build artifacts, not the eval fan-out.

Resolution (shared with v-nix's guide matrix, which hits the identical wall): keep a tiny **committed
PLAIN-TEXT enumeration index** (newline records, tab fields: `name\tis_property`) that nix reads via
`builtins.readFile` + `splitString` (NOT `fromJSON`). The FULL cadenza-ast BINARY manifest stays
authoritative and is decoded at BUILD time inside the exec derivation (no IFD there). A **drift-guard**
(mirroring `guideManifestDriftAssert`) asserts the committed text index equals a freshly-derived index
(decode the binary at build, compare) → loud red if they diverge.

The text index is not itself cadenza-ast binary, so it is a minimal **eval-time exception** to "binary
everywhere" — unavoidable since nix eval is text/JSON-only. This needs an operator nod (v-nix is carrying
that ask, covering both guide + `@test`). If rejected, the fallback is a coarser ONE-derivation-loops-all-
targets model (no per-`@test` CA caching or parallelism) — which contradicts "ca-derivation per test", so
per-test CA + a tiny text index is the recommended reading.

## Property `@tests` — v1 deferral (safe for the spine)

A property `@test` takes parameters (or is a synthesized `-gen` `Test.gen` wrapper) and runs many trials
with generated inputs. v1 SKIPS these (`is_property` in the manifest). This is SAFE for the `localGate`
compiler-ml spine: the only property `@tests` in the 4 gate suites live in `compiler-ml/tests-example.cdz`,
which is OUTSIDE `src/` and therefore NOT in the suite (`def tests = ["src/*.cdz"]`). cad/choreography/
iterators have none. v2 will feed generated inputs via `cdz-run --host-response` (moving trial generation
to emit/build time).

## Division of labor

- **v-test-shred**: the shred DESIGN + the nix ca-derivation matrix (`mkTestShred` → per-`@test`
  `mkTestExec` → per-project + `cad-tests` aggregates), the committed text index + drift-guard, and the
  atomic retirement of `testCadenzaProject`/`cad-tests`' in-process path (same PR as the replacement).
- **v-cdz-crate-split**: the `cdz-compile` query/subcommand surface — the "emit main + per-test targets"
  mode + the binary manifest + the enumeration query (`Query::TestList` → binary), and (downstream)
  reimplementing interactive `cdz test` to emit-shred + exec the `cdz-run` binary per test in parallel
  subprocesses, then dropping `cdz-run`'s lib dep from `cdz`.
- **v-nix**: the flake mechanism (builds the emitted targets; owns the CA-build + drift-assert patterns);
  the guide manifest migration to the same plain-text-index shape (which this mirrors).

## Scale

Authoritative `@test` counts (from `db.test_defs`, not a source regex — #5196): compiler-ml 854, cad 138,
choreography 177, iterators 360 = **~1529 `@tests`**, all `is_property = false`. So the matrix is ~1529
per-`@test` exec derivations (parallel + CA-cached) replacing the 4 coarse per-project derivations.

## Status (2026-08-29)

Design is APPROVED and the interface is settled. Formerly-open items, now resolved:
1. ✅ Plain-text eval-enumeration index — APPROVED by concierge (no-json reading at the text-only nix
   boundary; operator FYI'd, veto-only). Shared with v-nix's guide-manifest migration.
2. ✅ `cdz test --list` project-mode bug — FIXED (#5196 manifest_strings dual-read for the M2 native
   Ctor(List) head + #5193 as_name bridge). Works on all 4 gate projects.
3. ✅ Per-test target links main via COMPONENT `--peer` compose (main = provider, test-`<k>` = consumer);
   shred is ONE command (`cdz test --emit-shred`), `--list` separate — confirmed by v-cdz-crate-split (S6b,
   `DESIGN-cdz-plugin-dispatch.md`).

Remaining before the nix matrix + drift-guard land: (a) rcdzc `Query::TestList` → binary + the
`cdz test --list` binary output (v-inference), (b) `Request::EmitTestsShred` emit (v-cdz-crate-split,
reusing `compute_tests_consumer`), (c) v-nix's guide committed-text-index migration (this mirrors it).
Sequencing: `Query::TestList` first, then `EmitTestsShred`; v-test-shred wires `mkTestShred`/`mkTestExec`
+ the drift-guard + the atomic `testCadenzaProject` retirement the moment those land.
