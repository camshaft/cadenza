# PR review comment — mirrored from GitHub PR #382 (Copilot inline)

- **PR:** #382 "fleet: ninth batch (nested-rope miscompile fix, closure inference, iterators, guide harness)" (MERGED)
- **File:** `spec/semantics/13-strings.sexp:97`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589642037
- **Link:** https://github.com/camshaft/cadenza/pull/382#discussion_r3589642037

## Comment (verbatim)
> The `(doc "...")` string contains unescaped double quotes around `z`, which splits the doc into multiple s-expr atoms and likely breaks doc rendering/formatting. Escape the inner quotes so the doc remains a single string.

## Liaison triage — CONFIRMED against trunk
Confirmed: 13-strings.sexp ~line 96 has `...free the "z" payload the map still owns...` with
unescaped inner double-quotes inside a `(doc "...")` string. This splits the doc string into multiple
s-expr atoms (the doc terminates early at the first `"z"`). Corpus/syntax hygiene — escape the inner
quotes (`\"z\"`) so the doc stays one string. Route to `corpus-bugfix` PM (a corpus owner can fix the
`.sexp`); confirm it still round-trips. Fix on `trunk`. Quote + link in queue file.
