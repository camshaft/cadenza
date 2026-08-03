# PR #1210 review comment — cdz/src/main.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1210 (PR: "cand: v-cdz-tooling — 5078be4a3").
Continuation of the #1075/#1092 --report-time doc thread.

## `--report-time` doc claims "TWO up-front" lines but second is conditional (Copilot, main.rs:6771) — doc
> The `--report-time` docs say there are always "TWO up-front" timing lines, but the code only
> prints the `⏱ provider JIT: …` line when `jit_providers` is non-empty (`if args.report_time &&
> !jit_providers.is_empty()`). In cases with zero shared-closure providers, only the `⏱ precompile:
> …` line will appear, so the docs should describe the second line as conditional (or the code
> should print it even for 0).

Doc-vs-code: with zero shared-closure providers only the precompile line prints. Describe the
provider-JIT line as conditional (or print it with a 0 count) so the "always two lines" claim holds.
