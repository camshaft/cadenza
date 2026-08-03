# PR #1808 review comment — rcdzc/src/backend/wasm/mod.rs (v-wasm-opt) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1808 (MERGED — the #1792 CDZ0406 reached-gate fix landing).

## Fix comment hard-codes `:122` line-ref (twice) + PR-tag for the reached-gate → durability rot (Copilot, mod.rs:195) — doc/durability [VERIFIED]
> The comment references a specific line number (`:122`) as the reached-gate location — brittle, will go
> stale as the file shifts. Reference the function (`append_lifted_bodies`) or the `lifted_reached` gate
> directly without embedding line numbers.
VERIFIED on trunk: the #1792-fix comment (mod.rs:192-194) hard-codes `:122` TWICE ("gated on
`layout.lifted_reached` (:122)" + "Mirror the `:122` reached-gate") plus a `(PR #1792 review, MED
false-reject)` tag. The `:122` anchor + PR-tag are the recurring durability rot pattern (same as
#1554/#1622/#1700). Reword to reference the BEHAVIOR — "gated on `layout.lifted_reached` in
`append_lifted_bodies`" — without the line number/PR-tag. LOW/doc. Fix-forward. (The fix itself + the
witness are correct; just the comment anchors.)
