# PR review comment — mirrored from GitHub PR #432 (Copilot inline)

- **PR:** #432 "fleet: fifty-sixth batch (compiler-ml optimize capstone, …)" (MERGED)
- **File:** `implementation/compiler-ml/src/optimize.cdz:44` (`cp` non-constant Let arm)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592281315
- **Link:** https://github.com/camshaft/cadenza/pull/432#discussion_r3592281315

## Comment (verbatim)
> In `cp`, the non-constant `let` case propagates `body` under the unchanged `env`. If `env` already contains an outer constant for the same name `x`, this incorrectly substitutes the shadowed name inside `body` (e.g. `let x=1 in let x=w in x+1` can fold the inner `x` to 1). The body should be propagated under an environment with `x` removed for the non-constant case.

## Liaison triage — CONFIRMED against trunk — TWIN of the pr430 constprop miscompile
Confirmed: `optimize.cdz`'s `cp` non-constant `Ex.Let` arm is `Ex.Let(x, r2, cp(env, body))` — the SAME
shadowing bug just fixed in `constprop.cdz` (pr430, MR 0a0bb0ed added `Map.remove(env, x)`), but in a
SECOND file (the "optimize capstone"). So `let x=1 in let x=w in x+1` folds the inner shadowing `x` to
the outer constant 1 → wrong-value fold. This is the classic "a repair applied at one position but not
its twin" — the constprop fix must be mirrored here. FIX: propagate the non-constant body under
`Map.remove(env, x)` (same as constprop.cdz), + a regression test. compiler-ml (v-compiler-ml). Fix on
`trunk`. Quote + link in queue file.
