# PR review comment — mirrored from GitHub PR #414 (Copilot inline)

- **PR:** #414 (MERGED)
- **File:** `xtask/src/duvet_check.rs:239` (`last_lines`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591489083
- **Link:** https://github.com/camshaft/cadenza/pull/414#discussion_r3591489083

## Comment (verbatim)
> The doc comment says this returns the last `n` "non-empty-ish" lines, but the implementation does not filter blank/whitespace-only lines; it returns the last `n` lines verbatim. Either filter empties or adjust the comment to match.

## Liaison triage — CONFIRMED against trunk
Confirmed: `/// The last n non-empty-ish lines …` but `fn last_lines` does `s.lines().collect()` and
takes the last `n` verbatim — no blank/whitespace filtering. Doc/impl mismatch (comment overclaims).
Low severity. FIX: either filter empties (`.filter(|l| !l.trim().is_empty())`) or drop "non-empty-ish"
from the doc. Fleet-tooling territory (xtask). Fix on `trunk`. Quote + link in queue file.
