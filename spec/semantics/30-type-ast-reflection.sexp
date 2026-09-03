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
; simplest shape), plus `Type.ast` on a NON-generic type (coincides with `-generic`). INCREMENT 2: total
; coverage for structural record/tuple/List/Map/Set/primitive via the `type_ast` surface fallback (they have
; no `TypeDecl`, so `-generic` == `-ast` == the canonical type-surface AST); a `Fn` type is TODO-pinned
; (its arrow surface `(-> …)` is a later increment — `type_ast` has no value-form surface for a function).
; INCREMENT 3: `Type.ast` INSTANTIATED on a GENERIC type — substitute the decl's params by the type's
; concrete args (dropping the head binders), rendered finite (a nested self-reference stays a named
; application, never unfolded). INCREMENT 4: interop lock — a reflected definition is an ordinary `Ast`, so
; it round-trips through `Ast.encode`/`Ast.decode` byte-identically and renders via `Ast.print`, with no
; reflection-specific codec path. INCREMENT 5 (operator directive): reflect BUILT-IN type definitions
; (`Option`, `Result`, …) — prelude sums with no user source file reflect their `(type …)` definition from
; the synthesized decl, like a user type. REMAINING: only the `Fn` arrow surface (TODO-pinned above) — the
; core feature is complete.
(diagnostic-quality)

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
        #list((Ast.Name "type")
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
  (input (do (type Sign (Neg) (Zero) (Pos)) (def (main) (Type.ast (Type.of (Zero)))) (export main)))
  (call main)
  (output
    (:
      (Ast.List
        #list((Ast.Name "type")
          (Ast.Name "Sign")
          (Ast.List #list((Ast.Name "Neg")))
          (Ast.List #list((Ast.Name "Zero")))
          (Ast.List #list((Ast.Name "Pos")))))
      Ast)))

(case
  "Type.ast-generic reflects a STRUCTURAL type's canonical type-surface AST (increment 2)"
  (doc
    "A structural type — a tuple / `List` / record / `Map` / `Set` / primitive with NO user `(type …)`
           declaration — has no `TypeDecl` to reify, so `Type.ast-generic` reflects its CANONICAL
           type-surface AST via the same `type_ast` renderer the value-form / `encode_ty` use. There are no
           type params, so `Type.ast` and `Type.ast-generic` COINCIDE. Equivalently: reflecting a value's
           type equals QUOTING that type's surface form — so each shape is checked
           `(= (Type.ast-generic (Type.of <value>)) (quote <type-surface>))`, weighted by position; the
           self-witness sum is 1+2+3+4+5+6 = 21. A shape that reflected to a wrong surface (or a
           name-headed node instead of the ctor) drops its term and shifts the total.")
  (input
    (do
      (def
        (main)
        (+
          (* 1 (if (= (Type.ast-generic (Type.of #tuple(1 true))) (quote (Tuple Int64 Bool))) 1 0))
          (+
            (* 2 (if (= (Type.ast-generic (Type.of #list(1 2 3))) (quote (List Int64))) 1 0))
            (+
              (*
                3
                (if
                  (=
                    (Type.ast-generic (Type.of #record((= a 1) (= b true))))
                    (quote (Record (: a Int64) (: b Bool))))
                  1
                  0))
              (+
                (*
                  4
                  (if
                    (= (Type.ast-generic (Type.of #map((= 1 true)))) (quote (Map Int64 Bool)))
                    1
                    0))
                (+
                  (* 5 (if (= (Type.ast-generic (Type.of #set(1 2 3))) (quote (Set Int64))) 1 0))
                  (* 6 (if (= (Type.ast-generic (Type.of 42)) (quote Int64)) 1 0))))))))
      (export main)))
  (call main)
  (output (: 21 Int64)))

(case
  "Type.ast-generic of a FUNCTION type reflects its arrow surface (-> …)"
  (doc
    "Reflecting a `Fn` type yields its arrow type-surface AST `(-> Param Result)`, just as a structural
           type reflects its surface. `type_ast` has no value-form surface for a function (a function is not
           a boundary value), so the reflection renders the arrow directly (`type_surface_ast`), matching
           `Ty::render_name`'s arrow form. Here `id : Int64 -> Int64` reflects `(-> Int64 Int64)`; a curried
           multi-arg fn nests, `(-> p0 (-> p1 r))`. Closes the arrow-surface gap the earlier increments
           TODO-pinned — reflection is now total over function types too (only a continuation type has no
           surface).")
  (input
    (do
      (def (id (: x Int64)) x)
      (def (main) (if (= (Type.ast-generic (Type.of id)) (quote (-> Int64 Int64))) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "Type.ast INSTANTIATES a generic type's params; Type.ast-generic keeps them verbatim (they differ)"
  (doc
    "Increment 3 — the instantiated variant of `Type.ast` on a GENERIC type. For
           `(type Opt a (Sm a) (Nn))` reflected from a value of type `Opt Int64`: `Type.ast` substitutes
           the concrete arg (`a` -> `Int64`) into the decl and DROPS the head param binders, folding to
           `(type Opt (Sm Int64) (Nn))`; `Type.ast-generic` keeps the params intact, `(type Opt a (Sm a)
           (Nn))`. So the two DIFFER for a generic type (they coincided only for the monomorphic case
           pinned above). Checks: instantiated == the substituted-decl quote (weight 1), generic == the
           verbatim quote (weight 2), and the two are NOT equal (weight 4) — self-witness 1+2+4 = 7. The
           substitution reuses the type params' first-appearance order (`TypeDecl.params`), so a wrong
           arg->param mapping or a failure to drop the binders shifts the total.")
  (input
    (do
      (type Opt a (Sm a) (Nn))
      (def
        (main)
        (+
          (* 1 (if (= (Type.ast (Type.of (Sm 1))) (quote (type Opt (Sm Int64) (Nn)))) 1 0))
          (+
            (* 2 (if (= (Type.ast-generic (Type.of (Sm 1))) (quote (type Opt a (Sm a) (Nn)))) 1 0))
            (* 4 (if (= (Type.ast (Type.of (Sm 1))) (Type.ast-generic (Type.of (Sm 1)))) 0 1)))))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "Type.ast on a RECURSIVE generic stays finite — a nested self-reference is not unfolded"
  (doc
    "Finiteness (design §3.3): instantiating a RECURSIVE generic substitutes only the decl's OWN param
           binders in its OWN body; every nested named type reference — including the self-reference — stays
           a `(Name arg…)` application, NEVER expanded. So `(type Lst a (Nil) (Cons a (Lst a)))` reflected at
           `Lst Int64` folds to `(type Lst (Nil) (Cons Int64 (Lst Int64)))` — the `a` in `Cons`'s payload
           becomes `Int64`, and the self-reference `(Lst a)` becomes `(Lst Int64)` but is not inlined, so
           the result is finite even though the type is infinite. A substitution that unfolded the
           self-reference would not terminate (or would diverge from this pinned shape).")
  (input
    (do
      (type Lst a (Nil) (Cons a (Lst a)))
      (def
        (main)
        (if
          (=
            (Type.ast (Type.of ((Cons 1) (Nil))))
            (quote (type Lst (Nil) (Cons Int64 (Lst Int64)))))
          1
          0))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a reflected type-definition AST round-trips byte-identically through Ast.encode / Ast.decode"
  (doc
    "Increment 4 — interop lock. A reflected type definition is an ORDINARY `Ast` value, so it crosses
           the binary-AST codec like any quote result: `Ast.encode` serializes the reflected
           `(Type.ast-generic Color)` to its canonical `cdzast` bytes and `Ast.decode` reads them back to an
           `Ast` equal to the original (the decode is total → `(Ok a)`). Pins that type-reflection composes
           with the rest of the metaprogramming machinery with no new codec path — the whole point of
           reflecting to the ordinary `Ast` sum rather than a bespoke descriptor.")
  (input
    (do
      (type Color (Red) (Green) (Rgb Int64 Int64 Int64))
      (def
        (main)
        (match
          (Ast.decode (Ast.encode (Type.ast-generic Color)))
          ((Ok a) (if (= a (Type.ast-generic Color)) 1 0))
          ((Err _u) 0)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "Ast.print of a reflected type definition renders its canonical (type …) source form"
  (doc
    "Increment 4 — interop lock. `Ast.print` on the reflected definition renders the canonical
           s-expression source of the `(type …)` declaration — the reflected `Ast` prints exactly as the
           written type would, closing the loop from a `Type` value back to readable source text. Pins that
           the reflected shape is the verbatim declaration form (head `type`, the name, one child per
           variant), and that it flows through the ordinary `Ast.print` with no reflection-specific path.")
  (input
    (do
      (type Color (Red) (Green) (Rgb Int64 Int64 Int64))
      (def (main) (Ast.print (Type.ast-generic Color)))
      (export main)))
  (call main)
  (output (: "(type Color (Red) (Green) (Rgb Int64 Int64 Int64))" String)))

(case
  "Type.ast of a NON-CONCRETE type is a coded rejection — a type variable has no definition to reflect"
  (doc
    "Increment 4 — the non-concrete rejection. Reflecting a type that still carries an unresolved type
           variable (here `(Type.of (Nn))` for the generic `(type Opt a (Sm a) (Nn))` — the nullary variant
           pins nothing, so its type is `Opt <var>`) is a GENUINE SEMANTIC error: a type variable has no
           definition to reflect. So `Type.ast` REJECTS it with a specific machine-readable diagnostic
           (CDZ0203), NOT a codeless decline — a decline is reserved for a well-formed construct the compiler
           does-not-yet-compile, whereas an unresolved type variable is permanently ill-formed here (operator
           corpus policy + v-spec-oracle review). The fix is to annotate the value's type so the reflection
           has a concrete definition to render.")
  (input (do (type Opt a (Sm a) (Nn)) (def (main) (Type.ast (Type.of (Nn)))) (export main)))
  (error CDZ0203 (message "requires a concrete type")))

(case
  "Type.ast reflects a BUILT-IN type's definition (Option, Result) like a user type"
  (doc
    "Increment 5 (operator directive) — reflection MUST handle BUILT-IN type definitions, not just
           user-declared ones. `Option` and `Result` are prelude sums with no user source file, but they
           have the same `(type …)` definition shape, so `Type.ast-generic`/`Type.ast` reflect them
           identically to a user type: generic `Option` → `(type Option (Some a) (None))`, its instantiation
           at `Option Int64` → `(type Option (Some Int64) (None))`, and `Result` → `(type Result (Ok a) (Err
           e))`. Checked `(= reflected (quote <def>))` weighted 1/2/4 — self-witness 7. (Structural
           built-ins — `List`/`Map`/`Set`/`Tuple`/primitives — already reflect via the type-surface
           fallback, pinned in the structural case above.) A built-in that failed to reflect its definition
           would drop its term.")
  (input
    (do
      (def
        (main)
        (+
          (*
            1
            (if (= (Type.ast-generic (Type.of (Some 1))) (quote (type Option (Some a) (None)))) 1 0))
          (+
            (* 2 (if (= (Type.ast (Type.of (Some 1))) (quote (type Option (Some Int64) (None)))) 1 0))
            (*
              4
              (if
                (=
                  (Type.ast-generic (Type.of (: (Ok 1) (Result Int64 String))))
                  (quote (type Result (Ok a) (Err e))))
                1
                0)))))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "Type.ast reflects the type-of-types Type itself as the bare Ast.Name Type"
  (doc
    "Design §3.4: the type-of-types `Type` is concrete and reflects to the bare `(Ast.Name \"Type\")` — it
           has no `(type …)` declaration and no structural surface, it IS the primitive type name. Here
           `(Type.of Int64)` is `Type` (the type of the type-VALUE `Int64`), so `(Type.ast (Type.of Int64))`
           folds to `(quote Type)` == `(Ast.Name \"Type\")`. Pins the `Ty::Type` reflection arm the earlier
           increments left implicit — reflection is total over concrete types INCLUDING the type of types. A
           compiler that failed to name `Type`, or fabricated a declaration for it, diffs here.")
  (input (do (def (main) (if (= (Type.ast (Type.of Int64)) (quote Type)) 1 0)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "Type.ast on a transparent alias reflects its own declaration verbatim and both forms coincide"
  (doc
    "Design §7.4: a transparent alias / newtype `(type Meters Int64)` reflects its OWN declaration form
           `(type Meters Int64)` — reflection does NOT chase through to the aliased `Int64` (that would lose
           the alias's identity, the whole point of reflecting the DEFINITION). It has no type params, so
           `Type.ast` and `Type.ast-generic` COINCIDE (like the non-generic sum case). Checks the verbatim
           reflection (weight 1) and the coincidence (weight 2) — self-witness 3. A compiler that chased the
           alias to `Int64`, or diverged the two forms, drops a term.")
  (input
    (do
      (type Meters Int64)
      (def
        (main)
        (+
          (* 1 (if (= (Type.ast-generic Meters) (quote (type Meters Int64))) 1 0))
          (* 2 (if (= (Type.ast Meters) (Type.ast-generic Meters)) 1 0))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "Type.ast on a MUTUALLY-recursive generic stays finite — a cross-type reference is not unfolded"
  (doc
    "Design §3.3 extends finiteness to MUTUALLY-recursive groups, not just self-recursion (pinned above
           for `Lst`). Instantiating `(type Tree a (Leaf a) (Node (Forest a)))` at `Tree Int64` substitutes
           only `Tree`'s OWN param `a` in `Tree`'s OWN body; the cross-reference `(Forest a)` becomes
           `(Forest Int64)` but is NEVER inlined to `Forest`'s definition — so the result
           `(type Tree (Leaf Int64) (Node (Forest Int64)))` is finite even though `Tree`/`Forest` are mutually
           infinite. A substitution that unfolded the cross-reference would not terminate (or diverge from
           this pinned shape).")
  (input
    (do
      (type Tree a (Leaf a) (Node (Forest a)))
      (type Forest a (Empty) (More (Tree a) (Forest a)))
      (def
        (main)
        (if
          (=
            (Type.ast (Type.of (Node (More (Leaf 1) (Empty)))))
            (quote (type Tree (Leaf Int64) (Node (Forest Int64)))))
          1
          0))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "Type.ast instantiates a generic with a COMPOUND type argument, keeping the nested application named"
  (doc
    "The instantiated form substitutes a param by a COMPOUND (non-scalar) type argument, not just a
           primitive (increment 3 pinned `a`->`Int64`). For `(type Opt a (Sm a) (Nn))` reflected from a value
           of type `Opt (Opt Int64)` (`(Sm (Sm 1))`), `Type.ast` substitutes `a`->`Opt Int64`, folding to
           `(type Opt (Sm (Opt Int64)) (Nn))` — the argument `(Opt Int64)` is a named application left intact
           (not unfolded to `Opt`'s own definition), consistent with the finiteness rule. Pins that argument
           surfaces of arbitrary shape flow through the substitution, not only scalar type names.")
  (input
    (do
      (type Opt a (Sm a) (Nn))
      (def
        (main)
        (if (= (Type.ast (Type.of (Sm (Sm 1)))) (quote (type Opt (Sm (Opt Int64)) (Nn)))) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "Type.ast reflects a DIRECT user-generic instantiation (Box Int64), coherent with the value-mediated form"
  (doc
    "The DIRECT spelling of an instantiated user generic in a type context. `Type.ast` reflects a TYPE,
           so its argument is a TYPE EXPRESSION: `(Box Int64)` for a user `(type Box a (Mk a))` is the
           generic `Box` instantiated at `Int64` — a type, exactly as the builtin `(List Int64)` is `List`
           instantiated at `Int64`. So `(Type.ast (Box Int64))` MUST reflect the instantiated declaration
           `(type Box (Mk Int64))` — the SAME Ast the value-mediated form `(Type.ast (Type.of (Mk 1)))`
           already produces (increment 3), and the direct analogue of the builtin-direct `(Type.ast
           (List Int64))`. The three forms — builtin-direct, value-mediated, user-direct — cohere. Checks:
           the direct form equals the substituted-decl quote (weight 1) AND equals the value-mediated form
           (weight 2) — self-witness 1+2 = 3. Previously `(Box Int64)` in argument position was mis-grounded
           as a VALUE application (its type-constructor head has no arrow scheme, unlike a builtin), which
           rejected CDZ0203 with a self-contradictory 'a value appears here' — that gap is now closed:
           user and builtin generics are uniform in a type context.")
  (input
    (do
      (type Box a (Mk a))
      (def
        (main)
        (+
          (* 1 (if (= (Type.ast (Box Int64)) (quote (type Box (Mk Int64)))) 1 0))
          (* 2 (if (= (Type.ast (Box Int64)) (Type.ast (Type.of (Mk 1)))) 1 0))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "Type.ast of a WRONG-ARITY direct user-generic application is rejected, not silently reflected"
  (doc
    "The ill-formed control bounding the direct-instantiation support: `(Box Int64 Int64)` over-applies the
           one-parameter generic `(type Box a (Mk a))`. Accepting the DIRECT form `(Type.ast (Box Int64))`
           must NOT accept a WRONG-ARITY spelling — a two-argument application of a one-parameter constructor
           has no coherent instantiation, so `Type.ast` REJECTS it (CDZ0203) rather than tolerantly dropping
           the surplus argument and silently reflecting `(type Box (Mk Int64))`. Pins that the type-context
           exemption grounds the argument as a TYPE without disabling arity validity — the same arity
           discipline a builtin `(List Int64 Int64)` obeys.")
  (input (do (type Box a (Mk a)) (def (main) (Type.ast (Box Int64 Int64))) (export main)))
  (error CDZ0203 (message "type constructor")))

(case
  "Type.ast does not suppress a fault inside a (Type.of e) argument"
  (doc
    "The type-context exemption is NARROW: it skips the value-check ONLY for a direct WRITTEN type-ctor
           application `(Box Int64)`, never for a `(Type.of e)` reflection — whose argument `e` is a VALUE
           expression whose OWN faults must still surface through `Type.ast`. Here `e` = `(g 1 2 3)`
           over-applies the arity-1 `g`, a genuine CDZ0203 that must NOT be swallowed by reflecting the
           result type. Pins that reflecting a value's type does not create a fault-suppression hole (a
           broad `typeval_of`-grounds-so-skip exemption did — regression guard).")
  (input (do (def (g x) x) (def (main) (Type.ast (Type.of (g 1 2 3)))) (export main)))
  (error CDZ0203 (message "function of arity 1")))

(case
  "Type.ast of a non-type value (an effect name) is a CODED rejection, not a codeless decline"
  (doc
    "`Type.ast` reflects a TYPE, so an argument that does not denote a type-value — an EFFECT name, a
           plain value, any non-type expression — is a genuine SEMANTIC reject, not a not-yet-compiled
           decline (a well-formed type reduces to a type-value; a not-yet-supported arrow/cont SURFACE is a
           separate decline over a type that DOES reduce). Per seq-286 (every reject coded), the guard emits
           a CODED CDZ0203 — `(Type.ast E)` for an effect `E` rejects with a machine-readable code, not the
           earlier codeless `error:` text — matching the sibling unresolved-type-variable reject and making
           it fenceable.")
  (input (do (effect E (op bail (-> Int64))) (def (main) (Type.ast E)) (export main)))
  (error CDZ0203 (message "requires a concrete type-value")))

; tdx1: the DERIVE idiom — the reflected declaration CONSUMED, not merely compared. The cases above
; pin equality against hand-built quotes and the codec round-trip; this walks the reflected decl
; with ordinary collection ops + nested Ast matches to derive facts a metaprogram would need:
; variant COUNT (list length minus the head pair) and a variant's payload ARITY (List.at into the
; decl, match the ctor list, length minus the name). (type Color (Red) (Green) (Rgb Int64 Int64
; Int64)) -> 3 variants, Rgb arity 3 -> 33 + n, all folding at compile time (the reflected Ast is a
; first-class constant). Pins that reflection output feeds the ordinary Ast/List toolchain with no
; reflection-specific accessors. (breaker probe td1, verified tri-target exact + byte-idempotent.)
(case
  "a reflected declaration is walked with collection ops to derive variant count and arity"
  (input
    (do
      (type Color (Red) (Green) (Rgb Int64 Int64 Int64))
      (def (count-variants (: a Ast)) (match a ((Ast.List items) (- (List.len items) 2)) (_ -1)))
      (def
        (payload-arity (: a Ast))
        (match
          a
          ((Ast.List items)
            (match
              (List.at items 4)
              ((Some v) (match v ((Ast.List parts) (- (List.len parts) 1)) (_ -2)))
              ((None) -3)))
          (_ -1)))
      (def
        (main (: n Int64))
        (+
          (count-variants (Type.ast-generic Color))
          (+ (* 10 (payload-arity (Type.ast-generic Color))) n)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 33 Int64))
  (call main (: 5 Int64))
  (output (: 38 Int64)))

; --- Instantiated-reflection head is spelling-independent (bare type name) --------------------------
; The reflected head of an INSTANTIATED type is the BARE type name regardless of the source decl's binder
; SPELLING — a parenthesized `(type (Box a) …)` reflects the same as a flat `(type Box a …)` would, since
; one type MEANING must reflect one Ast (v-spec-oracle ruling; consistent with the flat-decl #7688 case
; above, whose `(Type.ast (Box Int64))` → `(type Box (Mk Int64))`). The instantiation lives in the variant
; payloads (`(Mk Int64)`), not an applied head (`(Box Int64)`).
(case
  "an instantiated type reflects a BARE type-name head from a PARENTHESIZED (type (Box a) …) decl (spelling-independent)"
  (doc
    "A parenthesized generic decl `(type (Box a) (Mk a))` reflected instantiated at `Box Int64` MUST yield
           a BARE head `(type Box (Mk Int64))` — the same Ast the flat-spelled `(type Box a (Mk a))` yields
           (pinned via `(Type.ast (Box Int64))` above) — NOT an applied head `(type (Box Int64) …)`. Pins
           spelling-independence: the reflected Ast depends on the type's meaning, not the decl's binder form.
           The instantiation is carried in the payload `(Mk Int64)`; the head names the type's identity.")
  (input
    (do
      (type (Box a) (Mk a))
      (def (main) (if (= (Type.ast (Type.of (Mk 5))) (quote (type Box (Mk Int64)))) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64)))
