# PR #1677 review comment — rcdzc/src/backend/rust/expr.rs (v-rust-backend) — OPEN

https://github.com/camshaft/cadenza/pull/1677 (odd-width signed div MIN/-1 guards the DECLARED width —
adv-67 fix).

## Aliased-width check duplicates the `{8,16,32,64}` literal instead of `ALIASED_INT_WIDTHS` (Copilot, expr.rs:4475) — cleanliness/drift
> The aliased-width check duplicates the `{8,16,32,64}` list even though the codebase already defines
> `ALIASED_INT_WIDTHS`. Using the shared constant avoids drift if the aliased set changes.

VERIFIED: `ALIASED_INT_WIDTHS: [u32; 4] = [8, 16, 32, 64]` is defined in ty.rs:174 and consumed via
`.contains()` at 4 infer.rs sites (:1526, :7026, :7052, :12687). The new guard at expr.rs:4475 uses
`matches!(w, 8 | 16 | 32 | 64)` — a duplicated literal that would drift if the aliased set ever changes.
Replace with `crate::ty::ALIASED_INT_WIDTHS.contains(&w)`. LOW/cleanliness. Fix-forward.
