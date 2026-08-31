# DESIGN — Reflect a `Type` value to the `Ast` of its definition (`Type.ast` / `Type.ast-generic`)

> **Status:** Phase-1 design (draft for review). Subsystem: `rcdzc` (with a spec touch to
> `spec/capabilities/metaprogramming.md` / `type-system.md`, coordinated with the spec owner).
> Produced by the `design-type-to-ast` design agent from an operator spark; hand-off item for a
> `vertical` owner to land top-to-bottom.
>
> **Operator intent (verbatim):** "One thing I was thinking would be cool is being able to get an
> AST value from a Type value at compile time. So it would return the AST of the type definition."
>
> **Decisions pinned with the operator (this design session):**
> 1. The reflected `Ast` is the **verbatim declaration form** — the `Ast` of the original
>    `(type Name (V1 pay1) …)` source, reusing `TypeDecl.occ` — not a synthesized descriptor.
> 2. Surface: an associated field on the existing `Type` reflection module (mirrors `Type.of` /
>    `Type.eq`), **kebab-case** to match the language's naming.
> 3. It is **total over concrete types**: nominal/sum → its decl AST; structural
>    record/tuple/`List`/`Map`/`Set`/primitive/`Fn` → the canonical type-surface AST; it **declines
>    only on non-concrete types** (an unresolved type variable).
> 4. **Two functions**, so the caller chooses generic vs concrete:
>    - **`Type.ast`** — the **instantiated** definition (the type's own params substituted by its
>      concrete arguments). The common case: you usually hold a concrete type. Short name = default.
>    - **`Type.ast-generic`** — the **generic** definition, verbatim, with type params intact.

## 1. Problem

