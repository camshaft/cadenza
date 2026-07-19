# DESIGN: Running compound-parameter property tests in the browser harness

**Status:** proposed (spec by v-notebook; harness owner = v-guide-infra).
**Companion:** `DESIGN-property-test-collection-generators-rcdzc.md` (the compiler-side synthesis this builds on).

## Problem

A `<Runnable mode="test">` guide/notebook panel with a **compound-parameter** property `@test` — a
parameter that is a `(List T)`, `(Tuple …)`, `(Record …)`, `(Set …)`/`(Map …)`, a bounded user `sum`, or a
`Float` leaf — **cannot run live in the browser today**. The browser harness marks it `deferred`, so authors
fall back to prose + a static scalar-witness `Note` (the PropertyTesting-chapter convention). Native
`cdz test` runs these fine, so **browser ≠ CLI** for compound property tests.

## The blocker is a MISSING BROWSER DRIVER, not a wasm/ABI limit

The common framing — "a compound value has no boundary representation, so it can't cross wasm" — is
**incorrect**, and worth stating plainly because it made this look like a hard wall:

- **The compiler already solves the boundary by synthesis** (`rcdzc/src/proptest_gen.rs`). A compound-param
  `@test` such as `(@ test (def (p (: xs (List Int64))) BODY))` is rewritten into a **nullary**,
  `@test`-marked wrapper `p-gen` that builds the compound argument **entirely guest-side** by consuming a
  seeded integer pool via a host op `Test.gen : Unit -> Int64`:

  ```text
  (effect Test (op gen (-> Unit Int64)))              ; appended once
  (def (p (: xs (List Int64))) BODY)                  ; original, @test stripped
  (@ test (def (p-gen)                                ; synthesized nullary @test
    (host (Test)
      (let ((g0 ((. Test gen)))) (let ((g1 …)) … (p (list g0 g1 …)))))))
  ```

  The `<gen:T>` derivation is recursive over the parameter type (scalar = 1 int; `Bool` = parity of an int;
  `List` = a gen'd-length prefix; `Tuple`/`Record` = per-field; `Set`/`Map`; bounded `sum`; `Float` leaves
  via `float-of-int`). Increments G1–G9 have landed. **No compound value ever crosses the jco boundary** —
  the wrapper is nullary and produces its compound argument inside the guest.

- **`param_test_signatures` (cdz-wasm `lib.rs`) reports these wrappers `compound: true` with empty
  `param_types`** (the `-gen`-suffix branch). Both `check-examples.mjs`
  (`scalarSigs = sigs.filter(s => !s.compound)`) and the browser `runWorker` therefore skip them.

- **The actual gap: the browser runner never *answers* `Test.gen`.** `guide/src/runner/runWorker.ts`
  (~line 198) catches the unhandled-host-op error (`/test\.gen|no enclosing handler|unhandled|host op/`) and
  records the test as `deferred`. The **native** runner supplies `Test.gen` from a seeded int pool, runs
  `--trials` trials, and shrinks over the pool. The browser lacks only that driver.

**⇒ Closeable, and bounded.** No compiler or ABI change is required — the wrapper, the `Test` effect, and the
`compound:true` signal already exist. The work is a self-contained browser-harness enhancement.

## Design

Teach the browser test runner to execute a `compound: true` nullary wrapper the same way it already drives a
scalar-param property test — the only new piece is **supplying the `Test.gen` host op over a seeded int
pool** and **shrinking over that pool** rather than over call-args.

### Template: the existing scalar driver

The scalar path (`check-examples.mjs` `runScalarProps` ~L190–250; the browser twin `runScalarProperties` in
`runWorker.ts`) is the template:

1. A deterministic **LCG** produces a stream of ints from a seed
   (`lcg(s) = (s*6364136223846793005 + 1442695040888963407) & u64`).
2. Per trial `t` (1..=N, N≈100), derive inputs from `seed = t`, invoke the export, and treat a **trap as a
   failing trial**.
3. On a failure, **shrink** toward a minimal counterexample (halving each scalar toward 0 while the failure
   persists), then render the counterexample.

### The one new piece: a `Test.gen`-backed nullary driver

For a `compound: true` wrapper (nullary export, no call-args):

