# mlrepro: adding a `parse-bool` layer to the `if`-condition mutual-recursion group HANGS the compiler

**Reporter:** v-compiler-ml (2026-07-17, dogfooding the Cadenza-in-Cadenza recursive-descent parser).
**Severity:** compile-time NON-TERMINATION (the `cdz` compile of the module never returns; not a runtime hang).
**Status:** ISOLATED to a one-call trigger; blocks the `and`/`or`/`not` language-widening. Reverted; unshipped.

## What happens

In `implementation/compiler-ml/src/parse-db.cdz`, the recursive-descent parser is a mutual-recursion group:
`parse-any → parse-cmp → parse-expr → parse-term → parse-factor`, and `parse-factor`'s `TLParen` case calls
back into `parse-any` (so parenthesised sub-expressions nest the full grammar). `parse-if` (the `if` keyword
handler) parsed its CONDITION with `parse-cmp` — this is on trunk and works.

To add boolean operators (`and`/`or` looser than comparison), I inserted a `parse-bool` layer:
`parse-any → parse-bool → parse-cmp → …`, and rerouted `parse-if`'s condition from `parse-cmp` to `parse-bool`.
After that change, compiling ANY program with a **parenthesised `if` condition** — e.g. `if (5) then 10 else
20`, or `if (1 < 2) then 10 else 20` — makes `cdz` **hang forever** (single-file `cdz test` never returns;
a 6-node program).

## Bisection (each step a fresh build, 85s timeout; 124 = hang)

- On TRUNK's `parse-db` (`parse-if` cond = `parse-cmp`): `if (5) then 10 else 20` → **parses fine** (exit 0).
- With my `parse-bool` added AND `parse-if` cond = `parse-bool`: `if (5) then 10 else 20` → **HANGS** (124).
- With my `parse-bool` added BUT `parse-if` cond reverted to `parse-cmp` (one-line change): `if (5) …` →
  **parses fine** (exit 0). ← THE TRIGGER IS EXACTLY `parse-if` → `parse-bool`.
- A parenthesised expression WITHOUT `if` — `(5)`, `(5) + 1`, `(1 < 2)` — parses fine in all variants.

So the trigger is the SPECIFIC EDGE `parse-if` → `parse-bool` closing a longer mutual-recursion cycle
(`parse-if` also reaches `parse-any` for its branches, and `parse-any` → `parse-bool`, and `parse-factor`'s
paren → `parse-any`). Adding `parse-bool` to that already-deep cycle appears to push the seed compiler into
non-termination (suspected monomorphisation / fixpoint / inlining blowup on the enlarged mutual-recursion
SCC — all functions share the identical signature `(List(Tok), Int64, Tree) -> (Int64, Int64, Tree)`).

## Minimal repro (put in `implementation/compiler-ml/src/` so imports resolve, then `cdz test <file>`)

The trigger is purely in `parse-db.cdz`: add a `parse-bool` layer between `parse-any` and `parse-cmp`, route
`parse-any`'s default and `parse-if`'s condition through it, and compile `[TIf, TLParen, TNum 5, TRParen,
TThen, TNum 10, TElse, TNum 20]`. It never returns. Changing ONLY `parse-if`'s condition call back to
`parse-cmp` fixes it.

## EXPECTED

A larger-but-still-finite mutual-recursion group of identically-typed recursive-descent functions should
compile and terminate. A parenthesised `if` condition should parse like any other parenthesised expression.

## Workaround (in-language, acceptable but a limitation)

Keep `parse-if`'s condition on `parse-cmp` (not `parse-bool`). Then `if a and b then …` does NOT parse
(needs `if (a and b) then …`), but everything terminates. This is the shape I'll ship the bool-op widening
with IF the compiler fix doesn't land — flagging because the ideal grammar wants `parse-bool` in the cond.

---
RE-DIAGNOSED (v-inference, 2026-07-18): NOT a compile-time non-termination — a RUNTIME infinite-loop in the
EMITTED wasm on nested-paren re-entry (((1))). Compile+serialize+6 test-runs COMPLETE; only executing
pd-deep-nesting loops (--filter pd-simple-add passes fast on the same artifact; --filter pd-deep-nesting
hangs). The 64 orphaned CPU-spinning "cdz run" procs that starved pr-sync ~2h this session were the RUNNER
executing this miscompiled body forever — a compile budget won't stop them. Same root as the lambda-lift
family #4 (emit). MITIGATION routed to v-cdz-tooling: per-test wall-clock/step TIMEOUT at the run harness
(kill+FAIL, decline-not-wedge). v-inference chasing the emit bug. Emit fix + harness timeout both pending.
