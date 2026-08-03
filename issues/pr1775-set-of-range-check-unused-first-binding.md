# PR #1775 review comment — rcdzc/src/infer.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1775 (extend sibling-inferred-width CDZ0302 check).

## `first` bound-but-unused + `let _ = first;` noise in the homogeneous Set.of range-check (Copilot, infer.rs:10076) — cleanliness
> `first` is bound but unused in the homogeneous `Set.of` range-check block, and the `let _ = first;`
> workaround adds noise. Prefer destructuring with `..` (or `_`) so the binding isn't introduced.

Bind only what's used — replace the `first` binding + `let _ = first;` with a `..`/`_` destructure so
there's no unused binding to suppress. LOW/cleanliness. Fix-forward.
