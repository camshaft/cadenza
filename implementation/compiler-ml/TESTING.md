# Unit testing with `@test` and `cdz test`

Cadenza has a built-in unit-test workflow: mark a definition `@test`, then run `cdz test FILE`. The tests
compile to a **separate** wasm component from the normal build — they never affect (or burden) the
component a plain `cdz compile` produces.

## Writing a test

A `@test` marks a **nullary** definition as a test (`tests-example.cdz` is a complete example):

```
@test def arithmetic_holds() =
  if 1 + 1 == 2 then
    unit
  else
    host report in (
      report.fail("1 + 1 should be 2");
      trap("assertion failed")
    )
```

- A test that **returns** (yields `unit`) **PASSES**.
- A test that **traps** **FAILS**. To carry a failure message, the test performs a `report` host effect
  with the message *before* trapping; `cdz test` surfaces it as `FAIL <name>: <message>`.

The `report` effect is any effect with a `String -> Unit` operation the test delegates to the host:

```
effect report =
  | fail : String -> Unit
```

## Running

```
cdz test tests-example.cdz            # run every @test, report pass/fail, exit non-zero if any failed
cdz test tests-example.cdz --filter holds   # only tests whose name contains "holds"
```

Output:

```
PASS arithmetic_holds
PASS comparison_holds
FAIL a_failing_example: expected 2 + 2 to be 5 (it is not — this test fails on purpose)

2 passed, 1 failed
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

A normal `cdz compile` of a program that ALSO contains `@test` defs does **not** carry the `report`
import: the test defs are unexported there, so they are dead and dropped, and the effect they perform is
never reached. The report effect is compiled in **only** for the `cdz test` artifact.

## Known rough edges (stress-test findings)

The port-to-Cadenza workstream treats friction as a deliverable. These are pre-existing limitations this
example steers around (not specific to `@test`):

- A leading `//` line comment or `///` doc comment before a top-level `effect` or a `@test def` is
  wrapped as `(comment …)` / `(doc …)` around the following form, which the compiler's top-level scan
  does not see through — so the def is hidden. Keep top-level comments off an `effect` / annotated def
  for now (a `def`/`type`/`module` consumes a leading `///` fine).
- A `host` delegation factored into a **helper** that is inlined into an entrypoint does not add its op
  to the component's host-import set ("a host call's operation is not in the host-import set"). Put the
  `host report in (…)` directly in each test body (as the example does) rather than in a shared helper.
