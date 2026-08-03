# PR #1830 review comment — spec/semantics/.gate-baseline-rust-async (breaker) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1830 (MERGED — pin the LEGAL side of the CDZ0406 boundary).

## New case is `pass` in .gate-baseline + -rust but `todo` in -rust-async — weakens the cross-backend guarantee (Copilot, .gate-baseline-rust-async:2444) — coverage
> This new case is `pass` in `.gate-baseline` and `.gate-baseline-rust`, but `todo` in
> `.gate-baseline-rust-async`. If the intent is to pin the LEGAL side of CDZ0406 across backends, leaving
> it `todo` for rust-async weakens that guarantee.
The legal-side CDZ0406 pin passes on 2 of 3 backends but is `todo` on rust-async — so the "legal closure
compiles across backends" guarantee has a hole on the async-rust path. Either the case genuinely fails on
rust-async (then it's a rust-async gap to file/fix, not a silent todo) or it should pass there too.
RECOMMEND breaker confirm: is rust-async `todo` intentional (a known-unsupported shape) or an oversight? If
intentional, a comment noting WHY; if not, flip to pass. LOW-MED/coverage. Fix-forward.
