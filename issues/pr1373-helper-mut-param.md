# PR #1373 review comment — cdz/tests/common/mod.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1373 (PR: "[v-cdz-tooling] a46510666").
Follow-on to the #1348 BrokenPipe-helper extraction.

## Unnecessary shadow to make `stdin` mutable (Copilot, common/mod.rs:20) — style nit
> The helper makes `stdin` mutable by shadowing (`let mut stdin = stdin;`). This is unnecessary and
> slightly harder to read; you can declare the parameter as `mut` instead and drop the extra binding.

Trivial: declare the param `mut stdin: …` and drop the `let mut stdin = stdin;` rebinding.
