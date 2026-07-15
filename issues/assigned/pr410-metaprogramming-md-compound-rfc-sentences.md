# PR review comments — mirrored from GitHub PR #410 (Copilot inline)

- **PR:** #410 (MERGED)
- **File:** `spec/capabilities/metaprogramming.md` (lines 100, 102, 104, 106)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591321459, 3591321468, 3591321481, 3591321500
- **Links:** https://github.com/camshaft/cadenza/pull/410#discussion_r3591321459 (+ r3591321468, r3591321481, r3591321500)

## Comments (verbatim, condensed)
> [100] Contains two RFC-2119 requirements (`MUST NOT run any program code…` and `MUST only split…`) in one sentence.
> [102] Introduces three requirements (`MUST be parsed`, `MUST appear`, `MUST be exactly…`) in one sentence.
> [104] Combines dispatch-by-binding, resolve, and function-shape constraint into one sentence.
> [106] Contains multiple requirements (evaluate, splice, expand to fixpoint) in one sentence.

## Liaison triage — CONFIRMED against trunk (spot-check line 100)
Confirmed line 100: "The reader MUST NOT run any program code or learn any grammar when lexing a tagged
template: it MUST only split the string body into literal chunks and `{…}` holes, …" — two obligations
in one sentence. Four sentences in the new metaprogramming.md section each bundle multiple RFC-2119
obligations, violating the project's one-atomic-obligation-per-sentence rule. This is the SAME recurring
class the liaison has now surfaced across pr385, pr398, pr399 — a systematic spec-authoring pattern in
newly-added sections. Split each into atomic MUST/MUST-NOT sentences under the same heading (and update
any duvet citations that point at them). Squarely v-duvet-coverage territory. Fix on `trunk`. Quotes +
links in queue file.
