# DESIGN — Compiler-directed generators for property tests over collection/compound types

*2026-07-15. Operator directive (via concierge): "build up the unit/property/fuzzing stuff, go ALL IN
using bolero … extend the generators to be COMPILER-DIRECTED for ALL COLLECTION TYPES." Approach locked
to **(A) compiler-synthesis** (concierge decision 2026-07-15; option B — a user-overridable `Gen`
interface — is a later layer if the operator wants extensibility).*

> **STATUS (2026-07-15):** design doc; approach (A) confirmed. Implementation queued as F1 increments
> below. Prereq awareness: the `Test.fail(String)`-+-value-heap decline (filed to corpus-bugfix) means
> the FIRST increments report failure via `trap`, not a `Test.fail` message — not a blocker.

## The finding that shapes everything: the engine already exists

`cdz test` already runs **property tests** — a parameterized `@test` is run `--trials` times with
generated inputs, shrunk to a minimal counterexample, replayable from `--seed`. Two routes coexist
(`cdz/src/main.rs`):

- **Boundary-arg route** (`param_generators` → `GenKind`): the runner generates a *scalar* value
  (Int/Bool/Float/Char) and passes it to `cdz-run` as `--arg` text. Limited to scalars because
  `cdz-run --arg` parses only scalar text from the command line. A compound param → `None` →
  "cannot generate inputs" FAIL.
- **Guest-side `Test.gen` route** (`run_gen_driven`, `GEN_OP_LABEL = "test.gen"`): the guest performs
  `Test.gen : Unit -> Int64` to pull raw ints from a **seeded pool**; `cdz test` answers each with
  `--host-response Test.gen=<n>` and **shrinks over the int pool**. This is bolero's Driver model — one
  int source, type-directed decoding builds the value. A test that pulls ≥1 gen int is detected as a
  property test; zero → a plain unit test.

**So collection generation is not a new engine — it is: for a `@test` with a compound parameter,
compiler-synthesize a nullary wrapper that BUILDS the compound value by performing `Test.gen`, then
calls the real test.** The existing pool + shrink + detection drive it unchanged, with **no boundary-ABI
change** (the wrapper is nullary; the compound value never crosses the boundary — it is built guest-side).

## The synthesis

For a `@test def p(xs: List Int64) = …`, synthesize (at load / in `compute_tests`, where the test export
plan is built) a nullary companion:

```
def p$gen() =
  host Test in
    (let xs = <gen:(List Int64)> in
     p(xs))
```

where `<gen:T>` is a type-directed expression built by recursing over `T`:

| Type `T`            | `<gen:T>` (guest expression built from `Test.gen`)                                  |
|---------------------|--------------------------------------------------------------------------------------|
| `Int^s_w`           | `Test.gen()` masked/wrapped to the width & signedness (reuse the scalar `GenKind` logic) |
| `Bool`              | `Test.gen()` low bit `== 0`                                                          |
| `Float N`           | `Test.gen()` bits → float (or a decimal decode)                                     |
| `Char`              | `Test.gen()` reduced into a valid scalar range                                      |
| `List a`            | `let n = <len from Test.gen, bounded> in` build `n` elements each `<gen:a>` via `List.push` |
| `Tuple(a, b, …)`    | `(tuple <gen:a> <gen:b> …)`                                                          |
| record `{f: a, …}`  | `(record (f <gen:a>) …)`                                                             |
| sum `A(a) \| B(b)`  | pick a variant by `Test.gen() % k`, build its payload `<gen:payload>`               |
| `Set a`             | build a `List a` then `Set.of`                                                       |
| `Map k v`           | build a `List (Tuple k v)` then fold `Map.insert`                                    |

The wrapper is exported (in `compute_tests`) in place of the parameterized `p`; the runner sees a
nullary test that pulls gen ints → routes to `run_gen_driven` → trials + pool-shrink. **Shrinking is over
the int pool** (a smaller pool → a shorter list / smaller elements) — emergent structural shrinking,
adequate for a first cut; first-class structural shrinking of the built value is a later refinement.

