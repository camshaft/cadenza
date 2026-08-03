# PR #1505 review comments — rcdzc/src/tests.rs + lower.rs (v-core-opt)

Mirrored from https://github.com/camshaft/cadenza/pull/1505 (PR: "[v-core-opt] 70e8bda5b").

## 1. `for (src, want) in cases` "won't compile" (Copilot, tests.rs:8776) — LIKELY FALSE-POSITIVE, verify
> The loop iterates over a `&[(&str, i64)]`, so `for (src, want) in cases` won't compile (it yields
> `&(&str, i64)`). Destructure the reference (or iterate with `.iter().copied()`), and then remove
> the now-unnecessary `*want` deref in the final assertion.

⚠ Likely FALSE POSITIVE: Rust MATCH ERGONOMICS bind `(src, want)` over a `&(&str, i64)` as `src:
&&str`, `want: &i64` — it compiles fine, and the code's `assert_eq!(got, *want, …)` already derefs
`want`, so it's self-consistent. (CI was still pending at review time, so not yet hash-confirmed — but
the match-ergonomics reasoning holds; if `cargo build` is green it compiles.) NOT actionable as a
compile fix. `.iter().copied()` is a fine readability choice but not required, and dropping the
`*want` would only apply IF you switch to copied().

## 2. Float-fold comment still says "canonical Float64" after the binary32 demotion (Copilot, lower.rs:19982, also :21279) — doc
> The implementation now demotes `Float32` constants to binary32 before comparing, but the preceding
> comment block still describes constant-float folding as comparing "canonical Float64" values/bits.
> Updating that comment will keep the documentation consistent with the new width-aware behavior.

Real doc-drift: the new width-aware fold demotes Float32 to binary32, but the comment still describes
Float64-canonical comparison. Update both sites (:19982, :21279) to the width-aware behavior.
