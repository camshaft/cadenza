# PRs #1907 + #1904 review comments — LOW/doc

## PR #1907 (rcdzc/src/effects.rs:3411, v-effects) — doc/accuracy
The docstring says this peels only PURE-binding block wrappers (a PERFORMING binding is a different
threaded shape), but the `Resolved::Let` arm [per Copilot] may also match/peel a performing-binding case —
verify the arm's guard matches the docstring's "pure-only" claim; align doc or guard. LOW/doc.

## PR #1904 (cdz-agent-host/src/host.rs:113, v-agent-harness-host) — doc/grammar
"after calling it, a `push_capabilities_changed` recomputes" — drop the stray article ('a'); it refers to
a function. LOW/grammar. (Same host.rs:113 area as the #1900 link nit.)
