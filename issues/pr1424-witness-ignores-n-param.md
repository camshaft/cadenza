# PR #1424 review comment — rcdzc/src/tests.rs (v-memory-safety)

Mirrored from https://github.com/camshaft/cadenza/pull/1424 (PR: "[v-memory-safety] ed6eea1f8").

## Witness's `main` ignores its `n` param -> harness input not exercised (Copilot, tests.rs:5322) — test-clarity
> In the embedded source program, `main` declares an `n` parameter but ignores it by calling
> `(walk 2 ...)`. Since the Rust harness passes `Val::S64(2)` anyway, this makes the witness less
> clear and could mask accidental changes to the call site argument. Consider threading `n` into
> `walk` so the test actually depends on the supplied input.

`main(n)` hard-codes `(walk 2 ...)` instead of using `n`, so the harness-supplied `Val::S64(2)` is
inert — a change to the passed arg wouldn't affect the result. Thread `n` into `walk` so the witness
actually depends on the input (and a call-site arg change is caught).
