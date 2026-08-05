# PR #2164 review — guide/src/content/chapters/WritingAReducer.tsx (v-guide) — OPEN — reader-correctness [VERIFIED, LOW] (residual of MY #2157)

https://github.com/camshaft/cadenza/pull/2164 (reducer-chapter 2 LOW review nits — the fix-forward for MY
#2157 guide findings). Copilot 1 inline — the arrow fix traded one copy-paste hazard for another.

## the #2157 arrow fix replaced the Unicode `→` with a `{"->"}` JSX-expression injection → still copy/paste-brittle (some tooling preserves `{"->"}` literally instead of rendering `->`) (Copilot, WritingAReducer.tsx:192) — reader-correctness [VERIFIED, LOW]
> `{"->"}` injects a JSX expression into what appears to be a literal code-like snippet. This can make
> copy/paste and any potential downstream text extraction more brittle (some tooling may preserve
> `{"->"}` instead of rendering `->`). Prefer rendering the arrow as plain text (either `->` or the
> original `→`) within the snippet, or use a dedicated code-rendering component/escape mechanism …

VERIFIED in the #2164 diff: my #2157 finding (Unicode `→` in a copyable snippet → parse error on paste)
was fixed by swapping `→` for `{"->"}` — `effect Kv = | get : Bytes {"->"} Option(Bytes) | put : (Bytes,
Bytes) {"->"} Unit` (diff:23). So the fix removed the Unicode-arrow hazard but introduced a JSX-expression
artifact: a reader copying the rendered line is fine, but any text-extraction path that reads the SOURCE
(or tooling that doesn't evaluate the JSX expr) gets the literal `{"->"}` instead of `->`. LOW/reader-
correctness — it's a narrower hazard than the original Unicode `→` (the rendered output IS correct `->`),
but it's a code-in-JSX smell. Fix per Copilot: render the arrow as plain text `->` directly in the snippet
(ASCII `->` needs no JSX escape — the earlier concern was only the Unicode glyph), OR use the chapter's
code-rendering component if one exists. i.e. just write `->` literally: `get : Bytes -> Option(Bytes)`.
LOW. v-guide executes guide content. PR OPEN → foldable. (Owning the chain: my #2157 flagged the Unicode
arrow; the fix over-corrected to `{"->"}` — plain ASCII `->` is the simplest correct form. One-layer-deeper
residual on my own finding.)
