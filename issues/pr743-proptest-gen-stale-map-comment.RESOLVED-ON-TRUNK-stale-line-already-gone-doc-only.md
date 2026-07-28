# PR#743 review comment — proptest_gen.rs stale comment above GenTy::Map (old fixed-size fold)

Mirrored from GitHub PR review comment (Copilot), id `3623671109`.
PR: https://github.com/camshaft/cadenza/pull/743 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/proptest_gen.rs:1575`

## Comment (verbatim)

> There's a leftover/outdated comment line above the `GenTy::Map` case that still describes the old
> fixed-size `Map.insert` fold, but the implementation now delegates to `build_var_map_gen` for
> variable cardinality. Removing the stale line avoids misleading future edits.

## Liaison verification (CONFIRMED on trunk)

At proptest_gen.rs ~1570 there are TWO stacked comment descriptions above `GenTy::Map`:
- line ~1570 (STALE): "A fold of `Map.insert` over `G1_LIST_LEN` generated key/value pairs, seeded
  from `Map.empty`:" — the OLD fixed-size behavior.
- lines ~1571-1574 (CURRENT): "A VARIABLE-size map (`0..=G1_LIST_LEN` entries) via a `Map.insert`
  fold over `(Map.empty)` … See `build_var_map_gen`."

The impl is now `GenTy::Map(kty, vty) => build_var_map_gen(ast, kty, vty, binds)` (variable
cardinality, landed `04cdced1f` "variable-size Map generation — reach the empty + small maps").
The leading stale line should be removed.

Doc-only, no behavior change. Owner: whoever owns `rcdzc/src/proptest_gen.rs` (property-test generation;
landed `04cdced1f`). Filing to corpus-bugfix PM to route (property-testing was marked complete;
PM can place this trivial cleanup with the right owner).
