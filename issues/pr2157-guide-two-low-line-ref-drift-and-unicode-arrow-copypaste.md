# PR #2157 review — guide (v-guide) — OPEN — 2 LOW [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2157 (reducer-chapter review fixes — Kv arrow footnote,
correlation-token wording, gate strip/rewrap symmetry; the PR that carries the fix for MY #2146 toggle-gate
finding). Copilot 2 inline, both LOW, both on guide content → batched.

## the comment for the (my-#2146) toggle-gate fix hardcodes a line reference "renderToMl (line ~89)" → drifts as the file changes; cite the helper by NAME only (Copilot, check-examples.mjs:629) — comment-drift [VERIFIED, LOW]
> The comment hard-codes a nearby line reference ("line ~89"), which will drift as this file changes and
> makes the rationale harder to trust later. Prefer referencing the helper by name (renderToMl) without a
> line number.
VERIFIED in the #2157 diff (diff:12): the new toggle-gate comment reads "the EXACT mirror of renderToMl
(line ~89) in the ml→s-expr direction". The `(line ~89)` is a hardcoded line ref that goes stale on any
edit above it. LOW/comment-drift. Fix per Copilot: drop the line number, reference `renderToMl` by name
only. (This is the SAME discipline I hold — cite names/case-names not line numbers in code comments — and
it landed on the comment for the fix to MY #2146 finding, so worth getting right.)

## the chapter Note mixes the typographic Unicode arrow `→` with real surface syntax, but the lexer only accepts ASCII `->` → a reader copy/pasting `→` gets invalid code unless it's explicitly called out as presentation-only (Copilot, WritingAReducer.tsx:201) — reader-correctness [VERIFIED, LOW]
> This note mixes the typographic Unicode arrow (shown in the Note above) with the real surface syntax.
> Since the lexer only recognizes the ASCII `->`, readers who copy/paste may end up with invalid code
> unless we explicitly call out that `→` is just presentation.
VERIFIED in the diff (diff:52-53): the Kv arrow footnote shows the real source form `` `->`(Bytes, Bytes,
Unit) `` (ASCII, backticked) vs the tuple-looking `(Bytes, Bytes) -> Unit`, but the surrounding Note (per
Copilot, tsx:201) renders the arrow as the Unicode `→` for typography. The lexer only accepts ASCII `->`
(cf the backtick-name lexing — `→` is not an operator token), so a reader who copies the `→` from the Note
into real code gets a parse error. LOW/reader-facing correctness (guide code should be copy-paste-safe or
explicitly flag presentation glyphs). Fix per Copilot: either use ASCII `->` in the copyable spots, or add
a one-line "`→` here is typographic; the real surface syntax is ASCII `->`" caveat next to the Note.

Both LOW, both guide-content, foldable into #2157 pre-merge. v-guide executes guide content (github-liaison
directs). No behavior/code bug in either.
