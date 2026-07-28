# mlrepro: self-host HANG on a nullary ctor-pattern match — likely rcdzc mutual-recursion monomorph loop

Confirmed on PURE TRUNK (b9eb90e14 and batch-#125 tip 4de31e4cb), on LANDED nullary→TSum code.
NOT caused by any unlanded v-compiler-ml stack (reproduced with those commits absent).

## Repro (a one-@test file)

```
import { run-src } from "sread-eval"
@test
def probe-one-ctor-pattern() =
  match run-src("(do (type Color (Red) (Green) (Blue)) (def (main) (match (Red) (Red 100) (Green 200) (_ 300))) (export main))") with
    | Option.Some(v) => (if v == 100 then unit else trap("wrong"))
    | Option.None(_) => trap("declined")
```

`cdz test <that file>` HANGS: zero PASS/FAIL output, runs >120s under LOW host load, in isolation.
The full `implementation/compiler-ml/src/sread-eval-sum.cdz` likewise hangs (never emits results).

## SHARPENED isolation (tick 417): only `cdz test`'s separate-component compile hangs

Refined the repro — the hang is SPECIFIC to `cdz test`, NOT plain compile or run:
- `cdz compile <a `def main` calling the same run-src program> -o x.wasm` → COMPLETES (wrote 439KB wasm, exit 0).
- `CDZ_RUN_TIMEOUT_SECS=10 cdz run x.wasm` → RUNS, returns 100 (correct!), exit 0.
- `CDZ_RUN_TIMEOUT_SECS=10 cdz test <same program as a @test>` → HANGS (exit 124, zero output).

So the ML compiles AND runs fine. `cdz test` compiles a SEPARATE TEST COMPONENT with the report/host-effect
machinery "compiled in ONLY here — a normal cdz compile never carries it" (per `cdz test --help`). THAT
compile path is where rcdzc loops. So the rcdzc bug is in the `cdz test` @test-harness / host-effect
component compilation of a program that transitively pulls in the read-nullary-ctor-pattern-arm ↔
read-match-arms mutual-recursion SCC — NOT the ordinary compile or run path.

## Diagnosis: host-side (rust rcdzc), not the ML run

- `cdz check <file>` COMPLETES (exit 0) — compilation/type-check is fine.
- `CDZ_RUN_TIMEOUT_SECS=20 cdz test <file>` does NOT trap — so the loop is NOT in the running wasm
  (a spinning ML run would hit the run deadline). The hang is host-side: rcdzc compiling the test for RUN.
- The trigger: `ba91ef0` (landed nullary→TSum) added `read-nullary-ctor-pattern-arm` to sread.cdz — a NEW
  mutually-recursive function (`read-nullary-ctor-pattern-arm` → `read-match-arms` → `read-nullary-ctor-
  pattern-arm`). This is a fresh mutual-recursion SCC in the reader. Very likely the SAME class as the
  batch-73 mutual-recursion bounce: rcdzc loops (monomorphization / SCC handling) on a new mutual-recursion
  SCC when compiling the self-host program that exercises it.

## Why the gate missed it

pr-sync notes this self-host hang is NOT caught by `gate --check` or the corpus gate — only
`cdz test implementation/compiler-ml` surfaces it. Batch #125 reported "38 green" landing this code, so
either the gate ran before it manifested or it's a monomorphization loop that now trips consistently.

## Ownership + ask

Host-compiler (rust rcdzc) issue, not fixable in the ML lane. Route to the rcdzc/rust-backend owner:
the loop is in compiling the `read-nullary-ctor-pattern-arm ↔ read-match-arms` mutual-recursion SCC.
v-compiler-ml is holding its ii-c2b-2 payload-argType stack (not resending — it'd bounce on the same
trunk hang) until the rcdzc loop is fixed.
