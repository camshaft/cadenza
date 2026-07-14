# Unit testing with `@test` and `cdz test`

Cadenza has a built-in unit-test workflow: mark a definition `@test`, then run `cdz test FILE`. The tests
compile to a **separate** wasm component from the normal build — they never affect (or burden) the
component a plain `cdz compile` produces.

## Writing a test

A `@test` marks a **nullary** definition as a test. Write assertions with the `assert` / `assert_eq` /
`assert_ne` helpers, each taking an explicit failure message (`tests-example.cdz` is a complete example):

```
@test def arithmetic_holds() =
  assert_eq(1 + 1, 2, "1 + 1 should be 2")

@test def comparison_holds() =
  assert(3 > 2, "3 should be greater than 2")

@test def values_differ() =
  assert_ne(1, 2, "1 and 2 should differ")
```

- A test that **returns** (yields `unit`) **PASSES**.
- A test that **traps** **FAILS**. Each assertion, on failure, performs the well-known `Test` effect with
  the message *before* trapping, so `cdz test` surfaces it as `FAIL <name>: <message>`.

## The `Test` effect and the assertion helpers

Assertions delegate to a well-known `Test` effect whose `fail` operation carries the failure message to
the runner. Declare it and the helpers once per test file (a future revision will provide them
prelude-wide so no boilerplate is needed — see "Follow-ups"):

```
effect Test =
  | fail : String -> Unit

def assert(cond, msg: String) =
  if cond then unit
  else host Test in (Test.fail(msg); trap("assertion failed"))

def assert_eq(a, b, msg: String) =
  if a == b then unit
  else host Test in (Test.fail(msg); trap("assertion failed"))

def assert_ne(a, b, msg: String) =
  if a == b then host Test in (Test.fail(msg); trap("assertion failed"))
  else unit
```

`assert_eq` / `assert_ne` are **generic** — their `a`/`b` are unannotated, so one helper works over any
equatable type (`Int64`, `String`, a sum, …) via monomorphization.

## Running

```
cdz test tests-example.cdz            # run every @test, report pass/fail, exit non-zero if any failed
cdz test tests-example.cdz --filter holds   # only tests whose name contains "holds"
```

Output:

```
PASS arithmetic_holds
PASS comparison_holds
PASS values_differ
FAIL a_failing_example: expected 2 + 2 to be 5 (it is not — this test fails on purpose)

3 passed, 1 failed
```

`cdz test` compiles the test component in-process and shells out to the sibling `cdz-run` binary to
execute each test (both are built together under `target/<profile>/`).

## How it works

- `@test` is a general **attribute** — the ML sigil `@name form`, canonical `(@ name form)`. The compiler
  recognizes `test` and records the marked defs; they are otherwise ordinary defs.
- `cdz test` drives a compile with an `EmitTests` request, which lays the component boundary out from the
  `@test` **nullary** defs (each a no-argument entry) instead of the program's `(export …)` clauses.
- A failing test's body **diverges** (it traps after reporting), so it crosses the boundary as a
  no-result trapping entry; the runner grades a trap as a failure (a clean return is a pass).

## The single-artifact / no-host-burden guarantee

A normal `cdz compile` of a program that ALSO contains `@test` defs does **not** carry the `Test` import:
the test defs are unexported there, so they are dead and dropped, and the effect they perform is never
reached. The `Test` effect is compiled in **only** for the `cdz test` artifact.

## Follow-ups (not yet built)

- **Prelude `assert` + `Test`.** Today each test file declares the `Test` effect and the `assert*` helpers
  inline (cross-module `import` is not yet modeled, so a shared support module can't be imported). Once a
  prelude function with an emittable body + a well-known prelude `Test` effect land, these become built-in
  and the boilerplate disappears.
- **Value & expression printing.** `assert_eq`/`assert_ne` currently print only the author's message. A
  richer form (`FAIL: left = 2, right = 3`, and the operand source text) needs a `show : a -> String`
  value renderer (the compiler has one internally, not yet exposed in-guest) and a `stringify`
  metaprogramming primitive (none yet). Deferred.
- **Property testing.** The `Test` effect is designed to extend — a property runner adds operations
  (generation seed, shrink reporting) to `Test` rather than a new mechanism.

## Known rough edges (stress-test findings)

The port-to-Cadenza workstream treats friction as a deliverable:

- A leading `//` line comment or `///` doc comment before a top-level `effect` or a `@test def` is wrapped
  as `(comment …)` / `(doc …)` around the following form, which the compiler's top-level scan does not see
  through — so the def is hidden. Keep top-level comments off an `effect` / annotated def for now (a
  `def`/`type`/`module` consumes a leading `///` fine).
