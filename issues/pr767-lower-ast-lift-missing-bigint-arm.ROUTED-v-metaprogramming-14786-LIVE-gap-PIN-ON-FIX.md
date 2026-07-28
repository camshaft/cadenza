# PR#767 review comment — lower_ast_lift missing a `Ty::BigInt` arm; `,x` where `x : BigInt` declines instead of lifting to Ast.Int

Mirrored from GitHub PR review comment (Copilot), id `3627499317`.
PR: https://github.com/camshaft/cadenza/pull/767 (batch-staging; fix belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/lower.rs:2554` (`lower_ast_lift`, the active-unquote lift)

## Comment (verbatim)

> `Ast.Int`'s payload is now `BigInt`, but `lower_ast_lift` only lifts integers when the operand is a
> grounded `Int64` (by widening to `BigInt`). If the active-unquote operand is already typed `BigInt`,
> this match falls through to the `other => decline` arm later, even though `Ast.Int` is the correct
> leaf. Add an explicit `Ty::BigInt` lift arm so `(quasiquote ... (unquote x) ...)` works when
> `x : BigInt`.

## Liaison verification (CONFIRMED on trunk/staging — real functional gap)

`lower_ast_lift` (lower.rs ~2527) matches `operand_ty.strip_nominal()`:
- `Ty::Sum … == ast_decl` → identity (already an Ast)
- `Ty::Int(it) if it.ground_signed() && it.ground_width() == 64` → widen to BigInt, wrap `Ast.Int`
- `Ty::Float(_) if ConstFloatNan` → decline
- `Ty::Float(ft) if width==64` → `Ast.Float`
- `Ty::Bool` / `Ty::String` / … → respective leaves
- `other => Core::Poison(Reject::decline(...))`

There is NO `Ty::BigInt` arm. The `Ast.Int` payload was just changed Int64→BigInt (`1bfb5c29e`
"Ast.Int payload Int64->BigInt — non-lossy quoted-AST integer storage (Part 1b)"), which added the
Int64→BigInt WIDENING path but not the already-BigInt case. So an active unquote `,x` with `x : BigInt`
(e.g. a let-bound BigInt, a BigInt-returning call spliced into a quasiquote) matches neither the Int64
arm (width 64 but the TYPE is BigInt, not Int) nor any value-leaf → falls to `other => decline`, even
though `Ast.Int` is exactly the right leaf and the payload is already BigInt (no widen needed).

Result: `(quasiquote … (unquote x) …)` fails to compile for `x : BigInt` — a functional regression of
the metaprogramming splice surface introduced alongside the BigInt payload change.

Fix (per Copilot): add a `Ty::BigInt => Core::SumNew { disc: disc.int, payloads: vec![operand] }` arm
(the operand is ALREADY BigInt, so wrap it directly — no `bigint-of-i64`/const-retype needed). Place it
before/beside the Int64 arm. Add a metaprogramming corpus/@test pinning `,x` with `x : BigInt` lifts to
`Ast.Int`.

Owner: v-metaprogramming (quote/eval/`Ast` domain; the `Ast.Int` BigInt-payload series `1bfb5c29e`).
Routed as a note flagged FUNCTIONAL-GAP.
