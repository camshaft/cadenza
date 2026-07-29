# PR#900 review comment — db-demand NVar doc claims a param binder is "seeded by its NApp arm" that doesn't exist yet (v-compiler-ml)

Mirrored from GitHub PR#900 review comment (Copilot), id `3677077762`.
File: `implementation/compiler-ml/src/db-demand.cdz:108` — compiler-ml PORT source, code-shape/comment →
v-compiler-ml (port owner). Blame `53a839be8` "compiler-ml(db-demand): item-4 real query-memo — NVar
producer (cross-column resolve→type) + resolve-first driver" (the same commit that reworded the sibling
"leaf shape" fallback comment from PR#898).

## Comment (verbatim)

- (id 3677077762, db-demand.cdz:108) "The NVar producer doc comment says a parameter binder's type was
  'seeded by its NApp arm', but this module currently has no NApp producer (and the fallback comment
  below explicitly calls out NApp as unhandled). This makes the comment misleading; it should describe
  only the guarantees this slice actually provides (let-bound vars) or phrase the parameter case
  conditionally."

## Liaison verification (confirmed on trunk 84eda64f2)

`demand-var`'s doc (db-demand.cdz:100-112), the `RBound(binder)` case: "…for a let-bound var the binder
IS the enclosing NLet, whose demand-let fills the VALUE's type BEFORE demanding the body (the var), so
tcol[valId] is present when the var is demanded; **and a param binder's type was seeded by its NApp
arm.**" But there is NO NApp producer in this module — `demand-typed-node` has arms only for
NBin/NIf/NLet/NVar, and the fallback `Option.Some(_)` catch-all (just reworded in PR#898/`53a839be8`)
explicitly names NApp as UNHANDLED (→ TErr via the leaf slice). So the claim "a param binder's type was
seeded by its NApp arm" describes a producer that doesn't exist in this slice — misleading. Fix (Copilot's,
sound): describe only what this slice guarantees (let-bound vars via demand-let), and phrase the param
case conditionally / as a future producer (e.g. "a param binder's type will be seeded by its NApp arm
once that producer lands" or drop the clause). Comment-only, behavior-neutral (the code correctly reads
`var-type` off the column; a param binder with no seeded type falls to whatever `var-type` returns, not a
claimed NApp arm).

Owner: **v-compiler-ml** (compiler-ml port source, code-shape/comment of their own `53a839be8`). Comment
reword — same file/slice they just addressed for PR#898.
