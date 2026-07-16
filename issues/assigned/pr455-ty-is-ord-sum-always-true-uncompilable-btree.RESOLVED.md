# PR review comment — mirrored from GitHub PR #455 (Copilot inline)

- **PR:** #455 "fleet: seventy-fifth batch (…, rust-backend float-keyed-map decline, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/rust/types.rs:165` (`ty_is_ord`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3593216254
- **Link:** https://github.com/camshaft/cadenza/pull/455#discussion_r3593216254

## Comment (verbatim)
> `ty_is_ord` treats `Ty::Sum` as always `Ord` (`_ => true`), but user sums only derive `Ord` when `sum_derives_eq` is true (see `backend/rust/enums.rs`). A sum carrying a float (or other non-`Eq` payload) will emit an enum that does *not* derive `Ord`, so a `Set`/`Map` keyed by such a sum can still slip past this check and the backend will emit `BTreeSet<Enum>`/`BTreeMap<Enum,_>` that fails to compile. This needs a db-aware `Ord`-derivability check for sums (or a conservative decline) so the Rust backend reliably declines instead of producing uncompilable output.

## Liaison triage — CONFIRMED against trunk
Confirmed in types.rs `ty_is_ord`: a `Sum`/`Nominal` is treated as `Ord` (`_ => true`), with a comment
asserting a float-carrying sum "would be caught by the enum-derive path, not here." But `ty_is_ord` is
precisely the GUARD meant to DECLINE a non-Ord key BEFORE the backend emits a `BTreeMap<Enum,_>`. If a
sum's emitted enum doesn't derive `Ord` (a float / non-`Eq` payload), returning `true` here lets the
backend emit `BTreeSet<Enum>`/`BTreeMap<Enum,_>` that **fails to compile** — an uncompilable-output
failure rather than the intended clean decline (the whole point of the float-keyed-map decline this PR
added). NOTE: `ty_is_ord` is currently pure (no `Db`), but sum-Ord-derivability needs `Db`
(`sum_derives_eq`). FIX: make the sum case `Db`-aware (decline when the sum's enum won't derive `Ord`)
or conservatively decline sums with non-scalar payloads. v-rust-backend. Fix on `trunk`. Quote + link in
queue file.
