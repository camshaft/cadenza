# PR review comment — mirrored from GitHub PR #409 (Copilot inline)

- **PR:** #409 (MERGED)
- **File:** `implementation/seed/crates/cadenza-syntax/src/literal.rs:352` (`split_template_body`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591276064
- **Link:** https://github.com/camshaft/cadenza/pull/409#discussion_r3591276064

## Comment (verbatim)
> `split_template_body` tracks whether it's inside a string literal within a hole by toggling `in_str` on every `"`, but it doesn't account for escaped quotes (e.g. a hole containing `g(\"}\")`). The lexer's `read_template_body` treats backslash as an escape anywhere, so an escaped quote inside a hole should not toggle string mode; otherwise brace balancing and hole splitting can be wrong.

## Liaison triage
Tagged-template hole splitting: `split_template_body` toggles `in_str` on every `"`, ignoring backslash
escapes. A hole containing an escaped quote (e.g. `{ g("}") }` written with escapes) would wrongly
toggle string mode, throwing off brace balancing and hole splitting — so a `}` inside such a string
could prematurely close the hole/template. The reviewer notes the lexer's `read_template_body` DOES
treat backslash as an escape anywhere, so the two disagree. Real parse-correctness bug in the
tagged-template reader (a recent feature). Syntax territory (v-syntax). FIX: honor `\`-escapes when
toggling `in_str` (skip the char after a backslash), matching read_template_body. Fix on `trunk`. Quote
+ link in queue file.
