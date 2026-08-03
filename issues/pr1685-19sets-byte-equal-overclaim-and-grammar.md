# PR #1685 review comments — spec/semantics/19-sets.sexp (v-runtime) — OPEN

https://github.com/camshaft/cadenza/pull/1685 (pin the THREE-way CHAMP collision node — 19-sets/#1650
lineage; author v-runtime verified, NOT corpus-bugfix).

## 1. Docstring claims "byte-equal" but the check only uses `=` (content equality) (Copilot, :3207, also :3242) — doc/accuracy
> The docstring claims the post-remove single-key set must be "byte-equal" and that leftover collision
> structure would be detected by the equality check, but the code only uses `=` (set equality is content-
> defined earlier in this file). The ones-digit check can't witness internal canonicalization; it only
> witnesses content equality.

Valid — `=` on Sets is content equality (per the file's own definition), so it CANNOT observe leftover
collision-node structure vs a clean single-key set (both compare equal by content). The docstring
overclaims that the check witnesses canonicalization. Reword to what's actually asserted (content
equality), or if canonicalization-witnessing is wanted it needs a different probe. Recurs at :3242.
LOW/doc-accuracy.

## 2. "exercises faces" reads like a typo across the line break (Copilot, :3174) — doc/grammar
> "exercises faces" reads like a typo. Rephrase so the sentence reads naturally across the line break.

LOW/grammar. Fold both into the next 19-sets edit per the no-standalone-polish steer.
