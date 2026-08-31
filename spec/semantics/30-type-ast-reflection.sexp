; Type → AST reflection — `Type.ast` / `Type.ast-generic`. Witnesses DESIGN-type-to-ast-reflection.md.
; Given a `Type` VALUE (a `Type.of` result, or a written type name), reflect the `Ast` of that type's
; DEFINITION — the verbatim `(type Name …)` declaration form, reusing the ordinary `Ast.*` constructors
; (no new `Ast` variant, no encoding change). The missing dual of `encode_ty`, which emits only the type
; REFERENCE, not its shape. A pure compile-time fold on the metaprogramming tier (like `Type.of`/`Type.eq`/
; quote), so the reflected `Ast` is a first-class constant value that pattern-matches / prints / encodes
; like any quote result. `Type.ast-generic` reflects the GENERIC decl verbatim (type params intact);
; `Type.ast` reflects the INSTANTIATED decl (for a NON-generic type the two coincide). Total over concrete
; nominal/sum types; a non-concrete / non-nominal argument declines. See README.md for the case vocabulary.
;
; STAGE STATUS (2026-08-31). INCREMENT 1: `Type.ast-generic` for nominal/sum types (the full spine on the
; simplest shape), plus `Type.ast` on a NON-generic type (coincides with `-generic`). REMAINING: total
; coverage for structural record/tuple/List/Map/Set/primitive/Fn + the non-concrete decline (increment 2);
; the instantiated-substitution variant of `Type.ast` on a GENERIC type (increment 3); Ast.print / Ast.encode
; round-trip lock (increment 4).

(case
  "Type.ast-generic reflects a nominal sum type's verbatim declaration AST"
  (doc
    "The full type→AST spine on the simplest shape: `(Type.ast-generic Color)` for a user
           `(type Color (Red) (Green) (Rgb Int64 Int64 Int64))` folds to the `Ast` VALUE of that verbatim
           declaration — an `Ast.List` headed by `(Ast.Name \"type\")` then the type name, then one child
           per variant (each itself an `Ast.List` of its ctor name + payload type names). Pins that the
           reflected shape is the DEFINITION (its variants + payloads), not merely the type reference the
           existing `encode_ty` emits, and that it reuses the ordinary `Ast.List`/`Ast.Name` constructors
           (no new variant). A compiler that folded away the fields, or mis-tagged a node, diffs here.")
  (input
    (do
      (type Color (Red) (Green) (Rgb Int64 Int64 Int64))
      (def (main) (Type.ast-generic Color))
      (export main)))
  (call main)
  (output
    (:
      (Ast.List
        #list(
          (Ast.Name "type")
          (Ast.Name "Color")
          (Ast.List #list((Ast.Name "Red")))
          (Ast.List #list((Ast.Name "Green")))
          (Ast.List
            #list((Ast.Name "Rgb") (Ast.Name "Int64") (Ast.Name "Int64") (Ast.Name "Int64")))))
      Ast)))

(case
  "Type.ast on a non-generic type coincides with Type.ast-generic (verbatim decl)"
  (doc
    "The short `Type.ast` (the instantiated form) on a type with NO type parameters equals
           `Type.ast-generic` — there is nothing to substitute, so the instantiated and generic decl are
           the same verbatim `(type …)` form. Fed a `Type.of` result rather than a bare name to also pin
           that a value's reflected type reflects its DEFINITION. Pins the increment-1 promise that the
           common `Type.ast` works today for the monomorphic case (the generic-substitution variant is a
           later increment).")
  (input
    (do
      (type Sign (Neg) (Zero) (Pos))
      (def (main) (Type.ast (Type.of (Zero))))
      (export main)))
  (call main)
  (output
    (:
      (Ast.List
        #list(
          (Ast.Name "type")
          (Ast.Name "Sign")
          (Ast.List #list((Ast.Name "Neg")))
          (Ast.List #list((Ast.Name "Zero")))
          (Ast.List #list((Ast.Name "Pos")))))
      Ast)))
