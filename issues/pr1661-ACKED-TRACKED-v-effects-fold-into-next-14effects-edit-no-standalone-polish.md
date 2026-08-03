# PR #1661 review comment — spec/semantics/14-effects-and-handlers.sexp (v-effects) — OPEN

https://github.com/camshaft/cadenza/pull/1661 (pin a Bytes+scalar mixed-arity host case).
Author is v-effects (cand/v-effects-e3454beadea0), NOT corpus-bugfix (title says "corpus(14-effects)"
but the shared fleet-identity — verified via `gh pr view --json author`; routing to v-effects).

## `((UInt 8).wrap ...)` should use the established `UInt8.wrap` alias (Copilot, 14-effects-and-handlers.sexp:6997) — style/consistency
> Use the established `UInt8.wrap` alias for 8-bit narrowing instead of `((UInt 8).wrap ...)`. This file
> already uses `UInt8.wrap` elsewhere (e.g. ~line 6034), and this is the only occurrence of the `(UInt 8)`
> spelling.

VERIFIED on the cand branch: `UInt8.wrap` appears 4× in the file; line 6997 is the SOLE `((UInt 8).wrap
…)` spelling (`(io.sink2 (Bytes.of (list ((UInt 8).wrap k) ((UInt 8).wrap 66))) 5)`). Align to
`UInt8.wrap` for a consistent one-spelling file. LOW/style. (corpus .sexp — fold into next 14-effects
edit per the no-standalone-polish steer.)
