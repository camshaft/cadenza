# PR #2156 review — rcdzc/src/effects.rs (v-effects) — OPEN — reasoning/invariant-doc [PLAUSIBLE-MED] (folds MY #2120)

https://github.com/camshaft/cadenza/pull/2156 (op-arg let-lift #2120 review cleanup — de-dup perform
traversal + document the #cv-uniqueness + depth-bound invariants; 3 LOW, behavior-preserving; folds MY
#2120 review). Copilot 1 inline flags that the newly-DOCUMENTED uniqueness invariant is justified by a
FALSE premise. Note the recursion: this comment even cites "github-liaison/Copilot #2120 review: confirmed
unique" — my own prior review is being leaned on for a claim that's now challenged.

## the new comment claims the `#`-prefix makes `#cv…` "unspellable" (blocks source forgery), but the ML surface CAN spell arbitrary identifier text via backtick-names (`` `#cv0` ``) → the uniqueness argument rests on a false premise; it should rely on the monotonic StructId ALONE and treat `#cv` as a reserved-prefix CONVENTION, not a parser-enforced guarantee (Copilot, effects.rs:4622) — reasoning/invariant-doc [VERIFIED false-premise; collision-reachability PLAUSIBLE-MED]
> This comment claims the `#` prefix makes `#cv…` "unspellable" and therefore impossible for user code to
> collide with. However, the ML surface can spell arbitrary identifier text via backtick-names (e.g.
> `` `#cv0` ``), so `#cv…` is not a parser-enforced unforgeable namespace; the uniqueness argument should
> rely on the monotonic `StructId` only, and treat the `#cv` prefix as a convention/reserved prefix
> instead of a guarantee.

VERIFIED the false premise against SOURCE. The new comment (#2156 diff:23-24): "`#cv{StructId}` is a
globally-unique fresh binder: the `#`-prefix blocks source forgery (unspellable)". But
`read_backtick_name` (cadenza-syntax/src/lexer.rs:370-395) accepts ANY character between backticks (loops
`bump()` until the closing `` ` ``, with `\`-escaping) → `` `#cv0` `` is a fully spellable `BacktickName`
token (the lexer's own doc: "the lossless escape for symbolic/keyword names", token.rs:36; test
`` `|` `` → BacktickName, lexer.rs:874). So `#cv…` is NOT an unforgeable namespace — a user CAN write
`` `#cv0` ``. The comment's stated guarantee ("cannot collide with a user binder … the `#`-prefix blocks
source forgery") is FALSE as written.

WHAT ACTUALLY HOLDS (and the fix): the real uniqueness rests on the MONOTONIC StructId — `#cv{a.0}` where
`a.0` is the arg node's arena index (`StructId(structure.len())`, never reused). That part is sound. But
the collision-SAFETY-vs-user-code then depends on whether a user can spell `` `#cv{n}` `` whose decoded
`Leaf::Name("#cvN")` equals a live lift site's name for the SAME n — which, since backtick-names decode to
the raw text (`unescape_backtick_name`, printer.rs:7712), is at least SPELLABLE. Whether it's REACHABLE as
a real capture (user binder `` `#cv0` `` shadowing/aliasing a lift at StructId 0 in the same scope) is deep
binder-hygiene semantics — PLAUSIBLE-MED, v-effects' call. Either way the COMMENT is wrong: it asserts a
parser-enforced guarantee that the lexer doesn't provide. Fix per Copilot (sound): rely on the monotonic
StructId as the uniqueness argument, and describe `#cv` as a RESERVED-PREFIX CONVENTION (+ if capture is
actually reachable, gensym-check or reject a user `` `#cv…` `` binder). This is behavior-preserving PR
cleanup, so it's the DOC/reasoning that's in scope here — but getting the invariant's justification right
matters precisely because #2120 folded it as "confirmed unique". v-effects owns rcdzc effects. PR OPEN →
foldable. (Also: this is the third `#cv` review in the chain — worth v-effects settling the capture
question definitively so the comment can state the TRUE guarantee.)
