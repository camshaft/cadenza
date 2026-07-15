# DESIGN: recursive newtype erasure (nominal identity = decl + args)

Status: in progress. Recursive newtypes currently STAY BOXED (correct, one heap box per cell). This
plan erases them, grounded in what the spike + a failed first attempt taught.

## The real problem — equirecursive type equality, NOT infinite types

The "infinite inner type" fear was wrong: a recursive newtype's self-reference decodes to a
`Ty::Sum { decl }` LEAF (finite — variant set lives in `db.type_decls`, not inline), which is the
μ-binder. So `(type Lst (Mk (Option (Tuple Int64 Lst))))` has the FINITE inner `(Option (Tuple Int64
Ty::Sum{Lst}))`.

The spike's "2 backend arms" (`box_op_ty`/`get_op_ty` handling `Ty::Nominal`) make a recursive newtype
CONSTRUCT/MATCH/TRAVERSE — but that is incomplete. The wall is that the SAME recursive type appears in
two representations:
- **folded** (annotation / template path): `Nominal{Lst, inner: (Option (Tuple Int64 Ty::Sum{Lst}))}`
- **unfolded** (a value's type, built bottom-up from real sub-values): `Nominal{Lst, inner: (Option
  (Tuple Int64 Ty::Nominal{Lst}))}`

`unify`/`agrees_with`/`join` recurse STRUCTURALLY into `inner`, so folded ≠ unfolded → "type Lst does
not match type Lst". Canonicalizing `inner` at `normalize_sum` failed because value types are built
bottom-up at every construction site (whack-a-mole; the ctor-scheme-instantiation path reconstructs it
un-canonically). This is the classic μ-type equality problem.

## The design decision

**Compare `Ty::Nominal` by `decl + args`, NEVER by structural `inner`.** This is:
- how `Ty::Sum` already sidesteps recursion (identity = decl + args, variants looked up on demand);
- SPEC-ALIGNED — `type-system.md §A Nominal Type's Identity Is Its Fully-Qualified Name`: nominal
  identity is nominal (decl), not structural. Comparing `inner` was always slightly wrong — it only
  worked for non-recursive types because their inners never diverge.

Once identity is `decl + args`, folded-vs-unfolded compare EQUAL trivially (both are `Lst`, args `[]`),
and the whole μ-equality problem dissolves — no coinduction, no canonicalization.

`inner` stays on `Ty::Nominal` as a machine-rep HINT (depth-1 shape for `valtype_of`/`box_op_ty`/field
access, which take `&Ty` and cannot reach `Db`), but is EXCLUDED from equality. Since it is never
compared, the folded/unfolded divergence is harmless — both yield the same valtype.

New shape: `Ty::Nominal { decl, name, args, inner }` — `decl + args` is identity; `inner` is derived,
never compared.

## Phases (each gated + committed separately)

### Phase 0 — investigation (decides feasibility of the repr change) — ✅ DONE
FINDINGS:
- `Ty` derives `Clone, PartialEq, Eq, Debug` — **NO `Hash`**. `Ty` is NEVER a HashMap/HashSet key
  (the only `HashSet<BaseType>` is a DWARF-local, not `Ty`). So a custom `PartialEq` carries NO
  Hash-consistency obligation — the biggest feared risk is ABSENT.
- NO whole-`Ty` `==` anywhere: `agrees_with`/`join` compare `decl == decl` (a `StructId`) then recurse;
  no `self == other` on a full `Ty`. The only invokers of derived `PartialEq<Ty>` are test `assert_eq!`
  on `newtype_underlying` (we control those). `ty == "Any"` in sidecar.rs is a `&str`, not `Ty`.
- `Ty::Nominal` producers are centralized: `Db::normalize_sum` (both decode + generic-ctor paths route
  through it) + `resolve::decode_ty`'s `Nominal` wire arm + `eval::encode_ty`. Plus `unify.rs`'s
  `apply`/`rename`/`freshen_free` rebuild it structurally (mechanical `args` threading).
DECISION: implement a **custom `PartialEq for Ty`** comparing nominals by `decl + args` (drop
`inner` from equality) as the single source of truth, so `agrees_with`/`unify` are consistent with it.
No `Hash` to keep in sync. This is CLEANER than the doc's worst case (no map-key breakage).

### Phase 1 — representation refactor, BYTE-NEUTRAL — ✅ DONE (`spec`@Phase1)
- [x] Added `args: Vec<Ty>` to `Ty::Nominal`; threaded through every construction (via `normalize_sum`).
- [x] Comparators compare by `decl + args`, not `inner`: `unify`, `agrees_with`, `join`, `occurs`,
  `has_free_var` (all walk `args` like `Ty::Sum`).
