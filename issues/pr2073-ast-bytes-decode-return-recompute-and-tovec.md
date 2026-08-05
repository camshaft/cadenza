# PR #2073 review — rcdzc/src/lower.rs (v-metaprogramming) — OPEN — robustness/cleanup [VERIFIED, LOW] (the fix for my #2063)

https://github.com/camshaft/cadenza/pull/2073 (Ast.encode/decode round-trips Ast.Bytes — the B2a fix that
closes my #2063 MED codec gap). Copilot (id 3714651736) flags a residual overflow-recompute + a needless
copy in the new `AST_TAG_BYTES` decode arm.

## the `AST_TAG_BYTES` decode arm guards the slice with `checked_add` but the RETURN recomputes `1 + 4 + len` unchecked; `.to_vec()` needlessly copies (Copilot, lower.rs:3805) — robustness/cleanup [VERIFIED, LOW]
> `decode_ast_value`'s `AST_TAG_BYTES` arm computes the consumed byte count with `1 + 4 + len`, which can
> overflow `usize` on 32-bit targets for large untrusted `len` … (debug builds would panic, violating the
> "never-panic on untrusted input" invariant). Also, `to_vec()` unnecessarily copies the payload;
> `bytes_to_elems` already accepts a slice.

VERIFIED with a nuance. The arm DOES guard the slice: `let end = 4usize.checked_add(len)?; let raw_bytes =
rest.get(4..end)?...` — so the slice read is safe (a huge `len` → `checked_add` → `None`, no panic). BUT
the RETURN is `Some((node, 1 + 4 + len))` — recomputing `1 + 4 + len` UNCHECKED rather than reusing `end`.
By that point `checked_add(len)` proved `4 + len` didn't overflow, so `1 + 4 + len` overflows only in the
exact `4 + len == usize::MAX` corner — narrower than Copilot implies, but (a) it's inconsistent (guard the
slice with `checked_add`, then recompute unguarded) and (b) it CAN debug-panic on wasm32 at that corner,
against the arm's own never-panic comment. Clean fix: `Some((node, 1 + end))` — reuses the already-checked
value, provably non-overflowing (well, `1 + end` — `end ≤ usize::MAX`, so make it `1usize.checked_add(end)?`
if paranoid, or note `end` is bounded by `rest.len()` since `rest.get(4..end)?` succeeded, so `1 + end ≤ 1 +
rest.len()` can't overflow — the SLICE-SUCCESS bound is the real guarantee, worth a comment). And drop
`.to_vec()`: `bytes_to_elems(db, rest.get(4..end)?)` takes the slice directly, avoiding the copy. LOW —
narrow overflow corner + a copy, both cheap to fix, and it's the fix for my #2063 so worth getting clean.
(The sibling AST_TAG_INT arm at :46 also returns `1 + 4 + len`, but there `len` is a magnitude length
bounded by a successful prior read — same slice-success reasoning; a shared helper or comment would settle
both.) v-metaprogramming owns rcdzc codec. PR OPEN → foldable.
