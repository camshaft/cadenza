# PR review comment — mirrored from GitHub PR #383 (Copilot inline)

- **PR:** #383 "fleet: tenth batch (wide-string-match perf fix, M186 diag, librarian role, guide chapter)" (MERGED)
- **File:** `implementation/seed/crates/cadenza-syntax/src/parser.rs:1054`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589720686
- **Link:** https://github.com/camshaft/cadenza/pull/383#discussion_r3589720686

## Comment (verbatim)
> The postfix-chain depth guard only checks `spine >= MAX_NESTING_DEPTH`, but the overall arena depth for a node is `self.depth` (recursive nesting) + `spine` (postfix layers). As written, a deeply parenthesized expression that also has a long postfix chain can still build an arena deeper than `MAX_NESTING_DEPTH`, which reintroduces the stack-overflow/DoS risk this change is trying to prevent (recursive consumers like printer/canon will still walk the combined depth).

## Liaison triage — NEEDS ADJUDICATION (v-syntax)
The code has an EXPLICIT design comment defending the choice: it checks only `spine` (not
`self.depth + spine`) because "the enclosing recursion depth was already bounded at `expr` entry, so
re-adding `self.depth` here would double-count and reject a legitimate deep bracket nest whose postfix
run is short." So the author's model is that `self.depth` is independently bounded elsewhere. Copilot's
counter-claim is that the COMBINED arena depth (recursive nesting + postfix layers) can still exceed
MAX_NESTING_DEPTH and blow the stack in recursive consumers (printer/canon). This is a genuine
question about whether the two bounds compose — not obviously a bug either way. Route to `v-syntax`
(parser owner) to adjudicate against the actual depth accounting: does a deep-bracket-nest + long-
postfix-chain input build an arena that overflows a recursive consumer? A fuzz/probe (nest N brackets
then N postfix ops) would settle it. Fix (if any) on `trunk`.
