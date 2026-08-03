# PR #1617 review comments — rcdzc/src/backend/rust/{expr,enums}.rs (v-rust-backend) — MERGED, fix-forward

Mirrored from https://github.com/camshaft/cadenza/pull/1617 (PR: "rcdzc: a monomorphic float-carrying
sum gets a custom impl Ord …", MERGED 2026-08-03). 2 Copilot comments just before merge.

## 1. Unreachable BigInt/Rational branch + self-contradicting comments (Copilot, expr.rs:3330) — dead-code/correctness [VERIFIED]
> The `BigInt`/`Rational` special-case is unreachable because `ty_supports_eq(db, ty)` is already true
> for those types (see `enums::ty_derives_eq`), so this branch can be removed.

VERIFIED against the code: in `emit_value_ord_walk_seen` (expr.rs:3313), the FIRST branch is `if
super::enums::ty_supports_eq(db, ty) { return Ok(format!("{l}.cmp(&{r})")) }`. `ty_supports_eq` →
`ty_derives_eq`, whose tail arm is `Ty::BigInt | Ty::Rational => true` (enums.rs). So BigInt/Rational
ALWAYS take the first branch, making the SECOND `if matches!(ty, Ty::BigInt | Ty::Rational) { return
… .cmp() }` (expr.rs:3327) genuinely UNREACHABLE dead code. The two branches also CONTRADICT in their
comments: the first says "BigInt/Rational also reach here and have a total `cmp`" (:3316) while the
second says "BigInt/Rational are `Ord` but NOT yet `ty_supports_eq`-true" (:3327) — the second comment
is FALSE. Fix-forward: remove the dead second branch (both branches emit the identical `.cmp()`, so
behavior is unchanged) and drop the contradictory comment. LOW-MED (dead code, no runtime effect, but a
misleading invariant claim in a load-bearing ord-emit path).

## 2. `sentinel_sum_of(&decl)` computed twice back-to-back (Copilot, enums.rs:387) — cleanliness
> `sentinel_sum_of(&decl)` is computed twice back-to-back; computing it once improves readability and
> avoids constructing the same `Ty` twice.

Bind it to a local once and reuse. LOWEST/cleanliness.
