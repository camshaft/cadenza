# PR #1323 review comments — spec/semantics/12-metaprogramming.sexp + baselines (corpus-bugfix)

Mirrored from https://github.com/camshaft/cadenza/pull/1323 (PR: "cand: corpus-bugfix — 75283121e").

## Case title "declines CDZ0101" inconsistent with `(error CDZ0101)` outcome (Copilot, 12-metaprogramming.sexp:884 + 3 baseline lines) — doc/naming
> [.sexp:884] Case title says "declines CDZ0101" but the expected outcome is `(error CDZ0101)`, and
> the doc text says the form "is rejected CDZ0101". Using "declines" here is inconsistent with the
> result type and with surrounding metaprogramming cases; consider renaming the case to "is rejected
> CDZ0101" — "declines" (no artifact) is a different concept from a coded error.
> [.gate-baseline:4171, .gate-baseline-rust:4121, .gate-baseline-rust-async:4056] update the baseline
> entry title text to match the renamed case.

Terminology: "declines" (emit produced no artifact) ≠ "rejected with `(error CDZ0101)`" (a coded
diagnostic). Rename the case to "is rejected CDZ0101" for consistency with the outcome + sibling
cases, and update the matching title text in all three `.gate-baseline*` files (a corpus-title edit —
hand-edit the baseline rows, don't `--save`, per the gate-baseline discipline).