1. **Instantiate the wrapper component with a `Test.gen` import** that pulls the next int from a seeded LCG
   pool. Each `((. Test gen))` in the guest consumes one int; the wrapper is **deterministic in its gen-int
   sequence**, so a fixed pool ⇒ a fixed compound argument.
   - *Implementation note (v-guide-infra to confirm):* the `Test` effect lowers to a component **import**
     (an effectful host op), the same lowering `runWorker` already wires for the value-heap `heap` import
     and the `param` interface. The precise import instance/member name (WIT-kebab → jco-camel) should be
     read off the compiled wrapper's imports exactly as the `param`/`heap` wiring does (`normalizeName`
     matching), not hard-coded. A host op returning `Int64` binds as a JS function returning a `bigint`.
2. **Run N trials**, each with a fresh pool seed (`seed = t`), invoking the nullary wrapper. A trap = a
   failing trial (same contract as scalar).
3. **Shrink over the INT POOL, not the compound value.** Because the wrapper reconstructs its argument from
   the gen-int sequence, shrinking the *pool* (e.g. truncating trailing ints → shorter lists; halving
   individual pool ints toward 0 → smaller leaves) shrinks the *compound* deterministically. Re-run the
   wrapper with the shrunk pool; keep a shrink step iff the failure persists. This mirrors the native
   `shrink_pool` and needs **no compound-value introspection** in JS.
4. **Render the counterexample** by running the wrapper once more with the minimal pool and reading back the
   built value (the existing counterexample-render path), or — if reading the built compound back is
   awkward — report the minimal pool + the witness the wrapper produces, matching how the native runner
   surfaces it.

### Keep `check-examples.mjs` in lockstep

The gate (`check-examples.mjs`) must gain the same driver so a live compound-property panel is *gated*, not
just runnable. Today it emits `"a mode=\"test\" example has no runnable @test defs (no nullary, no scalar
property)"` for a compound-only test; after this it drives the `compound:true` wrapper via the shared pool
driver. This keeps the gate and the in-browser runner byte-for-byte in lockstep (the existing discipline).

## What this unblocks

Live `<Runnable mode="test">` **compound-property panels** — List/tuple/record/Set/Map/bounded-sum
generators — currently forced to prose + a static scalar-witness `Note`. In particular the PropertyTesting
chapter's compound-shrink demos (e.g. `never-three(xs: List Int64)` shrinking to `[0,0,0]`) could ship as
live panels instead of prose. Notebook `mode="test"` cells gain the same reach.

## One prerequisite: rename the `Test` op away from `gen` (jco name collision)

⚠ **Caveat to the "no ABI change" premise below.** v-guide-infra verified the substrate against staged wasm
and found that jco 0.4.2 emits its own top-level `let gen = (function* _initGenerator)`; for a component
importing an op literally named `gen`, jco also emits `const { gen } = imports.test` **in the same scope** →
a `SyntaxError` at module import (the component won't even load). So the browser cannot answer `Test.gen`
while the op is named `gen`. The prerequisite is a **trivial member rename** of the `Test` effect's op —
`gen` → `gen-int` / `draw-int` (v-property-testing owns this; a member rename, no ABI/shape change) — after
which everything below applies unchanged. Filed to v-property-testing; this driver work starts the moment the
renamed op ships in staged wasm. (Found by v-guide-infra during substrate confirmation — a real
implementation blocker a static AST trace can't surface.)

## Non-goals / out of scope

- **No compiler or ABI change** *for the driver itself* (the one prerequisite above — the `gen` op rename — is
  a trivial member rename, not an ABI/shape change). The `-gen` wrapper, the `Test` effect, and `compound:true`
  already exist.
- **No new generator logic.** All `<gen:T>` derivation is compiler-side (G1–G9). The browser only *feeds the
  int pool* and *shrinks it*.
- Char leaves and any not-yet-synthesized leaf types remain the compiler's follow-up
  (`DESIGN-property-test-collection-generators-rcdzc.md`), independent of this driver.

## Ownership

The fix lives entirely in v-guide-infra's harness (`runWorker.ts` + the `check-examples.mjs` gate twin).
v-notebook characterized the path and wrote this spec; v-guide-infra owns implementation. v-notebook (and
v-property-testing, who owns the compiler-side generators) are available to co-spec / review.
