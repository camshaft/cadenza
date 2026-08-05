# PR #2145 review — cadenza-ast/src/dict.rs (v-syntax) — OPEN — 3 LOW cosmetics [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2145 (dict I3 — model/resolution API for dict-bearing AST:
DictSet::from_artifacts + resolve). Copilot 3 inline, all LOW/cosmetic on ONE file → batched.

## test var `c` bound to symbol "b" (`let c = b.name("b")`) is easy to misread since `b` is also the Builder (Copilot, dict.rs:186 & :194) — test-readability [VERIFIED, LOW]
> `c` is used as the variable name for the symbol "b" (`let c = b.name("b")`), which is easy to misread
> given `b` is also the Builder. Renaming the variable makes the subtree construction clearer.
VERIFIED — diff:112 & :124 (inside `#[cfg(test)] mod tests`) both `let c = b.name("b")`. Purely a
test-readability nit. Fix: rename the var to match the symbol (e.g. `sym_b`) or the symbol to `"c"`.

## test assertion slices `&transport[..8]` — panics (obscuring the real failure) if the encoder ever returns a too-short buffer; `is_transport_header`/`starts_with` avoids it + dedups the header literal (Copilot, dict.rs:234 & :282) — test-robustness [VERIFIED, LOW]
> This assertion slices `&transport[..8]`, which will panic if the encoder ever returns a too-short
> buffer (making the failure less diagnosable). Since this module already has `is_transport_header`,
> prefer using it (or `starts_with`) to avoid panics and reduce header-literal duplication.
VERIFIED — diff:158 & :210 assert on `&transport[..8]` in tests; the module already defines
`is_transport_header(bytes)` (diff:69). If the encoder regressed to a <8-byte buffer the slice panics
with an index-OOB instead of a clean assertion diff, making the real failure less diagnosable + it
re-hardcodes the 8-byte header literal. LOW/test-robustness (the mildly-substantive one, but still
test-only). Fix: assert via `is_transport_header(&transport)` / `transport.starts_with(HEADER)`.

## doc grammar: "A `resolve` graft keys on this exact hash…" reads like a grammar slip (Copilot, dict.rs:110) — doc-polish [VERIFIED, LOW]
> Doc comment grammar: "A `resolve` graft keys…" reads like a typo/grammar slip. Rewording makes the
> API contract clearer.
VERIFIED — diff:36 `/// A resolve graft keys on this exact hash…`. Minor doc-polish. Fix: reword (e.g.
"`resolve` grafts key on this exact hash…" / "A `resolve` call keys the graft on this exact hash…").

All 3 LOW, all foldable into #2145 pre-merge if v-syntax is touching the file (the `[..8]`→
`is_transport_header` is the one with any real value — cleaner test failure). No behavior bug in any.
v-syntax owns cadenza-ast.
