# DESIGN: Nominal newtype erasure (`rcdzc`)

Status: in progress. Worktree `.claude/worktrees/nominal-newtype`, branched off `spec`.
Baseline at start: `930 pass, 292 todo, 0 fail`.

## The idea

A **nominal record / struct** is `§Nominal Is An Orthogonal Modifier Over Any Structural Type`
(`spec/capabilities/type-system.md`): a program tags any structural type — record, tuple, or sum —
with a name; the value is "its underlying structural value **together with a compile-time tag** …
the tag adds **nothing** to the value's runtime representation."

The realization is deliberately minimal: **a single-variant sum, with the box erased at runtime.**

```
(type UserId (Mk Int64))     ; a newtype over Int64  — value IS an i64 at runtime
(type Point  (Mk Int64 Int64)) ; a struct of two fields — value IS the payload tuple
(type Marker (The))          ; a unit tag — value IS unit
```

A single-variant sum's discriminant is always `0`, so it carries no information. Today such a sum
already **types, constructs, and matches** through the ordinary sum path (verified: `(type Wrap (Mk
Int64))` … `(match (Mk 42) ((Mk n) …))` → `43`, constant and runtime-payload) — it is merely
**boxed**: `sum-new(0, box(payload))`. This work removes that box.

Because the disc is erased and the tag is compile-time-only, a newtype tags **anything, even an
Int** — exactly the orthogonal-modifier spec. This is the `newtype`/`struct` feature and the
runtime-representation optimization in one move.

## Empirically confirmed before starting

- Single-variant sum **runs today** (boxed): constant + runtime payload both → correct value.
- Multi-variant sums are the untouched baseline (`(type E (A Int64) (B Int64))` matches fine).
- `(type UserId (UserId Int64))` — type name == variant name — currently fails **CDZ0203**
  ("cannot apply a value of type UserId"): the monomorphic type record has no `(meta apply)`, so
  `(UserId 42)` tries to apply the *type value*. The idiomatic same-name `newtype` spelling is a
  **separate, optional** synthesis fix (see Edge 2); the core work uses a distinct variant name.
- `valtype_of(ty: &Ty)` (`backend/wasm/lir.rs:259`) takes **only `&Ty`, no `Db`** and maps every
  `Ty::Sum` → `i32` handle. Erasing the box makes a `(Mk Int64)` value a **raw i64**, so the *type*
  must know the erased representation or the slot mismatches → invalid wasm (the
  narrow-value-normalization miscompile class). This is why we add a type-level wrapper rather than
  a lowering-only hack.

## Representation choice: `Ty::Nominal { decl, name, inner }`

Add a new `Ty` variant wrapping the underlying structural type:

```rust
Nominal { decl: StructId, name: String, inner: Box<Ty> }
```

- **Identity is opaque** — `unify`/`agrees_with` compare by `decl` **only** (reuse the `Ty::Sum`
  logic: same decl ⇒ same type; distinct decls with identical shape stay distinct, per
  `§Nominal Types Are Not Comparable Across Their Boundary`). A `Ty::Nominal` never unifies with a
  bare `inner` — forging is rejected.
- **Representation is transparent** — `valtype_of(Nominal{inner,..}) = valtype_of(inner)`, no `Db`
  needed. Boundary encode/decode, field/element access, and pattern binding all see through `inner`.
- Generalizes for free to nominal-tuple and nominal-sum (future); "tag anything, even ints" lives
  here naturally.
- Cost: a new `Ty` variant ⇒ a **Rust-backend arm** too (`backend/rust/`), per the repo trap that
  every new `Ty`/`Core`/`Prim` variant needs one.

Rejected alternative: thread `Db` into `valtype_of` (+ 33 callsites) and derive the underlying type
on demand. More churn, re-derives every call, and doesn't express "tagged" in the type. Not taken.

## Which sums erase (the predicate)

