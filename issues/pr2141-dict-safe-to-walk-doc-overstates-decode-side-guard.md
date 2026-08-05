# PR #2141 review — cadenza-ast/src/codec.rs (v-syntax) — OPEN — doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2141 (dict_is_safe_to_walk — reject out-of-range LEAF ids
too + rename from dict_is_acyclic; folds MY #2121 review). Copilot 1 inline, doc-accuracy.

## `dict_is_safe_to_walk`'s doc says it guards BOTH `encode_with_dict` AND `decode_with_dicts`' graft, but decode never calls it (decode rejects corruption via its own fallible `IdOutOfRange`/`NotATree` bounds+cycle checks) → doc misleads about where decode-side safety is enforced (Copilot, codec.rs:1004) — doc-accuracy [VERIFIED, LOW]
> The doc comment says this check is required for both `encode_with_dict` and `decode_with_dicts`
> grafting, but `decode_with_dicts` doesn't call `dict_is_safe_to_walk` and already handles dict
> corruption via fallible bounds/cycle checks (returning `IdOutOfRange`/`NotATree`). As written, the
> comment overstates the decode-side risk and can mislead future readers about where safety is
> enforced.

VERIFIED in the #2141 diff: the ONLY production call site is inside `encode_with_dict`
(`if !dict_is_safe_to_walk(dict) { continue; }`, diff:27 — the rename of the old `dict_is_acyclic`
skip). The function DOC (diff:50-51) says it "is only safe to walk (in `encode_with_dict`'s
`subtree_arena` match-table build AND `decode_with_dicts`' graft)". But decode has NO call to it — and
the code's OWN inline comment two lines down (diff:88-89) says the opposite: "The DECODE graft rejects
it as IdOutOfRange separately; encode has no such fallible seam, so the guard must [apply on encode]".
So the doc contradicts the adjacent comment + the actual call graph: decode enforces safety via its
fallible seam (`IdOutOfRange`/`NotATree`), encode enforces it via this infallible skip-guard. The doc
implies decode relies on `dict_is_safe_to_walk` when it doesn't. LOW/doc-accuracy — no behavior bug, but
a future reader editing decode could wrongly assume this guard protects it. Fix per Copilot: reword the
doc to scope the guard to `encode_with_dict` (the infallible path with no fallible seam), and note decode
enforces the same invariant via its own fallible bounds/cycle checks (`IdOutOfRange`/`NotATree`). PR OPEN
→ foldable pre-merge. v-syntax owns cadenza-ast. (This closes out the #2121 fold — the guard logic itself
is correct + well-tested; only the doc scope overstates.)
