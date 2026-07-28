# PR#755 review comment — is_trap_free `Core::Record` arm allocates a temp Vec just to iterate (hot fold path)

Mirrored from GitHub PR review comment (Copilot), id `3625590324`.
PR: https://github.com/camshaft/cadenza/pull/755 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/lower.rs:17593`

## Comment (verbatim)

> `is_trap_free`'s `Core::Record` arm allocates a temporary `Vec` just to iterate values. This runs on
> a hot path for fold eligibility (e.g. `List.len` constant-arity fold), so it's worth avoiding the
> extra allocation.

## Liaison verification (CONFIRMED on trunk)

lower.rs:17590-17593:
```rust
Core::Record { fields } => {
    let vals: Vec<StructId> = fields.values().copied().collect();
    vals.into_iter().all(|v| is_trap_free(db, v))
}
```
The throwaway `Vec` is unnecessary — the immediately-following arms iterate directly:
```rust
Core::Tuple { elems } | Core::ListNew { elems } => elems.into_iter().all(|e| is_trap_free(db, e)),
Core::SumNew { payloads, .. } => payloads.into_iter().all(|p| is_trap_free(db, p)),
```
`is_trap_free` runs on the fold-eligibility hot path (e.g. the `List.len` constant-arity fold). Fix:
`fields.values().copied().all(|v| is_trap_free(db, v))` (or `fields.values().all(|&v| …)`) — no temp
Vec. Borrow note: `fields.values()` yields `&StructId`; `.copied()` gives `StructId` by value, and
`is_trap_free(db, v)` takes it by value, so a direct `.all()` type-checks without the collect.

Compile-time efficiency cleanup, behavior-neutral. Owner: v-core-opt (broadened is_trap_free to pure
constructors in `0434021a8`). Same shape as the PR#707 lower.rs classification alloc nits. Routed as a
note.
