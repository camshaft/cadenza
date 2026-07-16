# PR review comment — mirrored from GitHub PR #430 (Copilot inline)

- **PR:** #430 "fleet: fifty-fourth batch (compiler-ml constprop, …)" (MERGED)
- **File:** `implementation/compiler-ml/src/constprop.cdz:47`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592228050
- **Link:** https://github.com/camshaft/cadenza/pull/430#discussion_r3592228050

## Comment (verbatim)
> `cp-env` keeps a non-constant `let x = rhs` but propagates the body under the *same* env. If `env` already contains a constant for `x` (outer binding), this incorrectly substitutes `x` inside the body even though the inner `let` shadows it (e.g. `let x=5 in let x=w in x+1` would propagate to `5+1`). The body should be propagated under an env with `x` removed (or otherwise marked unknown) for the non-constant case.

## Liaison triage — CONFIRMED against trunk — MISCOMPILE (shadowing)
Confirmed in constprop.cdz `cp-env` `Ex.Let(x, rhs, body)`:
```
| Ex.Num(v) => cp-env(Map.insert(env, x, v), body)   // constant: propagate + drop the let
| _         => Ex.Let(x, rhs2, cp-env(env, body))    // non-constant: keep the let, propagate body <-- SAME env
```
The non-constant arm propagates `body` under the SAME `env`, which may still hold an OUTER constant for
`x`. So `let x=5 in let x=w in x+1` const-folds the inner (shadowing) `x` to 5 → `5+1`, even though the
inner `let x=w` shadows it with a non-constant. That's a wrong-value constant fold — a soundness bug in
the compiler-ml constprop pass. FIX: propagate the non-constant body under `Map.remove(env, x)` (drop
the stale outer constant for the shadowed name). compiler-ml territory (v-compiler-ml). A regression
test (`let x=5 in let x=w in x+1` must NOT fold the inner x) would pin it. Fix on `trunk`. Quote + link
in queue file.
