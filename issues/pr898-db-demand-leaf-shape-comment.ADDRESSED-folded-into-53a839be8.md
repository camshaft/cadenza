# PR#898 review comment — db-demand.cdz "leaf shape" fallback comment also catches unhandled composites (v-compiler-ml)

Mirrored from GitHub PR#898 review comment (Copilot), id `3676071880`.
File: `implementation/compiler-ml/src/db-demand.cdz:103` — compiler-ml PORT source, code-shape/comment →
v-compiler-ml (the port owner). Blame `fe3cd84f8` "compiler-ml(db-demand): item-4 real query-memo —
COMPOSITE producer (recursive, demands children)".

## Comment (verbatim)

- (id 3676071880, db-demand.cdz:103) "The fallback arm comment says 'leaf shape', but this branch is also
  taken for any node kind not explicitly handled here (composites included), where the leaf slice returns
  TErr. Updating the comment avoids implying only leaf nodes reach this path."

## Liaison verification (confirmed on trunk 6c6610c3f)

`demand-typed-node` matches `node-at` with explicit arms for `NBin`/`NIf`/`NLet`, then:
```
| Option.Some(_) => demand-typed-leaf(db, id)   // leaf shape → compute-from-source
| Option.None(_) => let ty = Typed.TErr in (fill-typed(db, id, ty), ty)
```
The `Option.Some(_)` wildcard catches EVERY node kind not matched above — genuine leaves AND any
composite/other kind the explicit arms don't cover. For a non-leaf reaching it, `demand-typed-leaf`'s
leaf slice returns `TErr` (not a true "leaf shape"). So the comment "leaf shape → compute-from-source"
under-describes: it implies only leaves reach here. Reword to name it the catch-all/fallback arm (e.g.
"any node kind not handled above — a genuine leaf computes from source; an unhandled composite falls to
TErr via the leaf slice"). Comment-only, behavior-neutral.

Owner: **v-compiler-ml** (compiler-ml port source, code-shape/comment of their own `fe3cd84f8`). Comment
reword. (Per liaison routing: compiler-ml port code-shape/comment → v-compiler-ml, not v-inference —
[[liaison-routing-compiler-ml-source-is-v-compiler-ml-not-v-inference]].)
