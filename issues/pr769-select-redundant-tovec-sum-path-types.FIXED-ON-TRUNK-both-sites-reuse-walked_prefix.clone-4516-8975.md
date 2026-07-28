# PR#769 review comments — select.rs rebuilds `prefix[..k].to_vec()` twice for the sum_path_types key (redundant alloc)

Mirrored from GitHub PR review comments (Copilot), ids `3627883606`, `3627883630`.
PR: https://github.com/camshaft/cadenza/pull/769 (batch-staging; fix belongs on trunk)
Locations:
- `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:4485`
- `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:8945`

## Comments (verbatim)

- (id 3627883606, :4485) "This branch allocates `prefix[..k].to_vec()` twice (once for
  `walked_prefix`, then again for the `sum_path_types` lookup). Since this is in the per-prefix
  fast-path setup, it's worth avoiding the redundant allocation by reusing `walked_prefix.clone()`
  for the key."
- (id 3627883630, :8945) "`sum_path_types` lookup rebuilds `path[..k].to_vec()` even though the same
  prefix was already materialized into `walked_prefix` just above. Reusing `walked_prefix.clone()`
  avoids an extra allocation on the binder fast path."

## Liaison verification (CONFIRMED on trunk/staging)

Both sites build the SAME slice twice:
- select.rs:4481-4484:
  ```rust
  walked_prefix = prefix[..k].to_vec();
  cur = out.sum_path_types.get(&(scrutinee, prefix[..k].to_vec())).cloned().unwrap_or(Ty::Any);
  ```
- select.rs:8938-8943: identical with `path[..k].to_vec()`.

`prefix[..k].to_vec()` / `path[..k].to_vec()` is materialized into `walked_prefix` and then rebuilt for
the `sum_path_types` HashMap key. Reuse `walked_prefix.clone()` for the key → one alloc instead of two,
on the per-prefix / binder fast path. Behavior-neutral compile-time efficiency cleanup (same class as
the PR#707/#755 lower.rs alloc nits).

Owner: v-inference (emit-type-selection lane; `sum_path_types` scrutinee-keying landed `2b42e4f79`).
Routed as a note. Minor.
