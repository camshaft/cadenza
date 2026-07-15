; Tagged-template macros — witnesses metaprogramming.md §A Tagged Template Is A Binding-Dispatched
; Compile-Time Macro Over Literal Chunks And Holes, and DESIGN-tagged-template-macros.md. A tagged
; template `tag"…{expr}…"` (an identifier glued to a string, ML surface) lexes to the CANONICAL node
;   (tagged-template <tag> (chunks <str>…) (holes <expr>…))
; with the invariant chunks.len() == holes.len() + 1. This file writes that canonical node directly (the
; s-expr reader accepts it), so the cases are surface-independent: they pin the EXPANSION contract, not the
; reader. The `tag` is dispatched BY BINDING — resolved to a compile-time function `List String -> List
; Ast -> Ast` — and evaluated on the one-tier compile-time evaluator over the chunks + holes; its returned
; `Ast` is spliced in the template's position and expanded to a fixpoint before type-checking. JSX is then
; a library `jsx : List String -> List Ast -> Ast`, not a language feature.
;
; STAGE STATUS. The reader form (v-syntax B1+B2) is landed; the EXPANDER (rcdzc Inc 2 —
; `tagged_template::expand`) is NOT yet built, so these cases DECLINE (todo) until it lands: a
; `(tagged-template …)` node currently reaches resolve as an ordinary form and reports unbound `tagged-
; template`. They pin the contract the expander must meet — each flips todo→pass when Inc 2 lands.

; --- The core: a tag resolves to a function and its Ast result is spliced --------------------------
; metaprogramming.md: "The compiler MUST evaluate that function … applied to the chunks and holes, and
; MUST splice its resulting abstract syntax tree in the tagged template's position." The `id` echo macro
; returns an `Ast.Str` of its single chunk; expanding `(tagged-template id (chunks "hi") (holes))` yields
; `(Ast.Str "hi")`, matched here for its length (2).

(case "a tagged template expands via its binding-dispatched tag function"
  (doc    "The echo macro: `id` is a compile-time `List String -> List Ast -> Ast` that returns
           `(Ast.Str <first chunk>)`. Expanding `(tagged-template id (chunks \"hi\") (holes))` evaluates
           `id [\"hi\"] []` on the one-tier evaluator and splices the `(Ast.Str \"hi\")` it returns; the
           surrounding match reads its length, 2. Pins binding-dispatch + one-tier eval + splice.")
  (input  (do
            (def (id chunks holes) (match chunks
                                     ((list c) (Ast.Str c))
                                     (_        (Ast.Str ""))))
            (def (main) (match (tagged-template id (chunks "hi") (holes))
                          ((Ast.Str s) (String.byte-len s))
                          (_           0)))
            (export main)))
  (output (: 2 Int64)))

; --- A hole is spliced in at its position ----------------------------------------------------------
; A hole `{expr}` is an ordinary expression carried in the `(holes …)` list. A tag function that weaves a
; hole into its output produces an `Ast` mentioning that hole's value. Here `wrap` returns
; `(Ast.List (list (Ast.Name "f") <first hole>))`, so `(tagged-template wrap (chunks "" "") (holes (Ast.Int
; 7)))` expands to `(Ast.List (list (Ast.Name "f") (Ast.Int 7)))` — an AST value compared for equality.

(case "a tagged template weaves a hole into its expansion"
  (doc    "`wrap` is a `List String -> List Ast -> Ast` that builds `(f <hole0>)` as an `Ast.List`. The
           template supplies one hole `(Ast.Int 7)` (a hole is an ordinary expression — here an `Ast`
           value), so the expansion equals the hand-built `(Ast.List (list (Ast.Name \"f\") (Ast.Int 7)))`.
           Pins that holes reach the tag function and are spliced at the positions its parse reaches.")
  (input  (do
            (def (wrap chunks holes) (match holes
                                       ((list h) (Ast.List (list (Ast.Name "f") h)))
                                       (_        (Ast.List (list)))))
            (def (main) (= (tagged-template wrap (chunks "" "") (holes (Ast.Int 7)))
                           (Ast.List (list (Ast.Name "f") (Ast.Int 7)))))
            (export main)))
  (output (: true Bool)))

; --- The tag must resolve to a suitable function ---------------------------------------------------
; metaprogramming.md: the tag "MUST resolve … to a compile-time function from a list of the chunk strings
; and a list of the hole expressions to an abstract syntax tree." An UNBOUND tag is the ordinary lexical
; scope error (core-semantics.md #Binding Is Lexical) — CDZ0101 — at the template site, because whether a
; tag is a template macro is a binding fact, not a reader fact.

(case "a tagged template whose tag is unbound is a scope error"
  (doc    "`nope` is not bound, so `(tagged-template nope (chunks \"x\") (holes))` cannot resolve the tag
           to a template function — the ordinary unbound-name error (CDZ0101, core-semantics.md #Binding
           Is Lexical), raised at the template site. Pins that tag dispatch is by binding: no binding, no
           expansion.")
  (input  (do
            (def (main) (tagged-template nope (chunks "x") (holes)))
            (export main)))
  (error  CDZ0101))

; --- Expansion runs to a fixpoint ------------------------------------------------------------------
; metaprogramming.md §Expansion Runs In Phases To A Fixpoint (+ the tagged-template §: "expanding to a
; fixpoint before type checking"). A tag function whose returned `Ast` is ITSELF a construction the
; ordinary path folds must be fully reduced before type-checking. Here `two` returns `(Ast.Int 2)` and the
; surrounding program adds 40 — the spliced constant folds through the ordinary compile-time path to 42.

(case "a tagged template's spliced result folds through the ordinary path"
  (doc    "`two` returns `(Ast.Int 2)`; the program evals the template and adds 40. The spliced `Ast`
           value flows into the ordinary fold (compile-time evaluation is one tier), so `(+ 40 (…the 2…))`
           reduces to 42. Pins that a tagged template's expansion is meaning-equivalent to the code its tag
           function produces and is folded/type-checked as ordinary code.")
  (input  (do
            (def (two chunks holes) (Ast.Int 2))
            (def (main) (match (tagged-template two (chunks "") (holes))
                          ((Ast.Int n) (+ 40 n))
                          (_           0)))
            (export main)))
  (output (: 42 Int64)))
