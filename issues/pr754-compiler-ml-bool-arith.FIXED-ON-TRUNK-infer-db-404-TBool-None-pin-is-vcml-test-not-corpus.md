# PR#754 review comment — compiler-ml: arithmetic on Bool operands type-checks as Bool instead of rejecting (SOUNDNESS)

Mirrored from GitHub PR review comment (Copilot), id `3625199965`.
PR: https://github.com/camshaft/cadenza/pull/754 (merged; fix still belongs on trunk)
Location: `implementation/compiler-ml/src/infer-db.cdz:394` (`typed-to-ty-defer`, the `Typed.TBool` arm)

## Comment (verbatim)

> `typed-to-ty-defer` maps `Typed.TBool` to `Ty.TyBool`, which lets `arith-result-type` return
> `Some(Typed.TBool)` for Bool operands. Since `bin-type` uses `arith-result-type` for arithmetic ops
> (43/45/42/47/37), this can make expressions like `(+ true false)` type-check as `Bool` instead of
> rejecting as `TErr`.
>
> Return `None` for `Typed.TBool` here so `arith-result-type` only succeeds for integer operands;
> Bool-Bool relational ops are already handled by the `both-bool` fallback in `bin-type`.

## Liaison verification (CONFIRMED — real mis-accept / soundness bug)

Traced `(+ true false)` on trunk:
1. `typed-to-ty-defer` (infer-db.cdz:391-394): `Typed.TBool => Option.Some(Ty.TyBool)`.
2. `arith-result-type` (infer-db.cdz:323): both operands lookup `TBool` → `typed-to-ty-defer` gives
   `Some(TyBool)` for each → `unify-ty(TyBool, TyBool)` = `Some(TyBool)` (unify-ty.cdz:59-62). The
   `lit-operands-fit-result` guard only range-checks a bare `NLit` INT operand; a Bool literal node is
   `NBoolLit`, not `NLit`, so it passes → returns `Some(ty-to-typed-defer(TyBool))` = `Some(TBool)`.
3. The arithmetic arm of `bin-type` (infer-db.cdz:293-297): `match arith-result-type … | Some(rty) =>
   (match op with | 43 => rty …)` — for op 43 (`+`) yields `rty` = **`TBool`**.

So `(+ true false)` is accepted as `Bool` rather than rejected `TErr` — the compiler admits an
ill-typed arithmetic expression. This is a soundness/mis-accept bug (arith requires integer operands
per the `Typed` doc at infer-db.cdz:24 "arithmetic … both REQUIRE `TInt`").

Fix (per Copilot, verified safe): make the `TBool` arm of `typed-to-ty-defer` return
`Option.None(unit)` so `arith-result-type` succeeds only for integer operands. Bool-Bool RELATIONAL
ops (op 60 `<` / 61 `==`) are unaffected — the relational arm (infer-db.cdz:283-285) already falls
back to `both-bool` when `arith-result-type` is `None`, returning `TBool` for two Bools. So the fix
tightens arithmetic without breaking `true == false`.

Owner: v-compiler-ml (owns `compiler-ml/*` source — the Cadenza-in-Cadenza compiler; NOT v-inference,
which owns rcdzc infer/unify/resolve). Add a corpus/`@test` pin: `(+ true false)` → TErr/reject, while
`(== true false)` → Bool still passes. Routed as a note flagged CONFIRMED-MISCOMPILE.
