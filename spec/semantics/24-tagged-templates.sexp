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
; STAGE STATUS. The reader form (v-syntax B1+B2) AND the expander (rcdzc Inc 2 —
; `tagged_template::expand`, which rewrites `(tagged-template …)` to the binding-dispatched application
; `(<tag> (list …) (list …))` the one-tier evaluator reduces) are BOTH landed, so these cases PASS
; end-to-end. Deeper increments (JSX library, a 2nd DSL, hygiene for macro-introduced binders) are follow-ons.

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

; --- The tag's TYPE is enforced (dispatch by binding requires the right shape) ---------------------
; metaprogramming.md: the tag "MUST … require it to be a compile-time function from a list of the chunk
; strings and a list of the hole expressions to an abstract syntax tree." A tag bound to a NON-FUNCTION
; is the ordinary "not a function" application error (CDZ0201) on the rewritten call; a tag whose result
; type is NOT `Ast` is caught downstream at the spliced site (the design's "checked at the spliced site" —
; Increment 1 needs no typed quotes). These pin that a mis-typed tag is rejected, not silently expanded.

(case "a tagged template whose tag is not a function is rejected"
  (doc    "`notfn` is bound to an Int64, not a function. `(tagged-template notfn …)` rewrites to the
           application `(notfn (list …) (list …))`, which cannot apply a non-function — CDZ0201. Pins that
           a tag must resolve to a FUNCTION (dispatch by binding requires the right kind).")
  (input  (do
            (def notfn 5)
            (def (main) (tagged-template notfn (chunks "x") (holes)))
            (export main)))
  (error  CDZ0201))

(case "a tagged template whose tag returns a non-Ast is a type error at the spliced site"
  (doc    "`wrongsig` returns an Int64, not an `Ast`. Its expansion is spliced where an `Ast` is expected
           (here matched as a sum), so the mismatch is caught downstream — CDZ0203 (a variant pattern
           cannot match an Int64 scrutinee). Pins the design's 'checked at the spliced site': a tag that
           produces the wrong type is rejected as ordinary ill-typed code, not silently accepted.")
  (input  (do
            (def (wrongsig chunks holes) 5)
            (def (main) (match (tagged-template wrongsig (chunks "x") (holes))
                          ((Ast.Str s) 1)
                          (_           0)))
            (export main)))
  (error  CDZ0203))

(case "a tagged template's hole is threaded to the tag function and read"
  (doc    "A `{expr}` hole is carried in `(holes …)` and reaches the tag function positionally. `first`
           returns hole 0 unchanged; `(tagged-template first (chunks \"\" \"\") (holes (Ast.Int 7)))`
           expands to `(Ast.Int 7)`, read here as 7. Pins that holes flow to the tag function (the
           companion of the earlier weave case, isolating a bare pass-through hole).")
  (input  (do
            (def (first chunks holes) (match holes ((list h) h) (_ (Ast.Int 0))))
            (def (main) (match (tagged-template first (chunks "" "") (holes (Ast.Int 7)))
                          ((Ast.Int n) n)
                          (_           0)))
            (export main)))
  (output (: 7 Int64)))

; --- MULTIPLE holes + an INTERIOR chunk: the expander's multi-element list construction -------------
; Every case above uses zero or one hole (and only edge chunks). The expander builds `(<tag> (list c…)
; (list h…))` — with two holes and three chunks BOTH lists are multi-element, and an INTERIOR chunk
; (`chunks[1]`, between two holes) must land in the middle of the chunk list. This pins that
; `tagged_template::rewrite_of` threads ALL holes positionally and preserves chunk order (the
; `chunks.len() == holes.len() + 1` structure with an interior chunk) — an off-by-one in either list, or
; dropping a non-edge chunk, would flip this case. `weave` reads chunk 1 ("MID") and both holes (10, 20):
; `byte-len("MID") + 10 + 20 = 33`.

(case "a tagged template threads MULTIPLE holes and an interior chunk to its tag function"
  (doc    "Two holes and three chunks — both the `(list c…)` and `(list h…)` the expander builds are
           multi-element, and the middle chunk sits BETWEEN the holes. `weave` reads chunk 1 (\"MID\") and
           both holes (Ast.Int 10, Ast.Int 20) and returns `(Ast.List (Ast.Name \"MID\") 10 20)`, scored
           `byte-len(\"MID\") + 10 + 20 = 33`. Pins the multi-element list construction in
           `tagged_template::rewrite_of`: all holes are threaded positionally and chunk order (including a
           non-edge chunk) is preserved — an off-by-one or a dropped interior chunk flips the answer.")
  (input  (do
            (def (weave chunks holes)
              (match holes
                ((list a b) (match chunks
                              ((list c0 c1 c2) (Ast.List (list (Ast.Name c1) a b)))
                              (_               (Ast.List (list)))))
                (_          (Ast.List (list)))))
            (def (main) (match (tagged-template weave (chunks "p" "MID" "q") (holes (Ast.Int 10) (Ast.Int 20)))
                          ((Ast.List (list (Ast.Name nm) (Ast.Int x) (Ast.Int y)))
                           (+ (String.byte-len nm) (+ x y)))
                          (_ 0)))
            (export main)))
  (output (: 33 Int64)))

(case "a three-hole tag reads its holes in exact left-to-right order"
  (doc    "The 2-hole case above uses [10,20]; three holes with distinct DIGIT values catch a position
           off-by-one it cannot. `pick` reads `(list a b c)` and returns them REORDERED as `(list c a b)`;
           with holes `(Ast.Int 1) (Ast.Int 2) (Ast.Int 3)` the tag sees a=1, b=2, c=3, so the reordered
           result is [3,1,2], read as `3*100 + 1*10 + 2` = 312. Pins that the expander threads THREE holes
           positionally in exact left-to-right order (a scrambled or off-by-one threading would give a
           different digit arrangement), strengthening the two-hole order pin.")
  (input  (do
            (def (pick chunks holes)
              (match holes ((list a b c) (Ast.List (list c a b))) (_ (Ast.List (list)))))
            (def (main)
              (match (tagged-template pick (chunks "" "" "" "") (holes (Ast.Int 1) (Ast.Int 2) (Ast.Int 3)))
                ((Ast.List (list (Ast.Int x) (Ast.Int y) (Ast.Int z))) (+ (* x 100) (+ (* y 10) z)))
                (_ 0)))
            (export main)))
  (output (: 312 Int64)))

; --- Composition: a hole may itself be a quote/Ast expression ---------------------------------------
; A `{expr}` hole is an ORDINARY expression, so it may be a `(quote …)` (or any Ast-valued expression) —
; the two metaprogramming surfaces compose. The hole is parsed as one expression and lowered to `Ast`, so
; a quote in a hole reaches the tag function as the `Ast` value it denotes, exactly like a hand-written
; `Ast.*`. Pins that the tagged-template hole surface layers cleanly over quote/quasiquote.

(case "a quote inside a tagged-template hole reaches the tag function as an Ast value"
  (doc    "A hole's expression may be a `(quote …)`: `first\"a{quote (+ 1 2)}b\"` (canonical
           `(tagged-template first (chunks \"a\" \"b\") (holes (quote (+ 1 2))))`) passes the quote's `Ast`
           value — `(Ast.List (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2))` — as hole 0. The `first` tag
           returns it; `List.len` of its elements is 3. Pins that the hole surface composes with quote (the
           hole is an ordinary expression lowered to `Ast`, so a quote reaches the tag fn like a
           hand-written `Ast.*`).")
  (input  (do
            (def (first chunks holes) (match holes ((list h) h) (_ (Ast.Int 0))))
            (def (main) (match (tagged-template first (chunks "a" "b") (holes (quote (+ 1 2))))
                          ((Ast.List es) (List.len es))
                          (_             0)))
            (export main)))
  (output (: 3 Int64)))

; --- Composition: a tagged template NESTED in another's hole -----------------------------------------
; A hole is an ordinary expression, so it may itself be a `(tagged-template …)`. The expander scans every
; ORIGINAL node once (`tagged_template::expand`), so BOTH the inner and outer templates rewrite; the inner
; sits in the outer's `(holes …)` list, so after the inner rewrites to its tag application, the outer's
; hole IS that application — the inner's expanded `Ast` reaches the outer tag as a hole value. This pins
; that nested expansion composes: the two rewrites do not interfere (the append-bounded scan + node
; overwrite handle a template inside a template), and the outer tag consumes the inner's RESULT, not the
; raw `(tagged-template …)` node. `inner` returns `(Ast.Int 5)`; `outer` wraps its hole as `(+ <hole> 40)`,
; so the composition builds `(+ 5 40)`, scored `byte-len("+") + 5 + 40 = 46`.

(case "a tagged template nested in another's hole composes — the inner's result reaches the outer tag"
  (doc    "A `(tagged-template …)` inside another's hole: `inner` returns `(Ast.Int 5)`, and `outer` wraps
           its single hole as `(Ast.List (Ast.Name \"+\") <hole> (Ast.Int 40))`. The expander rewrites both
           original nodes, so the outer receives the inner's EXPANSION (`(Ast.Int 5)`) as hole 0 and builds
           `(+ 5 40)`; the match scores `byte-len(\"+\") + 5 + 40 = 46`. Pins that nested tagged-template
           expansion composes — the outer tag consumes the inner's result, not the raw template node, and
           the two rewrites in one scan do not interfere.")
  (input  (do
            (def (inner chunks holes) (Ast.Int 5))
            (def (outer chunks holes)
              (match holes
                ((list h) (Ast.List (list (Ast.Name "+") h (Ast.Int 40))))
                (_        (Ast.List (list)))))
            (def (main) (match (tagged-template outer (chunks "" "") (holes (tagged-template inner (chunks "x") (holes))))
                          ((Ast.List (list (Ast.Name op) (Ast.Int a) (Ast.Int b)))
                           (+ (String.byte-len op) (+ a b)))
                          (_ 0)))
            (export main)))
  (output (: 46 Int64)))

; --- The expander is a STRUCTURAL rewrite, not a validator: the chunks==holes+1 invariant is the READER's
; The chunks/holes count invariant (`chunks.len() == holes.len() + 1`) is guaranteed by the READER on the
; surface path (a `tag"…{}…"` literal always yields well-balanced chunks/holes); `tagged_template::expand`
; deliberately DOES NOT re-check it (its docstring says so) — it rewrites any well-SHAPED 4-child
; `(tagged-template <tag> (chunks …) (holes …))` node to `(<tag> (list c…) (list h…))` regardless of the
; count relationship. So a canonical node WRITTEN DIRECTLY (as this file does) with a mismatched count still
; expands: the tag function receives whatever chunk/hole lists it was given. This pins that split — a future
; change that added a count re-check in the expander would break directly-written nodes (and duplicate the
; reader's job), so the structural-not-validating contract is locked here.

(case "the expander rewrites a directly-written node even when chunks != holes+1 (no re-check)"
  (doc    "The reader guarantees `chunks.len() == holes.len() + 1`, but the expander does NOT re-check it — a
           canonical node written directly with 3 chunks and 1 hole (violating the invariant) still expands.
           `id` ignores its args and returns `(Ast.Int 0)`, so `(tagged-template id (chunks \"a\" \"b\" \"c\")
           (holes (Ast.Int 1)))` expands and folds to 0. Pins that the expander is a STRUCTURAL rewrite (any
           well-shaped 4-child node), not a validator — the count invariant is the reader's job, not
           duplicated here (a future re-check would break directly-written nodes).")
  (input  (do
            (def (id chunks holes) (Ast.Int 0))
            (def (main) (match (tagged-template id (chunks "a" "b" "c") (holes (Ast.Int 1)))
                          ((Ast.Int n) n)
                          (_           -1)))
            (export main)))
  (output (: 0 Int64)))

(case "the expander threads the hole list to the tag even with zero chunks and zero holes"
  (doc    "The degenerate shape: zero chunks and zero holes. The expander still rewrites `(tagged-template id
           (chunks) (holes))` to `(id (list) (list))`; `id` returns `(Ast.Int 5)`, folding to 5. Pins that
           empty chunk/hole lists are threaded (both `(list)`), not a decline — the structural rewrite has no
           lower bound on element count.")
  (input  (do
            (def (id chunks holes) (Ast.Int 5))
            (def (main) (match (tagged-template id (chunks) (holes))
                          ((Ast.Int n) n)
                          (_           -1)))
            (export main)))
  (output (: 5 Int64)))
