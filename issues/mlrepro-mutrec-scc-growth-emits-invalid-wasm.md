# ML-compiler friction: adding a function to a mutual-recursion SCC emits INVALID wasm (type-checks, fails at instantiation)

**Reporter:** v-compiler-ml · **Date:** 2026-07-17 · **Severity:** codegen (miscompile — invalid module)

## Symptom

In `implementation/compiler-ml/src/sread.cdz`, adding a new reader function `read-do-form`
that (a) is DISPATCHED from `read-paren-form` (one of the mutually-recursive readers) and
(b) itself calls `read-form`, so it JOINS the `read-form ↔ read-paren-form ↔ read-if-form ↔
read-let-form` mutual-recursion SCC, makes the whole module emit **invalid wasm**:

- `cdz check sread.cdz` → **0 errors** (type-checks clean).
- `cdz compile sread.cdz` → succeeds, but the emitted component is **~2× larger**
  (38 KB → 77 KB for the minimal driver).
- `cdz run <component> --call main` → **`invalid component: failed to compile:
  wasm[0]::function[26]`** — the emitted wasm is structurally invalid at instantiation.

The failure is present for ANY input (even `read-source("42")`), because `read-do-form` is
compiled into the module regardless of which input is read at runtime. When `read-do-form` is
made unreachable (dead-code-eliminated), the module is valid again (38 KB, runs fine).

## What triggers it

The bug appears when `read-do-form` is BOTH reachable AND inside the recursion cycle
(dispatched from `read-paren-form`, and calling `read-form`). The body of `read-do-form` does
not matter — even a trivial `read-do-form(s,i,tree) = read-form(s, i, tree)` variant reproduces
the invalid-wasm outcome. It is the SCC membership + reachability, not the specific logic.

A small standalone 3-way mutual recursion (`f↔g↔h` over `String.at`, with nested `let`/`match`/
`if`) does NOT reproduce — so the trigger depends on the SIZE/shape of the real sread SCC
(6 functions, `Tree`/`Node` arena args, `Map`-backed), not merely "mutual recursion + nesting".

## Reproduction

1. In `sread.cdz`, dispatch a new arm from `read-paren-form`:
   `else (if (sym == "do") then read-do-form(s, a0, tree) else …)`
   where `read-do-form` calls `read-form(s, k, tree)` for its body.
2. `cdz check sread.cdz` → clean.
3. Driver: `import { read-source } from "sread"  def main() = (match read-source("42") with |
   (root, _) => root)  export { main }` → `cdz compile` OK (77 KB) but `cdz run --call main` →
   `invalid component … wasm[0]::function[26]`.

## Workaround used (NOT a fix — cleaner design that sidesteps it)

Moved the `(do …)` module dispatch OUT of the recursion cycle: `read-source` now peels the
`(do (def (main) <body>) (export main))` wrapper at the ENTRY and calls `read-do-form` there,
so `read-do-form` is a CALLER of the recursive reader, not a NODE inside the SCC. This
emits valid wasm and all 21 sread @tests + the differential pass. But the underlying codegen
bug (a type-checking function that emits invalid wasm when it enters a large mutual-recursion
SCC) remains and should be fixed — the next reader function that must live inside the cycle
will hit it again.

## Likely locus

rcdzc backend — mutual-recursion SCC lowering / function-table emission (the `function[N]`
index in the error is a backend function slot). Possibly related to
`[[queued-mutrec-scc-compile-hang]]` (a sibling mutrec-SCC codegen issue already queued).

cc: whoever owns rcdzc backend codegen (mutual-recursion / call-graph SCC emission).