A `TypeDecl` is an **erasable newtype** iff:
1. it has **exactly one variant**, AND
2. that variant's payload types **do not reference the declaration itself** (directly or nested).

Condition 2 excludes a recursive single-variant sum — `(type Stream (More (Tuple Int64 Stream)))` —
whose erased `inner` would be an infinite `Ty`. Those stay **boxed as ordinary sums**
(decline-to-erase, not miscompile; matches repo discipline). Every real newtype (UserId, Meters,
Email, Point) satisfies both. Generic single-variant sums (`(type Box (Mk a))`) are in scope —
`inner` carries the type args like `Ty::Sum` does.

The underlying `inner` type of an erasable newtype:
- 0 payloads → `Ty::Unit`
- 1 payload  → that payload's solved `Ty`
- n payloads → `Ty::Tuple([payload tys…])` (matches how `Core::SumNew` already packs multi-payloads)

## Increment plan

Land each increment green (`cargo xtask gate` 0 fail, `--check` clean, `cargo test -p rcdzc`).
Steps 3 and 4 are coupled — the solved type and the lowering must flip together or a slot mismatches;
land them as one increment.

- [ ] **N1 — `Ty::Nominal` variant.** Add the variant; `unify`/`agrees_with` by `decl`;
  `has_free_var`/render/`walk` descend `inner`; Rust-backend type arm. Pure type-layer, **byte-neutral**
  (nothing produces a `Nominal` yet) — the gate must stay identical. Mirrors the `Ty::Float` "type
  layer, byte-neutral" landing shape.
- [ ] **N2 — `Db` newtype predicate + underlying type.** `Db::newtype_underlying(decl) -> Option<Ty>`
  (returns `Some(inner)` for an erasable newtype, `None` otherwise). Unit-tested; wired to nothing yet.
- [ ] **N3+N4 — solve + lower together (the core).**
  - Solve: a newtype constructor's **result type** and a newtype-typed value resolve to `Ty::Nominal`
    (not `Ty::Sum`); a `(Mk x)` construction and a `(Mk n)` pattern read the nominal.
  - Lower/select: for an erasable newtype, **skip `sum-new`/`sum-payload`/disc** — construction emits
    just the payload (1) / the payload tuple (n) / unit (0); the match arm binds the scrutinee value
    directly. `valtype_of` reads `inner` (N1) so slots agree.
  - Exhaustiveness: a single-arm match on a newtype is exhaustive (one variant).
- [ ] **N5 — boundary.** Encode/decode a `Ty::Nominal` value across the run boundary as its `inner`
  (`comp_valtype_of`, host render). A `UserId` prints as its underlying value.
- [ ] **N6 — corpus + bench.** Cases in `07-type-system.sexp` (nominal sections) and
  `05-compound-types.sexp`: newtype over int, struct (multi-payload), unit tag, generic newtype,
  identity-not-forgeable (two same-shape nominals don't unify), recursive-single-variant stays boxed.
  `cargo xtask bench` must show a newtype-over-int construction with **zero** heap allocs (vs. the
  boxed sum's one) — the proof the box is gone.

## Optional follow-up (not blocking)

- **E2 (same-name spelling).** Make `(type UserId (UserId Int64))` construct instead of CDZ0203 — the
  idiomatic newtype form. In synthesis, give a single-variant type record the variant's `(meta apply)`
  (or let the variant field shadow the type name in application-head position). Scoped separately so
  the core lands first.

## Traps to respect (from repo memory)

- Land only in this worktree; merge to `spec` via guarded CAS (`git update-ref`), never touch main's tree.
- `cargo xtask build` a FRESH runtime before gating, or heap-case verdicts are a false alarm.
- New `Ty`/`Core`/`Prim` variant ⇒ Rust-backend arm (`backend/rust/expr.rs` + `types.rs`).
- A node-synthesizing fold must be depth-bounded; the recursion-erasure predicate (Cond 2) is the
  guard against an infinite `inner`.
- Diff the FAIL set, not the pass count (P/todo drift).
