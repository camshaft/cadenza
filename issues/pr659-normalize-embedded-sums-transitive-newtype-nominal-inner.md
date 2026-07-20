# pr659 — normalize_embedded_sums may leave a keyed Ty::Sum in a transitive Nominal.inner (miscompile risk) (2 Copilot)

Mirrored from GitHub PR #659 review comments (Copilot). VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/659 (v-inference feature stack — the "9939" newtype normalization)
File: `implementation/seed/crates/rcdzc/src/db.rs`. Both comments = ONE finding (the pass + its test).

## #1 — id 3612201006 (db.rs:5579) — pass doesn't normalize Nominal.inner → transitive residual
> `normalize_embedded_sums` deliberately does not rewrite `Ty::Nominal.inner`, but multiple backend paths
> treat `Ty::Nominal` as its `inner` representation (wasm `valtype_of` / `is_heap_type` recurse into
> `inner`). If newtype A's template references B, and B's (pre-renorm) template references C, then
> rewriting A can bake a `Ty::Nominal{decl=B, inner=...}` whose `inner` still contains a raw
> `Ty::Sum{decl=C}` — reintroducing the "treat erased newtype as boxed handle" miscompile at transitive
> depth. Consider normalizing `Nominal.inner` with a recursion guard (visited decl set), or compute
> templates in dependency order / to a fixpoint.

## #2 — id 3612201019 (db.rs:5830) — assert_no_keyed_sum test helper doesn't descend Nominal.inner
> The `assert_no_keyed_sum` helper explicitly does not descend `Ty::Nominal.inner`, so the new tests can
> miss residual keyed-newtype `Ty::Sum` inside `Nominal.inner`. Add a transitive-chain regression (A wraps
> B, B wraps C, declaration order reversed) and extend the assertion to walk `Nominal.inner` with a cycle
> guard.

## VERIFIED + the crux (needs OWNER judgment — a real soundness Q, not obviously wrong)
Both the pass (db.rs:5581) and the test helper (db.rs:5831) DO deliberately stop at `Nominal` without
descending `inner`, and the code documents WHY: "`normalize_sum` re-derives `inner` from the decl's OWN
(separately-rewritten) stored template ... descending it would loop on a RECURSIVE newtype's Sum back-edge
forever." So the design's correctness HINGES on the invariant: when A is rewritten, B's and C's templates
are ALREADY normalized (their own map entries rewritten), so A's baked `Nominal{decl=B, inner=...}` gets a
clean inner. Copilot's concern is exactly whether that separate rewrite is GUARANTEED complete before A's —
i.e. are templates rewritten in dependency order / to a fixpoint, or could a reversed decl-order chain
(A→B→C) bake a `Nominal{decl=B, inner=<raw Sum decl=C>}`? If the rewrite is NOT order-independent, this is a
real transitive miscompile of the SAME family as [[int-ty-of-missing-strip-nominal-narrow-newtype-literal-box-invalid-wasm]]
(erased newtype read as a boxed handle). I can't determine from the pass alone whether the map is rewritten
to a fixpoint — that's the owner's call.

## Owner
`db.rs` normalize_embedded_sums / 9939 = v-inference (owns this, the PR is its stack). Route there:
confirm templates are rewritten in dependency order / to a fixpoint (so `Nominal.inner` can't retain a keyed
`Sum`); if not guaranteed, either normalize `inner` with a visited-decl guard or fixpoint the map — AND add
the transitive A→B→C reversed-order regression + extend `assert_no_keyed_sum` to walk `inner` with a cycle
guard. If the fixpoint invariant already holds, a comment/test pinning it closes the concern.