## Where it plugs in

- **Injection point:** `layout::compute_tests` (rcdzc/src/layout.rs:353), where each `@test`'s
  `export_params` currently declines a non-representable (compound) param. Instead of declining a
  compound param, synthesize the wrapper def (append AST nodes like `effects::synthesize` does — there's
  precedent for building ordinary nodes at load and recording their `StructId`), register it as an
  internal callable, and export the wrapper.
- **The `Test` effect** is the well-known one already carrying `gen`/`fail`; the wrapper's `host Test in
  …` delegation is exactly what the scalar property tests already emit.
- **cdz-side:** no change to `run_gen_driven`/the pool/shrink — the wrapper looks like any gen-driven
  test. `param_generators` still returns the scalar kinds for the boundary-arg route (kept for scalar
  tests, which need no wrapper); the compound case is handled by the synthesized wrapper being nullary.

## Increments

> **Synthesis target validated (2026-07-15).** The exact wrapper below compiles + runs (8 trials pass)
> as hand-written sexpr, so the synthesis has a proven goal:
> ```
> (do (effect Test (op gen (-> Unit Int64)))            ; synthesize if not already declared
>     (def (p (: xs (List Int64))) …)                   ; the user's test — kept as an ordinary callee
>     (@ test (def (p-gen) (host (Test)                 ; the synthesized nullary wrapper, @test-marked
>       (let ((x0 (Test.gen))) (let ((x1 (Test.gen)))   ; k gen-calls (fixed-length first)
>         (p (list x0 x1)))))))
>     (export p-gen))
> ```
> The wrapper must be `@test`-marked (so `test_defs` picks it up) + exported; the original compound-param
> `p` is NOT a test export (it becomes the wrapper's callee). `run_gen_driven` auto-detects the wrapper as
> a property test (it pulls `Test.gen` ints) and shrinks over the pool — no cdz-side change.

- **G1 — `List Int64`** (the smallest end-to-end slice): synthesize the wrapper for a single `List Int^`
  param; gate a `@test def p(xs: List Int64)` that passes (a true property, e.g. `List.len` bound) and
  one that fails with a reported counterexample (trap-based assert, per the `Test.fail`+heap constraint).
  Build **fixed-length first** (k gen-calls + a `(list …)` literal — no recursive `gen-list` helper to
  synthesize), then **variable length** (a bounded gen'd length + a recursive `List.push` builder).
- **G2 — element-type recursion:** `List Bool`, `List (List Int64)` — recurse `<gen:a>` into the element.
- **G3 — tuple & record:** `<gen>` over positional slots / named fields.
- **G4 — sum:** variant pick + payload gen (reuse the discriminant machinery).
- **G5 — Set / Map:** build-then-collect.
- **G6 (stretch) — structural shrinking:** shrink the built value's shape (drop a list element, shrink a
  field) rather than only the int pool. Likely wants the (B) `Gen` interface's structure.

## Gate

Each increment lands with a `cdz test` integration case (a passing property + a failing one with a
counterexample) and, where a fold-unit is possible, an rcdzc unit test for the synthesis. The behavior
witness is "a property test over `<type>` runs `--trials` times and shrinks a counterexample."

## Open questions / coordination

- **v-inference:** the synthesis reads each param's SOLVED type (`type_of` on the binder) — the same
  `export_params` uses. A generic `List a` where `a` is unresolved at the test site can't be generated
  (decline with an actionable message: "annotate the element type").
- **`Test.fail(String)` + heap decline** (filed to corpus-bugfix): until it composes, a heap-collection
  property reports failure via `trap`, not a message. G1–G5 use trap-based assert; the message path
  lights up when that host-ABI gap closes.
- **fuzzer agent:** shares the bolero/int-pool model — coordinate if the pool/shrink infra is refactored.
- **(B) later:** a user-overridable `Gen a` capability (derived by the compiler, overridable per type)
  layers on top of this synthesis if the operator wants custom generators — the synthesis becomes the
  default derivation.
