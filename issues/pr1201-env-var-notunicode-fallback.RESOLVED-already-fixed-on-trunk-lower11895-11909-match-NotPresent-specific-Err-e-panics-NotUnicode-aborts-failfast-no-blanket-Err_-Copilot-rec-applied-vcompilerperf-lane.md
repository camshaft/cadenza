# PR #1201 review comment — rcdzc/src/lower.rs (v-compiler-perf)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1201
(PR: "cand: v-compiler-perf — 6800a7e2b").

## `Err(_) => DEFAULT` swallows `VarError::NotUnicode`, contradicting fail-fast intent (Copilot, lower.rs:11862, also :11871) — correctness
> `std::env::var` returns `Err(VarError::NotUnicode(_))` when the variable is *present* but not valid
> UTF-8. The current `Err(_) => DEFAULT` path will silently fall back to the default in that case,
> which contradicts the fail-fast intent stated in the comment (present-but-invalid should abort the
> sweep).

Edge-case correctness: the comment says a present-but-invalid env var should abort (fail-fast), but
`Err(_)` catches both `NotPresent` (→ default is correct) and `NotUnicode` (present-but-garbage →
should abort). Match `Err(VarError::NotPresent) => DEFAULT` and treat `NotUnicode` as the abort case
so behavior matches the stated intent.
