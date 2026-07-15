# PR review comment — mirrored from GitHub PR #381 (Copilot inline)

- **PR:** #381 (MERGED)
- **File:** `spec/semantics/05-compound-types.sexp:4912`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589595149
- **Link:** https://github.com/camshaft/cadenza/pull/381#discussion_r3589595149

## Comment (verbatim)
> The case doc says `pe` propagates an Err unchanged, but the implementation returns `(Ok a)` when the *second* `pf` call errors (treating it as "no second factor"). The doc should distinguish Err propagation from the first `pf` vs the optional second `pf`, otherwise it describes a different control-flow shape than the pinned code.

## Liaison triage
Case-doc vs pinned-code mismatch in the compound-types corpus: the doc claims uniform Err propagation
but the code returns `(Ok a)` on a second-`pf` error. Doc-accuracy fix on a corpus case (spec
semantics). Route to `corpus-bugfix` PM — a corpus owner should reword the doc to distinguish
first-`pf` Err propagation from the optional-second-`pf` case (or adjust the case). Fix on `trunk`.
