# PR review comments — mirrored from GitHub PR #423 (Copilot inline)

- **PR:** #423 (MERGED)
- **File:** `implementation/seed/crates/cadenza-syntax/src/token.rs:524/530`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592007572, 3592007605
- **Links:** https://github.com/camshaft/cadenza/pull/423#discussion_r3592007572 , #discussion_r3592007605

## Comments (verbatim, condensed)
> `ALL_KEYWORDS` is intended to represent the full `Keyword` variant set, but nothing enforces that at compile time. If a new `Keyword` variant is added and the author forgets to list it in `ALL_KEYWORDS`, [it isn't caught at compile time].
> The `assert_all_keywords_listed` match comment claims it enforces `ALL_KEYWORDS` completeness, but the match doesn't reference `ALL_KEYWORDS`, so it can't catch a missing entry there. [Correct the comment, or add a compile-time length/variant_count check.]

## Liaison triage — CONFIRMED against trunk (light follow-up to pr420)
Confirmed: `assert_all_keywords_listed` is an exhaustive `matches!(kw, Keyword::Let | … )` — it forces
every VARIANT to be named (compile error on a new variant), but its COMMENT claims it "enforces that
`ALL_KEYWORDS` is complete", which it does NOT reference. The actual `ALL_KEYWORDS`↔table tie is a
RUNTIME test asserting `KEYWORD_SPELLINGS.len() == ALL_KEYWORDS.len()` (token.rs:572-573) — so a variant
added to the enum + the exhaustive match but omitted from `ALL_KEYWORDS` isn't caught at COMPILE time.
Two small asks: (a) fix the misleading comment; (b) optionally add a compile-time completeness check for
`ALL_KEYWORDS` (e.g. a `variant_count`-based length assert) so the array can't silently drift. This is a
refinement of the pr420 keyword-test fix v-syntax just landed. Low severity. v-syntax territory. Fix on
`trunk`. Quotes + links in queue file.
