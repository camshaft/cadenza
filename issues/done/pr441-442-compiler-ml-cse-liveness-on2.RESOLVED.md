# PR review comments — mirrored from GitHub PRs #441/#442 (Copilot inline) — compiler-ml O(n²) analyses

- **PRs:** #441, #442 (MERGED)
- **Files:** `implementation/compiler-ml/src/cse.cdz:52`, `implementation/compiler-ml/src/liveness.cdz:52`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592691404 (cse), 3592727183 (liveness)
- **Links:** https://github.com/camshaft/cadenza/pull/441#discussion_r3592691404 , https://github.com/camshaft/cadenza/pull/442#discussion_r3592727183

## Comments (verbatim)
> [cse.cdz] `collect` recomputes `hash(e)` at every node, and `hash` recursively traverses the subtree, so `distinct-count` becomes O(n^2) for larger expressions. You can compute each node's hash once (bottom-up) and reuse it.
> [liveness.cdz] `dead-store-count` recomputes `live-in(rest, end-out)` for every prefix, making the analysis O(n²) over the statement list. Since this is intended as a compiler middle-end analysis, compute the liveness once (bottom-up / from the end) and reuse it.

## Liaison triage — CONFIRMED against trunk
Two sibling O(n²) inefficiencies in the compiler-ml analysis modules:
- `cse.cdz`: `collect` calls `hash(e)` at every node, and `hash` recursively re-traverses the subtree →
  `distinct-count`/`savings` are O(n²). Compute each node's hash once bottom-up and reuse.
- `liveness.cdz`: `dead-store-count` recomputes `live-in(rest, end-out)` for every prefix → O(n²) over
  the statement list. Compute liveness once (from the end) and reuse.
Both are the Cadenza-written compiler's middle-end analyses (v-compiler-ml). Not correctness bugs —
efficiency/algorithmic-complexity, matter as the ML compiler scales. Fixes on `trunk`. Quotes + links
in queue file.
