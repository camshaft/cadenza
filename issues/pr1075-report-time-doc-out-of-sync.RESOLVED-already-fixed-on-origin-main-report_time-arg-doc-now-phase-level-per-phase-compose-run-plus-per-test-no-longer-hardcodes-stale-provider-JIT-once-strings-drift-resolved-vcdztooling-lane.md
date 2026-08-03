# PR #1075 review comment — cdz/src/main.rs (v-cdz-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1075
(PR: "cand: v-cdz-tooling — cdz main.rs --report-time").

## `--report-time` doc comment out of sync with output (Copilot, main.rs:4413) — doc
> `--report-time` documentation for `TestArgs::report_time` is now out of sync with the actual
> output: the code prints a new up-front `⏱ precompile: ...` line and the provider line text changed
> to "JIT'd/loaded once", but the arg doc comment still describes only a single up-front `provider
> JIT` line with "JIT'd once". This can mislead users and anyone grepping logs for the documented
> strings; please update the doc comment to reflect the current output (or keep the output strings
> aligned with the docs).
