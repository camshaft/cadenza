# PR #2167 review — reducer-cadenza fixtures (v-harness-bootstrap) — OPEN — 4 LOW cosmetic/formatting [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2167 (coverage — pin the effect-request target + correlation
fields B2/B3). Copilot 4 inline, ALL LOW formatting/readability on the test fixtures → batched.

## B2 pinning tests use single-line match/if → harder to read, inconsistent with b2_the_effect_is_http above (Copilot, reducer_b2.cdz:61) — test-readability [VERIFIED, LOW]
> The new B2 pinning tests use a single-line `match`/`if` expression … Reformatting … into multi-line
> `match` and `if` blocks will improve readability and reduce very long lines.

## `b3_http_effect_target_and_correlation_pinned` is a deeply nested parenthesized `if` chain → hard to scan (Copilot, reducer_b3.cdz:122) — test-readability [VERIFIED, LOW]
> … formatted as a deeply nested parenthesized `if` chain … Reformatting into a multi-line `if` block
> (without extra parentheses) would improve readability …

## a comment breaks `sum-disc/str-get/sum-payload` across lines leaving a dangling `/` that reads like a typo (Copilot, reducer_b3.cdz:110) — comment-clarity [VERIFIED, LOW]
> This comment currently breaks `sum-disc/str-get/sum-payload` across lines as `sum-disc/str-get/` +
> `sum-payload`, leaving a dangling slash … reflow to avoid the trailing `/`.

## `b3_http_effect_kind_is_http` nests a second `match` inside parens on one line → harder to read vs sibling tests (Copilot, reducer_b3.cdz:114) — test-readability [VERIFIED, LOW]
> … nests a second `match` inside parentheses on a single line … Reformatting into a multi-line `match`
> improves clarity and keeps the style consistent.

ALL VERIFIED as the described formatting shapes in the #2167 diff (single-line match/if, nested-paren if
chain, wrapped-slash comment). All 4 are LOW cosmetic/test-readability — NO behavior, NO correctness issue;
these are new @test pins, so formatting-only. Batched. Fix: reflow to multi-line match/if blocks matching
the `b2_the_effect_is_http` sibling style + reflow the `sum-disc/str-get/sum-payload` comment to avoid the
dangling `/`. Foldable into #2167 pre-merge if v-harness-bootstrap is polishing the fixtures. (Note: the
fixtures must pass the ML round-trip, not just gate — but these are formatting-only so low-risk.)
v-harness-bootstrap owns the reducer fixtures.
