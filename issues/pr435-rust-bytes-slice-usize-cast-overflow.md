# PR review comment — mirrored from GitHub PR #435 (Copilot inline)

- **PR:** #435 "fleet: batch 61+62 (rust-backend BYTES S2, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:1162` (`Bytes.slice`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592380449
- **Link:** https://github.com/camshaft/cadenza/pull/435#discussion_r3592380449

## Comment (verbatim)
> `Bytes.slice` Rust backend uses `(__start as usize) + (__len as usize)` to bounds-check. For large positive i64 values this `as usize` cast can wrap (or the `usize` addition can overflow/wrap in release), potentially letting an out-of-range slice through and panicking on the `__v[start..end]` indexing instead of returning `None`.

## Liaison triage — CONFIRMED against trunk
Confirmed in expr.rs: the emitted `Bytes.slice` check is
`if __start >= 0 && __len >= 0 && (__start as usize) + (__len as usize) <= __v.len() { Some(__v[…]) } else { None }`.
The `(__start as usize) + (__len as usize)` ADDITION can overflow `usize` (wraps in release) for two
near-`i64::MAX` operands → the `<= __v.len()` guard passes on the wrapped small sum → the subsequent
`__v[(__start as usize)..(__start as usize)+(__len as usize)]` indexing PANICS instead of returning
`None` (Bytes.slice should be total/fallible). Rust-backend correctness edge. FIX: use checked addition
(`(__start as usize).checked_add(__len as usize).is_some_and(|end| end <= __v.len())`) or compare
against `__v.len()` without summing. v-rust-backend. Fix on `trunk`. Quote + link in queue file.