- [x] Hand-written `PartialEq for Ty` (dropped the derive) — nominal by `decl + args`; `Eq` marker; no
  `Hash` needed. `encode_ty`/`decode_ty` wire form gained the `(args …)` group.
- [x] GATE: byte-neutral (no regressions, 970 tests). Generic distinctness preserved via `args`.

### Phase 2 — flip recursion on — ✅ DONE (`spec`@Phase2)
- [x] Removed the `reaches_decl` box-guard (deleted the fn) — every single-variant sum erases.
- [x] Added the `Ty::Nominal` arms to `box_op_ty`/`get_op_ty`.
- [x] `valtype_of`/`rust_type` read through `inner` and terminate on the `Ty::Sum` leaf (finite).
- [x] ⚠ FIXED A CACHE-POLLUTION BUG the recursion exposed: the load-time `newtype_underlying` walk
  called `typeval_of` on a recursive self-reference BEFORE `newtype_inner` was populated, memoizing
  `Resolved::TypeVal(Ty::Sum{decl})` (the PRE-normalization boxed form) on the shared sum-record
  `(meta t)` node — so a later annotation `(: x Lst)` read that stale `Ty::Sum` and failed to unify with
  the value's `Ty::Nominal` ("type Lst does not match type Lst"). FIX: `db.resolved`/`db.types` reset to
  empty after the precompute (it is the only thing that touched them in `load`; re-decoded on demand).
- [x] GATE: `(: (Mk …) Lst)`, direct traversal, MUTUAL recursion, let-bound recursive — all pass; escape
  still declines cleanly. Gate 1271/0.

### Phase 2 (original gate line, kept for the record):
- GATE: `(: (Mk …) Lst)`, direct traversal, MUTUAL recursion `(type A (Mk B)) (type B (Wrap A))`,
  recursive+generic all pass. No unify mismatch. Escape still declines cleanly (Phase 3).

### Phase 3 — recursive-newtype HOST-ESCAPE walker — ✅ DONE (`spec`@Phase3)
- [x] `ShapeTableBuilder::shape_of` gains a `Ty::Nominal` arm: reserve an entry keyed by the nominal's
  `decl` (a recursive newtype's inner re-references it → closes to a `Ref`), build the inner shape, fill
  `Named(<type name>, inner)`. The inner's `Ty::Sum{decl}` back-edge resolves to the SAME reserved entry
  (shared `decl` key), so the table is finite.
- [x] `sum_shape_descriptor` accepts a `Ty::Nominal` (its `shape_of` already roots at `Named`, so encode
  directly — no double-wrap).
- [x] `wasm::emit` routes an un-stripped nominal-over-recursive-sum to the walker BEFORE the stripped-sum
  path, so the top-level tag is the newtype's OWN name (`Lst`), not the inner sum's (`Option`).
- [x] VERIFIED: `(type Lst (Mk (Option (Tuple Int64 Lst))))` escapes as `(: (Some (tuple 7 (: (None unit)
  Lst))) Lst)`; a 3-cell chain nests correctly; MUTUAL recursion escapes with both names at their
  positions; recursive-SUM + non-recursive-newtype escapes unregressed. Gate 1285/0.

## Result: recursive newtype erasure COMPLETE
All three phases landed. A recursive (and mutually-recursive) newtype now erases its per-cell box,
constructs/matches/traverses, type-checks across folded/unfolded reps, AND returns to the host. The
"needs μ-machinery / a new Ty variant" fear was wrong — the fix was `decl+args` nominal identity (Phase 1)
+ two backend arms + a cache-eviction (Phase 2) + a shape-builder nominal arm (Phase 3).
⚠ FLAKE NOTE: the `rustc_roundtrip_*` rust-backend tests shell out to `rustc` and flake under a highly
parallel `cargo test` (a different subset fails each run; each passes in isolation / with bounded
threads). Not a regression — the gate (release) and `cargo test backend::rust::` alone are the true
signals.

## Fallback
If the repr refactor spiders too far (Phase 0), fall back to COINDUCTIVE comparison: thread an
`assumed: &mut Vec<(StructId, StructId)>` through the comparators — when `Nominal{d}~Nominal{d}` is
already on the stack, return equal. Smaller `Ty` change, but keeps the fragile inner-comparison and
changes every comparator signature. Take only if the repr change is infeasible.

## Traps (from the failed first attempt)
- Value types are built BOTTOM-UP; do NOT try to canonicalize `inner` at production sites.
- `unify`/`agrees_with`/`join` are PURE over `Ty` (no `Db`) — they cannot ask "is this decl
  recursive?"; the fix must live in the data (decl+args identity), not a Db-consulting comparator.
- A recursive newtype's escape must DECLINE, never miscompile, until Phase 3.