Types are already first-class values in Cadenza (`spec/capabilities/type-system.md:232` — "Types Are
First-Class Values Whose Type Is The Type Of Types"), and the `Ast` reflection sum already lets a
program build and pattern-match syntax trees (quote/reify, `Ast.module`, `Ast.print`/`Ast.read`,
`Ast.encode`/`Ast.decode`). But there is **no way to go from a `Type` value to the `Ast` of that
type's *definition*.** The nearest existing bridge, `encode_ty` (`eval.rs:3844`), emits only the
type's **identity/reference** — `(Sum NAME <decl> args…)` / `(Nominal NAME <decl> (args…) inner)`
(`eval.rs:3915-3961`) — deliberately *not* the fields/variants/shape. The definition's shape lives in
`TypeDecl` (`db.rs:580-625`): its `.occ` is the original `(type Name …)` arena node, plus
`.params`/`.variants`.

The goal: **a pure compile-time reflection primitive that, given a `Type` value, returns the `Ast` of
that type's definition** — the missing dual of `encode_ty` (definition, not reference). This closes
the metaprogramming loop: a program can inspect not just *how it was written* (quote) and *what a
module looks like* (`Ast.module`), but *how a type is defined*, then analyze / print / encode /
transform that definition with the ordinary `Ast` machinery.

## 2. Current state (verified this session; anchors on the worktree base)

### 2.1 `Ast` is an ordinary prelude sum
`Ast` is not compiler-special-cased; it is built by the normal sum path (`sums.rs:130-243`,
`ast_decl`). Its variants (`sums.rs:216-233`, discriminants read by name via `ast_variant_discs`):
- scalar leaves `Int / Float / Bool / Str / Name / Bytes / Char / Symbol`;
- the generic node `List ((List Ast))` — for name-headed non-collection forms (`if`/`fn`/`match`/
  application **and a `(type …)` decl form**);
- native-collection ctors `ListCtor / TupleCtor / RecordCtor / MapCtor / SetCtor`, plus `FieldPair /
  Member` and `Rational`.

A quote reifies to exactly these constructors; a `(type …)` decl form reflects as an `Ast.List` whose
children are the reflected head/params/variant sub-forms (each itself an `Ast.List` or `Ast.Name`).
Spec anchor: `spec/capabilities/type-system.md:218-230`.

### 2.2 Types as values, and the `Type` reflection module
- `Ty::Type` is the type-of-types (`ty.rs:926`); a term-level type-value is
  `Resolved::TypeVal(Ty)` (`resolved.rs:1427`, produced at `resolve.rs:543`), stored in the arena as a
  `(typeval PAYLOAD)` node. Bridges: `encode_typeval`/`encode_ty` (`eval.rs:3844`/`:3864`), inverse
  `decode_ty` (`resolve.rs:6396`).
- The `Type` reflection module (`prelude.rs:1860-1875`): `Type.of e` (intrinsic `type-of` →
  `Prim::TypeOf`, reduced at `eval.rs:2872`) yields the type-VALUE of `e`'s inferred type; `Type.eq a
  b` (intrinsic `type-eq`) folds to a compile-time `Bool`. Both recognized **structurally** on the
  reflection module shape, not by the literal name "Type" (`eval.rs:3160-3192`). **This is the exact
  pattern the new fields extend.**

### 2.3 The type-definition AST (what we reflect)
- `TypeDecl` (`db.rs:580-625`): `name`, `occ` (the `(type Name …)` arena node — the definition's
  syntax lives here), `params`, `variants: Vec<Variant>`, `open_tail`, `synth`, `associated`.
- Recover a decl from a `Ty::Sum`/`Nominal`'s `decl` occurrence via `db.type_decl_by_occ(...)` (used
  by `encode_ty` at `eval.rs:3919`).
- The **canonical type-surface renderer** `type_ast` (`lower.rs:1510-1627`) already renders any `Ty`
  (`Sum`/`Record`/`Tuple`/`List`/`Map`/`Set`/`Fn`/`Type`/…) to its surface AST structurally — this is
  the total-coverage fallback for structural types with no `TypeDecl`.

### 2.4 Compile-time evaluation is one tier
Macro expansion, generic reduction, monomorphization, and constant folding are the **same** pure
mechanism (`spec/capabilities/metaprogramming.md:70-82`; reducer in `eval.rs`). The new primitive is a
pure reduction on the same tier — it folds a `(typeval …)` argument to a constant `Ast` value, exactly
as `Type.of`/`Ast.module` fold today. Type substitution under an env (for the instantiated variant)
already exists for generic reduction/monomorphization (`eval.rs:27+`).

### 2.5 binary-AST is the interchange form
The reflected `Ast` value is an ordinary `Ast`, so `Ast.encode`/`Ast.decode` (`core.rs:654-680`) and
`Ast.print`/`Ast.read` work on it unchanged. A constant `Ast` folds to `Core::ConstBytes`
byte-identical to the runtime op via the shared codec (`cadenza-ast/src/codec.rs`). No byte-format
change is needed — this feature adds *no* new `Ast` variant (it reuses `Ast.List`/`Ast.Name`/etc.).

## 3. Surface & semantics

Two associated fields on the `Type` reflection module, both pure, both `Type → Ast`:

```
Type.ast         : Type -> Ast   // instantiated: decl with the type's params substituted by args
Type.ast-generic : Type -> Ast   // generic: the verbatim decl, type params intact
```

Usage:
```
type Color = Red | Green | Rgb Int Int Int
Type.ast (Type.of someColor)
=> (Ast.List #list( (Ast.Name "type") (Ast.Name "Color")
                    (Ast.List #list((Ast.Name "Red")))
                    (Ast.List #list((Ast.Name "Green")))
                    (Ast.List #list((Ast.Name "Rgb") (Ast.Name "Int")
                                    (Ast.Name "Int") (Ast.Name "Int"))) ))
```

For a **parameterized** type the two functions differ:
```
type Pair a b = Pair a b
let p : Pair Int Str = ...

Type.ast-generic (Type.of p)
=> (type Pair a b (Pair a b))        // params a,b stay as names — verbatim .occ

Type.ast (Type.of p)
=> (type Pair (Pair Int Str))        // a->Int, b->Str substituted; param binders dropped
```

### 3.1 `Type.ast-generic` — verbatim decl form
Reflect `TypeDecl.occ` (the original `(type …)` arena node) into an `Ast` value using the **same
node→`Ast.*` reification quote uses** (`quote::reify`, `quote.rs:419/462`). This is a direct reuse: a
`(type …)` form is just another arena form, and reifying it yields the `Ast.List`/`Ast.Name` tree
shown above. Params remain textual names. For a **structural** type (no `TypeDecl` — a bare record,
tuple, `List`, primitive, `Fn`), fall back to reflecting the canonical surface AST from `type_ast`
(`lower.rs:1510`) — there are no params to keep generic, so `Type.ast-generic` and `Type.ast` coincide
for structural types.

### 3.2 `Type.ast` — instantiated decl form
Substitute the decl's own type params with the type-value's concrete arguments, then render:
- **Recommended path (reuse monomorphization):** the `Ty::Sum`/`Nominal` carries the concrete args;
  build the substituted per-variant payload `Ty`s using the existing type-reduction-under-env
  (`eval.rs:27+`), then render the substituted decl via `type_ast` (`lower.rs:1510`) and reflect that.
  This reuses the compiler's own monomorphization substitution rather than doing textual Ast-name
  replacement, so it is correct under shadowing/capture.
- **Param binders are dropped** in the head (the params are now concrete), e.g. `(type Pair (Pair Int
  Str))`. (Open nit — §7.1.)
- For a **non-generic** type, `Type.ast` == `Type.ast-generic` == the decl/surface AST (nothing to
  substitute).

### 3.3 Finiteness under recursion (both functions)
Neither function unfolds **nested named type references**. A recursive
`type List a = Nil | Cons a (List a)` instantiated at `List Int` yields
`(type List (Nil) (Cons Int (List Int)))` — the inner `List Int` stays a `(Name arg…)` application, it
is not expanded. Substitution replaces only the decl's **own** param binders in its **own** body; every
type reference (including the self-reference) remains a named application. So the result is always
finite, even for recursive and mutually-recursive generics.

### 3.4 Totality & the decline
Total over **concrete** types (every `Ty` with no free type variable): nominal/sum → decl AST;
structural record/tuple/`List`/`Map`/`Set`/primitive/`Fn` → canonical surface AST; `Type` itself →
`(Ast.Name "Type")`. It **declines with a compile-time diagnostic** when the argument is *not
concrete* — i.e. still contains an unresolved type variable (e.g. calling it on a polymorphic value
before instantiation). This is a proper type/reduction error ("`Type.ast` requires a concrete type;
found an unresolved type variable"), not the unsupported-tracker path.

### 3.5 Typing
Both fields type as `Type -> Ast` with an **empty effect row** (pure, no host), consistent with the
one-tier compile-time rule (`metaprogramming.md:70-82`). Return type is the prelude `Ast` sum.
Recognition is structural on the `Type` reflection module (like `Type.of`/`Type.eq`), so it is not
captured by a rebound `Type` name.

## 4. Implementation seams (file anchors)

| Concern | Anchor | Change |
|---|---|---|
| New intrinsics + `Prim` | `prelude.rs:1860-1875` (the `Type` module), `Prim::TypeOf` neighbourhood | Add `type-ast` / `type-ast-generic` intrinsics wired to `Prim::TypeAst { instantiated: bool }` (one prim, bool arg — mirrors how `Type.of`/`Type.eq` sit together). |
| Associated fields | `prelude.rs` `Type` associated-field table (cf. `ast_associated_fields` for `Ast`, `prelude.rs:2290`) | Register `ast` / `ast-generic` on the `Type` module. |
| Reduction | `eval.rs:2872` (`Prim::TypeOf` reduction), `eval.rs:3160-3192` (structural recognition of the reflection module) | Add the `Prim::TypeAst` arm: decode the `(typeval …)` arg to `Ty` (`decode_ty`, already reachable), branch concrete/non-concrete, produce the `Ast` value. |
| Verbatim reflection | `quote.rs:419/462` (`reify`/`reify_inner`) | Reuse to reflect `TypeDecl.occ` → `Ast` (the generic path, and the nominal/sum decl for instantiated). |
| Decl lookup | `db.type_decl_by_occ` (`eval.rs:3919`), `TypeDecl` (`db.rs:580`) | From `Ty::Sum`/`Nominal` `decl` occurrence → `TypeDecl` → `.occ`/`.params`/`.variants`. |
| Surface fallback + instantiation | `type_ast` (`lower.rs:1510-1627`), type-reduction-under-env (`eval.rs:27+`) | Structural-type fallback; substitute params for the instantiated variant. |
| Typing | inference of the `Type.*` fields (follow `Type.of`/`Type.eq` typing) | `Type -> Ast`, empty effect row. |

No change to `cadenza-ast/src/codec.rs` or the frozen encoding contracts — reuses existing `Ast`
variants.

## 5. Increments (top-to-bottom, the way a vertical lands them)

1. **`Type.ast-generic`, nominal/sum only.** Add the `Prim`, prelude wiring, structural recognition,
   and the reduction that reflects `TypeDecl.occ` via `quote::reify`. Typing `Type -> Ast`. Gate: one
   corpus case reflecting a sum type (e.g. `Color`) with the pinned folded `Ast` literal. *This proves
   the whole spine end-to-end on the simplest shape.*
2. **Total coverage for `Type.ast-generic`.** Extend to structural record/tuple/`List`/`Map`/`Set`/
   primitive/`Fn` via the `type_ast` surface fallback; emit the decline diagnostic on a non-concrete
   type. Gate: a corpus case per shape + the decline (an `(error …)` reject case).
3. **`Type.ast` (instantiated).** Add the instantiated variant (param substitution via
   type-reduction, rendered through `type_ast`); for non-generic types it equals `Type.ast-generic`.
   Gate: a generic type shown both ways (`Type.ast` vs `Type.ast-generic`), and a recursive generic
   (`List a` at `List Int`) proving finiteness.
4. **Interop + spec lock.** Confirm `Ast.print` / `Ast.encode` → `Ast.decode` round-trip on a
   reflected result (a corpus case that prints and one that re-decodes byte-identically). Land the
   spec section in `spec/capabilities/metaprogramming.md` (a "Reflecting A Type To Its Definition
   AST" subsection) coordinated with the spec owner; pin the corpus.

Each increment is independently green and a coherent unit (one MR each).

## 6. The gate (what protects it)
- **Corpus** (the authoritative gate): new cases under `spec/semantics/` — either extending
  `12-metaprogramming.sexp` or a new `spec/semantics/type-ast-reflection.sexp`. Each case pins the
  **folded constant `Ast` literal** (like the existing quote case `12-metaprogramming.sexp:13`,
  `(: (Ast.List …) Ast)`), so a miscompile is a corpus diff. Cover: sum, record (structural),
  newtype/nominal, recursive type, generic type (both functions), tuple, primitive, `Fn`, `Type`
  itself, and the non-concrete decline. Include an `Ast.print` round-trip and an `Ast.encode`/`decode`
  round-trip case. Run `--target wasm` for cases that also execute a value (e.g. pattern-matching the
  reflected `Ast`).
- **Unit:** `rcdzc` fold-unit tests for the reduction (a `Ty` → expected `Ast` value) covering the
  substitution and the decline, via `dev-gate` / `cargo test -p rcdzc --lib`.
- **Spec:** the metaprogramming spec section is the human-facing contract; `type-system.md:218-230`
  already permits the `Ast` shape (no new variant, so no encoding version bump).

## 7. Open decisions (with chosen defaults)

1. **Instantiated head form.** Does `Type.ast` on `Pair Int Str` render `(type Pair (Pair Int Str))`
   (param binders dropped) or `(type Pair Int Str (Pair Int Str))` (concrete args shown in the head)?
   **Default:** drop the binders (the params are gone once substituted); the body carries the concrete
   args. The vertical settles the exact head token during increment 3 and pins it in the corpus.
2. **`Prim` shape.** One `Prim::TypeAst { instantiated: bool }` vs two variants. **Default:** one prim
   with a bool, mirroring how `Type.of`/`Type.eq` share the reflection-module neighbourhood; cheaper
   for the RUST-backend arm to cover (one new `Prim`).
3. **Structural record field order.** `type_ast` renders record fields in a canonical order already;
   **default:** inherit whatever `type_ast` produces (do not impose a new ordering) so `Type.ast`
   agrees with existing type-surface rendering.
4. **Aliases.** A transparent alias (`type Meters = Int`) reflects its own decl `(type Meters Int)`
   for `-generic`; `Type.ast` yields the same (no params). **Default:** reflect the alias decl as
   written; do not chase through to the aliased type (chasing would lose the alias's identity, which
   is the point of reflecting the *definition*).

## 8. Hand-off
Subsystem: **`rcdzc`** (with a coordinated spec touch). First increment: **`Type.ast-generic` for
nominal/sum types** (§5.1) — the full spine on the simplest shape. Suggested vertical area
`area=rcdzc` (metaprogramming/reflection lane; coordinate with the `Ast`/quote owners
`v-metaprogramming` / `v-quote-corpus` and the spec owner for §5.4). The RUST-backend arm owner should
be looped in for the new `Prim` (per the standing "new `Prim` needs a rust-backend arm" note).
