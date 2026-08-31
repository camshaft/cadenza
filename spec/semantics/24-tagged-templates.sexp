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
(case
  "a tagged template expands via its binding-dispatched tag function"
  (doc
    "The echo macro: `id` is a compile-time `List String -> List Ast -> Ast` that returns
           `(Ast.Str <first chunk>)`. Expanding `(tagged-template id (chunks \"hi\") (holes))` evaluates
           `id [\"hi\"] []` on the one-tier evaluator and splices the `(Ast.Str \"hi\")` it returns; the
           surrounding match reads its length, 2. Pins binding-dispatch + one-tier eval + splice.")
  (input
    (do
      (def (id chunks holes) (match chunks (#list(c) (Ast.Str c)) (_ (Ast.Str ""))))
      (def
        (main)
        (match
          (tagged-template id (chunks "hi") (holes))
          ((Ast.Str s) ((. String byte-len) s))
          (_ 0)))
      (export main)))
  (output (: 2 Int64)))

; --- A hole is spliced in at its position ----------------------------------------------------------
; A hole `{expr}` is an ordinary expression carried in the `(holes …)` list. A tag function that weaves a
; hole into its output produces an `Ast` mentioning that hole's value. Here `wrap` returns
; `(Ast.List (list (Ast.Name "f") <first hole>))`, so `(tagged-template wrap (chunks "" "") (holes (Ast.Int
; 7)))` expands to `(Ast.List (list (Ast.Name "f") (Ast.Int 7)))` — an AST value compared for equality.
(case
  "a tagged template weaves a hole into its expansion"
  (doc
    "`wrap` is a `List String -> List Ast -> Ast` that builds `(f <hole0>)` as an `Ast.List`. The
           template supplies one hole `(Ast.Int 7)` (a hole is an ordinary expression — here an `Ast`
           value), so the expansion equals the hand-built `(Ast.List (list (Ast.Name \"f\") (Ast.Int 7)))`.
           Pins that holes reach the tag function and are spliced at the positions its parse reaches.")
  (input
    (do
      (def
        (wrap chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "f") h))) (_ (Ast.List #list()))))
      (def
        (main)
        (=
          (tagged-template wrap (chunks "" "") (holes (Ast.Int 7)))
          (Ast.List #list((Ast.Name "f") (Ast.Int 7)))))
      (export main)))
  (output (: true Bool)))

; --- The tag must resolve to a suitable function ---------------------------------------------------
; metaprogramming.md: the tag "MUST resolve … to a compile-time function from a list of the chunk strings
; and a list of the hole expressions to an abstract syntax tree." An UNBOUND tag is the ordinary lexical
; scope error (core-semantics.md #Binding Is Lexical) — CDZ0101 — at the template site, because whether a
; tag is a template macro is a binding fact, not a reader fact.
(case
  "a tagged template whose tag is unbound is a scope error"
  (doc
    "`nope` is not bound, so `(tagged-template nope (chunks \"x\") (holes))` cannot resolve the tag
           to a template function — the ordinary unbound-name error (CDZ0101, core-semantics.md #Binding
           Is Lexical), raised at the template site. Pins that tag dispatch is by binding: no binding, no
           expansion.")
  (input (do (def (main) (tagged-template nope (chunks "x") (holes))) (export main)))
  (error CDZ0101))

; --- Expansion runs to a fixpoint ------------------------------------------------------------------
; metaprogramming.md §Expansion Runs In Phases To A Fixpoint (+ the tagged-template §: "expanding to a
; fixpoint before type checking"). A tag function whose returned `Ast` is ITSELF a construction the
; ordinary path folds must be fully reduced before type-checking. Here `two` returns `(Ast.Int 2)` and the
; surrounding program adds 40 — the spliced constant folds through the ordinary compile-time path to 42.
(case
  "a tagged template's spliced result folds through the ordinary path"
  (doc
    "`two` returns `(Ast.Int 2)`; the program evals the template and adds 40. The spliced `Ast`
           value flows into the ordinary fold (compile-time evaluation is one tier), so `(+ 40 (…the 2…))`
           reduces to 42. Pins that a tagged template's expansion is meaning-equivalent to the code its tag
           function produces and is folded/type-checked as ordinary code.")
  (input
    (do
      (def (two chunks holes) (Ast.Int 2))
      (def (main) (match (tagged-template two (chunks "") (holes)) ((Ast.Int n) (+ 40N n)) (_ 0N)))
      (export main)))
  (output (: 42 BigInt))
  (live-objects known-leak))

; A DISTINCT fixpoint dimension: a tag function whose BODY is ITSELF a tagged template (not just a tag
; returning a plain Ast). `outer` expands to `(tagged-template inner …)`, which must ITSELF be expanded —
; the expander re-runs on the result until no template survives ("expanding to a fixpoint before type
; checking"). This exercises the RECURSION of the expansion pass through a tag body, which the case above
; (a tag returning a bare `Ast.Int`) does not reach. If the expander ran only once, the inner template
; would survive into resolve as an unbound `tagged-template` form (4× CDZ0101), not fold to the Int.
(case
  "expansion recurses to a fixpoint through a tag whose body is another tagged template"
  (doc
    "`outer`'s body IS `(tagged-template inner …)`; `inner` returns `(Ast.Int 7)`. Expanding the
           outer template yields the inner template, which the expander must ITSELF expand (fixpoint) so the
           final spliced value is `(Ast.Int 7)`, read here as 7. Pins that expansion re-runs on its own
           output through a tag body — not a single pass — so a nested-macro-producing tag fully reduces
           before type-checking (else the inner `(tagged-template …)` survives as an unbound form).")
  (input
    (do
      (def (inner chunks holes) (Ast.Int 7))
      (def (outer chunks holes) (tagged-template inner (chunks "") (holes)))
      (def (main) (match (tagged-template outer (chunks "x") (holes)) ((Ast.Int n) n) (_ 0N)))
      (export main)))
  (output (: 7 BigInt)))

; --- A RECURSIVE tag function const-folds (the compile-time evaluator reduces recursion) -----------
; metaprogramming.md: the tag is "evaluated on the one-tier compile-time evaluator." A tag that calls a
; RECURSIVE helper (the shape every real DSL parser has — a scan/count loop) must fold to a compile-time
; constant `Ast`, not be emitted as runtime code. This exercises the compile-time evaluator's recursion
; reduction (the eval-core depth-guarded fold): `tri` calls `sum-to`, a self-recursive `1..n` sum, and
; splices `(Ast.Int (sum-to 4))` = `(Ast.Int 10)`. A terminating recursive tag folds; the depth backstop
; stops a runaway one. This is the precondition a recursive-descent tag parser (e.g. JSX) needs.
(case
  "a recursive tag function const-folds to a compile-time Ast"
  (doc
    "`tri` returns `(Ast.Int (sum-to 4))` where `sum-to` is a self-RECURSIVE `1..n` sum — the tag's
           body is not straight-line. The compile-time evaluator reduces the recursion (sum-to 4 = 10) and
           splices `(Ast.Int 10)`, read here as 10. Pins that a tag calling a recursive helper folds to a
           constant `Ast` at expansion — the capability a real recursive-descent DSL parser tag depends on
           — rather than declining or emitting runtime code. A terminating recursion folds; the evaluator's
           depth backstop stops a non-terminating one (this case terminates).")
  (input
    (do
      (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (- n 1)))))
      (def (tri chunks holes) (Ast.Int (BigInt.of (sum-to 4))))
      (def (main) (match (tagged-template tri (chunks "x") (holes)) ((Ast.Int n) n) (_ 0N)))
      (export main)))
  (output (: 10 BigInt))
  (live-objects known-leak))

; The JSX precursor: a recursive tag that BUILDS A COMPOUND `Ast` (not just a scalar) — the shape a real
; recursive-descent parser tag has (recursively assembling child nodes into an `Ast.List`). `build-list`
; recursively pushes `(Ast.Int k)` nodes; `mk` wraps them in an `Ast.List`. The compile-time evaluator must
; reduce the recursion AND fold the compound construction to a constant `(Ast.List (Ast.Int 1) (Ast.Int 2)
; (Ast.Int 3))`, spliced + read here as length 3. This is a strictly stronger capability than the scalar
; recursive-tag case above (recursion + compound-Ast build, not recursion + a bare Int).
(case
  "a recursive tag builds a compound Ast.List at compile-time fold"
  (doc
    "`mk` returns `(Ast.List (build-list 3))` where `build-list` RECURSIVELY pushes `(Ast.Int k)`
           nodes — so the tag both recurses AND assembles a compound `Ast`. The compile-time evaluator
           reduces the recursion and folds the whole construction to the constant `Ast.List` of three
           `Ast.Int` children, read here as length 3. Pins the direct JSX precursor: a recursive-descent
           parser tag assembling child AST nodes into a list folds to a compile-time constant, not runtime
           code — a strictly stronger capability than a recursive tag returning a bare scalar.")
  (input
    (do
      (def
        (build-list (: n Int64))
        (if (= n 0) #list() (List.push (build-list (- n 1)) (Ast.Int (BigInt.of n)))))
      (def (mk chunks holes) (Ast.List (build-list 3)))
      (def
        (main)
        (match (tagged-template mk (chunks "x") (holes)) ((Ast.List xs) (List.len xs)) (_ 0)))
      (export main)))
  (output (: 3 Int64)))

; The full JSX-lexer precursor: a tag that RECURSIVELY SCANS its CHUNK TEXT byte-by-byte — the exact shape
; a recursive-descent parser tag consumes its input (mirroring implementation/compiler-ml/src/lex.cdz, which
; scans char-codes). `count-b` recurses over `String.to-bytes` of the chunk via `Bytes.at`/`Bytes.len`,
; counting a target byte; `scan` wraps the count in `(Ast.Int …)`. The compile-time evaluator must reduce a
; recursion DRIVEN BY THE CHUNK CONTENT (not a bare counter) and fold to a constant `Ast`. This is the last
; capability a real DSL parser tag needs before the full parser: recurse + read the chunk + build an Ast.
(case
  "a recursive tag scans its chunk text byte-by-byte and folds to a constant Ast"
  (doc
    "`scan`'s tag calls `count-b`, which RECURSES over the chunk's bytes (`Bytes.at`/`Bytes.len` on
           `String.to-bytes` of the chunk) counting the byte `98` ('b'). For chunk \"abbcbb\" that is 4, so
           the tag folds to `(Ast.Int 4)`, read here as 4. Pins the JSX-lexer shape: a tag whose recursion
           is DRIVEN BY THE CHUNK CONTENT (a scan/lex loop, not a bare counter) const-folds to an `Ast` —
           the input-consumption capability a recursive-descent parser tag needs, mirroring lex.cdz's
           char-code scan. The module carries a real byte-scan (1132 bytes) folded to the constant.")
  (input
    (do
      (def
        (count-b (: bytes Bytes) (: i Int64) (: acc Int64))
        (match
          (Bytes.at bytes i)
          ((Option.Some c) (count-b bytes (+ i 1) (if (= c 98) (+ acc 1) acc)))
          ((Option.None _) acc)))
      (def (first-chunk chunks) (match chunks (#list(c) c) (#list(c (.. r)) c) (_ "")))
      (def
        (scan chunks holes)
        (Ast.Int (BigInt.of (count-b ((. String to-bytes) (first-chunk chunks)) 0 0))))
      (def (main) (match (tagged-template scan (chunks "abbcbb") (holes)) ((Ast.Int n) n) (_ 0N)))
      (export main)))
  (output (: 4 BigInt))
  (live-objects known-leak))

; --- The tag's TYPE is enforced (dispatch by binding requires the right shape) ---------------------
; metaprogramming.md: the tag "MUST … require it to be a compile-time function from a list of the chunk
; strings and a list of the hole expressions to an abstract syntax tree." A tag bound to a NON-FUNCTION
; is the ordinary "not a function" application error (CDZ0201) on the rewritten call; a tag whose result
; type is NOT `Ast` is caught downstream at the spliced site (the design's "checked at the spliced site" —
; Increment 1 needs no typed quotes). These pin that a mis-typed tag is rejected, not silently expanded.
(case
  "a tagged template whose tag is not a function is rejected"
  (doc
    "`notfn` is bound to an Int64, not a function. `(tagged-template notfn …)` rewrites to the
           application `(notfn (list …) (list …))`, which cannot apply a non-function — CDZ0201. Pins that
           a tag must resolve to a FUNCTION (dispatch by binding requires the right kind).")
  (input (do (def notfn 5) (def (main) (tagged-template notfn (chunks "x") (holes))) (export main)))
  (error CDZ0201))

(case
  "a tagged template whose tag returns a non-Ast is a type error at the spliced site"
  (doc
    "`wrongsig` returns an Int64, not an `Ast`. Its expansion is spliced where an `Ast` is expected
           (here matched as a sum), so the mismatch is caught downstream — CDZ0203 (a variant pattern
           cannot match an Int64 scrutinee). Pins the design's 'checked at the spliced site': a tag that
           produces the wrong type is rejected as ordinary ill-typed code, not silently accepted.")
  (input
    (do
      (def (wrongsig chunks holes) 5)
      (def (main) (match (tagged-template wrongsig (chunks "x") (holes)) ((Ast.Str s) 1) (_ 0)))
      (export main)))
  (error CDZ0203))

(case
  "a tagged template's hole is threaded to the tag function and read"
  (doc
    "A `{expr}` hole is carried in `(holes …)` and reaches the tag function positionally. `first`
           returns hole 0 unchanged; `(tagged-template first (chunks \"\" \"\") (holes (Ast.Int 7)))`
           expands to `(Ast.Int 7)`, read here as 7. Pins that holes flow to the tag function (the
           companion of the earlier weave case, isolating a bare pass-through hole).")
  (input
    (do
      (def (first chunks holes) (match holes (#list(h) h) (_ (Ast.Int 0))))
      (def
        (main)
        (match (tagged-template first (chunks "" "") (holes (Ast.Int 7))) ((Ast.Int n) n) (_ 0N)))
      (export main)))
  (output (: 7 BigInt)))

; --- MULTIPLE holes + an INTERIOR chunk: the expander's multi-element list construction -------------
; Every case above uses zero or one hole (and only edge chunks). The expander builds `(<tag> (list c…)
; (list h…))` — with two holes and three chunks BOTH lists are multi-element, and an INTERIOR chunk
; (`chunks[1]`, between two holes) must land in the middle of the chunk list. This pins that
; `tagged_template::rewrite_of` threads ALL holes positionally and preserves chunk order (the
; `chunks.len() == holes.len() + 1` structure with an interior chunk) — an off-by-one in either list, or
; dropping a non-edge chunk, would flip this case. `weave` reads chunk 1 ("MID") and both holes (10, 20):
; `byte-len("MID") + 10 + 20 = 33`.
(case
  "a tagged template threads MULTIPLE holes and an interior chunk to its tag function"
  (doc
    "Two holes and three chunks — both the `(list c…)` and `(list h…)` the expander builds are
           multi-element, and the middle chunk sits BETWEEN the holes. `weave` reads chunk 1 (\"MID\") and
           both holes (Ast.Int 10, Ast.Int 20) and returns `(Ast.List (Ast.Name \"MID\") 10 20)`, scored
           `byte-len(\"MID\") + 10 + 20 = 33`. Pins the multi-element list construction in
           `tagged_template::rewrite_of`: all holes are threaded positionally and chunk order (including a
           non-edge chunk) is preserved — an off-by-one or a dropped interior chunk flips the answer.")
  (input
    (do
      (def
        (weave chunks holes)
        (match
          holes
          (#list(a b)
            (match
              chunks
              (#list(c0 c1 c2) (Ast.List #list((Ast.Name c1) a b)))
              (_ (Ast.List #list()))))
          (_ (Ast.List #list()))))
      (def
        (main)
        (match
          (tagged-template weave (chunks "p" "MID" "q") (holes (Ast.Int 10) (Ast.Int 20)))
          ((Ast.List #list((Ast.Name nm) (Ast.Int x) (Ast.Int y)))
            (+ (BigInt.of ((. String byte-len) nm)) (+ x y)))
          (_ 0N)))
      (export main)))
  (output (: 33 BigInt))
  (live-objects known-leak))

(case
  "a three-hole tag reads its holes in exact left-to-right order"
  (doc
    "The 2-hole case above uses [10,20]; three holes with distinct DIGIT values catch a position
           off-by-one it cannot. `pick` reads `(list a b c)` and returns them REORDERED as `(list c a b)`;
           with holes `(Ast.Int 1) (Ast.Int 2) (Ast.Int 3)` the tag sees a=1, b=2, c=3, so the reordered
           result is [3,1,2], read as `3*100 + 1*10 + 2` = 312. Pins that the expander threads THREE holes
           positionally in exact left-to-right order (a scrambled or off-by-one threading would give a
           different digit arrangement), strengthening the two-hole order pin.")
  (input
    (do
      (def
        (pick chunks holes)
        (match holes (#list(a b c) (Ast.List #list(c a b))) (_ (Ast.List #list()))))
      (def
        (main)
        (match
          (tagged-template pick (chunks "" "" "" "") (holes (Ast.Int 1) (Ast.Int 2) (Ast.Int 3)))
          ((Ast.List #list((Ast.Int x) (Ast.Int y) (Ast.Int z))) (+ (* x 100N) (+ (* y 10N) z)))
          (_ 0N)))
      (export main)))
  (output (: 312 BigInt))
  (live-objects known-leak))

; --- Composition: a hole may itself be a quote/Ast expression ---------------------------------------
; A `{expr}` hole is an ORDINARY expression, so it may be a `(quote …)` (or any Ast-valued expression) —
; the two metaprogramming surfaces compose. The hole is parsed as one expression and lowered to `Ast`, so
; a quote in a hole reaches the tag function as the `Ast` value it denotes, exactly like a hand-written
; `Ast.*`. Pins that the tagged-template hole surface layers cleanly over quote/quasiquote.
(case
  "a quote inside a tagged-template hole reaches the tag function as an Ast value"
  (doc
    "A hole's expression may be a `(quote …)`: `first\"a{quote (+ 1 2)}b\"` (canonical
           `(tagged-template first (chunks \"a\" \"b\") (holes (quote (+ 1 2))))`) passes the quote's `Ast`
           value — `(Ast.List (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2))` — as hole 0. The `first` tag
           returns it; `List.len` of its elements is 3. Pins that the hole surface composes with quote (the
           hole is an ordinary expression lowered to `Ast`, so a quote reaches the tag fn like a
           hand-written `Ast.*`).")
  (input
    (do
      (def (first chunks holes) (match holes (#list(h) h) (_ (Ast.Int 0))))
      (def
        (main)
        (match
          (tagged-template first (chunks "a" "b") (holes (quote (+ 1 2))))
          ((Ast.List es) (List.len es))
          (_ 0)))
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
(case
  "a tagged template nested in another's hole composes — the inner's result reaches the outer tag"
  (doc
    "A `(tagged-template …)` inside another's hole: `inner` returns `(Ast.Int 5)`, and `outer` wraps
           its single hole as `(Ast.List (Ast.Name \"+\") <hole> (Ast.Int 40))`. The expander rewrites both
           original nodes, so the outer receives the inner's EXPANSION (`(Ast.Int 5)`) as hole 0 and builds
           `(+ 5 40)`; the match scores `byte-len(\"+\") + 5 + 40 = 46`. Pins that nested tagged-template
           expansion composes — the outer tag consumes the inner's result, not the raw template node, and
           the two rewrites in one scan do not interfere.")
  (input
    (do
      (def (inner chunks holes) (Ast.Int 5))
      (def
        (outer chunks holes)
        (match
          holes
          (#list(h) (Ast.List #list((Ast.Name "+") h (Ast.Int 40))))
          (_ (Ast.List #list()))))
      (def
        (main)
        (match
          (tagged-template
            outer
            (chunks "" "")
            (holes (tagged-template inner (chunks "x") (holes))))
          ((Ast.List #list((Ast.Name op) (Ast.Int a) (Ast.Int b)))
            (+ (BigInt.of ((. String byte-len) op)) (+ a b)))
          (_ 0N)))
      (export main)))
  (output (: 46 BigInt))
  (live-objects known-leak))

; --- The expander is a STRUCTURAL rewrite, not a validator: the chunks==holes+1 invariant is the READER's
; The chunks/holes count invariant (`chunks.len() == holes.len() + 1`) is guaranteed by the READER on the
; surface path (a `tag"…{}…"` literal always yields well-balanced chunks/holes); `tagged_template::expand`
; deliberately DOES NOT re-check it (its docstring says so) — it rewrites any well-SHAPED 4-child
; `(tagged-template <tag> (chunks …) (holes …))` node to `(<tag> (list c…) (list h…))` regardless of the
; count relationship. So a canonical node WRITTEN DIRECTLY (as this file does) with a mismatched count still
; expands: the tag function receives whatever chunk/hole lists it was given. This pins that split — a future
; change that added a count re-check in the expander would break directly-written nodes (and duplicate the
; reader's job), so the structural-not-validating contract is locked here.
(case
  "the expander rewrites a directly-written node even when chunks != holes+1 (no re-check)"
  (doc
    "The reader guarantees `chunks.len() == holes.len() + 1`, but the expander does NOT re-check it — a
           canonical node written directly with 3 chunks and 1 hole (violating the invariant) still expands.
           `id` ignores its args and returns `(Ast.Int 0)`, so `(tagged-template id (chunks \"a\" \"b\" \"c\")
           (holes (Ast.Int 1)))` expands and folds to 0. Pins that the expander is a STRUCTURAL rewrite (any
           well-shaped 4-child node), not a validator — the count invariant is the reader's job, not
           duplicated here (a future re-check would break directly-written nodes).")
  (input
    (do
      (def (id chunks holes) (Ast.Int 0))
      (def
        (main)
        (match
          (tagged-template id (chunks "a" "b" "c") (holes (Ast.Int 1)))
          ((Ast.Int n) n)
          (_ -1N)))
      (export main)))
  (output (: 0 BigInt)))

(case
  "the expander threads the hole list to the tag even with zero chunks and zero holes"
  (doc
    "The degenerate shape: zero chunks and zero holes. The expander still rewrites `(tagged-template id
           (chunks) (holes))` to `(id (list) (list))`; `id` returns `(Ast.Int 5)`, folding to 5. Pins that
           empty chunk/hole lists are threaded (both `(list)`), not a decline — the structural rewrite has no
           lower bound on element count.")
  (input
    (do
      (def (id chunks holes) (Ast.Int 5))
      (def (main) (match (tagged-template id (chunks) (holes)) ((Ast.Int n) n) (_ -1N)))
      (export main)))
  (output (: 5 BigInt)))

(case
  "a tag function INTROSPECTS its chunk and hole counts and returns both"
  (doc
    "The count-introspection face: `count-meta` reads `(List.len chunks)` and `(List.len holes)` —
           the tag sees the template's SHAPE as data, not only its content — and returns both in an
           Ast.List the match verifies (3 chunks, 2 holes → 42). A DSL that dispatches on arity (a
           printf-style tag validating hole count against format specifiers) rests on exactly this; an
           expander that passed truncated or padded lists would misreport a count.")
  (input
    (do
      (def
        (count-meta chunks holes)
        (Ast.List
          #list((Ast.Int (BigInt.of (List.len chunks))) (Ast.Int (BigInt.of (List.len holes))))))
      (def
        (main (: n Int64))
        (match
          (tagged-template
            count-meta
            (chunks "a" "b" "c")
            (holes (Ast.Int (BigInt.of 1)) (Ast.Int (BigInt.of 2))))
          ((Ast.List #list((Ast.Int c) (Ast.Int h)))
            (if (= c (BigInt.of 3)) (if (= h (BigInt.of 2)) (+ 42 n) -2) -1))
          (_ -3)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

; --- Runtime values through the expansion seam ------------------------------------------------------
; Every case above is fully compile-time (no `(call …)`). These pin the seam between the one-tier
; compile-time expansion and RUNTIME values: the expansion happens once at compile time, but its residual
; must be ordinary code over the function's runtime parameters — an expander that re-runs per call, or
; that snapshots a runtime-fed hole at fold time, diverges below.
(case
  "a compile-time expansion residual combines with a runtime param per call"
  (doc
    "`two` returns `(Ast.Int 2)` (fully compile-time), but the surrounding match arm adds the
           BOUNDARY PARAMETER `a`: `(+ a n)`. The template expands and folds ONCE at compile time to the
           residual `(+ a 2)`, which then computes per call — a=40 → 42, a=-2 → 0. Pins that the
           compile-time tier's output composes with runtime data as ordinary code (the expansion is not
           re-entered per call, and the residual is not constant-folded past the runtime operand).")
  (input
    (do
      (def (two chunks holes) (Ast.Int 2))
      (def
        (main (: a Int64))
        (match (tagged-template two (chunks "") (holes)) ((Ast.Int n) (+ a (Int64.of n))) (_ 0)))
      (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64))
  (call main (: -2 Int64))
  (output (: 0 Int64)))

(case
  "a RUNTIME-dependent hole threads through the tag application per call"
  (doc
    "The hole is `(Ast.Int a)` — an Ast CONSTRUCTOR over the boundary parameter, so the hole's
           VALUE only exists at run time. The expander's structural rewrite to `(keep (list …) (list
           (Ast.Int a)))` happens at compile time, but `keep` (echoing its hole) cannot fold to a constant:
           the applied residual carries the runtime construction, and the match reads back whatever `a`
           the call supplied (7 → 7, 9 → 9). Pins that a hole is an ORDINARY EXPRESSION (this file's
           header) in the strongest sense — one whose value is runtime-dependent — and the expansion seam
           degrades gracefully from fold-to-constant to residual code.")
  (input
    (do
      (def (keep chunks holes) (match holes (#list(h) h) (_ (Ast.Int 0))))
      (def
        (main (: a Int64))
        (match
          (tagged-template keep (chunks "" "") (holes (Ast.Int (BigInt.of a))))
          ((Ast.Int n) n)
          (_ -1N)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 BigInt))
  (call main (: 9 Int64))
  (output (: 9 BigInt))
  (live-objects known-leak))

(case
  "a runtime hole woven into a compound Ast is read back by a nested match"
  (doc
    "The weave case's runtime companion: `wrap` builds `(Ast.List (list (Ast.Name \"f\") h))` around
           a hole whose payload is the runtime `(* a 10)`. The compound Ast is assembled at run time (its
           spine from the compile-time expansion, its leaf from the call), and the nested pattern
           destructures BOTH tiers — the statically-woven `(Ast.Name g)` (byte-len 1) and the
           runtime-filled `(Ast.Int n)` (a=4 → 40) — summing to 41. Pins that the woven compound keeps
           its static and runtime parts in their exact positions (a weave that reordered or re-folded
           the spine would misalign the nested destructure).")
  (input
    (do
      (def
        (wrap chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "f") h))) (_ (Ast.List #list()))))
      (def
        (main (: a Int64))
        (match
          (tagged-template wrap (chunks "" "") (holes (Ast.Int (BigInt.of (* a 10)))))
          ((Ast.List #list((Ast.Name g) (Ast.Int n))) (+ n (BigInt.of ((. String byte-len) g))))
          (_ -1N)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 41 BigInt))
  (live-objects known-leak))

(case
  "a tag DISPATCHES ON CHUNK TEXT to weave different operator spines around runtime holes"
  (doc
    "The content-directed DSL move — none of the pins above BRANCH on chunk content: `op`
           compares its first chunk string (\"add\" vs \"mul\") to choose the operator NAME woven
           into the Ast spine around two holes, i.e. the template TEXT is the program (the idiom a
           JSX-style library is built from). The holes carry a runtime (Ast.Int a) and a constant;
           the caller destructures the woven compound and computes THROUGH the chosen operator, so
           a dispatch that picked the wrong arm (or a weave that reordered the spine) diverges at
           one of the 2×2 faces. add/5 → 8; mul/5 → 15; add/0 → 3; mul/0 → 0 (the annihilator
           face — a wrong + would give 3).")
  (input
    (do
      (def
        (op chunks holes)
        (match
          chunks
          (#list(c _t)
            (match
              holes
              (#list(h0 h1) (Ast.List #list((Ast.Name (if (= c "add") "+" "*")) h0 h1)))
              (_ (Ast.Int 0))))
          (_ (Ast.Int 0))))
      (def
        (main (: a Int64) (: which Int64))
        (do
          (def
            r
            (if
              (= which 1)
              (match
                (tagged-template op (chunks "add" "") (holes (Ast.Int (BigInt.of a)) (Ast.Int 3)))
                ((Ast.List #list((Ast.Name o) (Ast.Int x) (Ast.Int y)))
                  (if (= o "+") (+ x y) (* x y)))
                (_ -1N))
              (match
                (tagged-template op (chunks "mul" "") (holes (Ast.Int (BigInt.of a)) (Ast.Int 3)))
                ((Ast.List #list((Ast.Name o) (Ast.Int x) (Ast.Int y)))
                  (if (= o "+") (+ x y) (* x y)))
                (_ -1N))))
          r))
      (export main)))
  (call main (: 5 Int64) (: 1 Int64))
  (output (: 8 BigInt))
  (call main (: 5 Int64) (: 2 Int64))
  (output (: 15 BigInt))
  (call main (: 0 Int64) (: 1 Int64))
  (output (: 3 BigInt))
  (call main (: 0 Int64) (: 2 Int64))
  (output (: 0 BigInt))
  (live-objects known-leak))

(case
  "a tag function recurses over its HOLES list and folds their sum"
  (doc
    "The recursive-tag pins recurse over CHUNK text; this tag recurses over the HOLES —
           `sum-holes` walks the `(List Ast)` of three `Ast.Int` holes with a rest-pattern, summing
           payloads into one `(Ast.Int 18)` (BigInt payload, narrowed at the consumer: 18 + k → 18
           at k=0, 118 at k=100). Pins the holes list as a first-class recursion subject at expansion
           time — a hole plumbing that delivered only the first hole (or reversed the list without
           consequence here but dropped one) folds to the wrong constant.")
  (input
    (do
      (def
        (sum-holes hs)
        (match
          hs
          (#list() (Ast.Int 0))
          (#list((Ast.Int n) (.. t))
            (match (sum-holes t) ((Ast.Int m) (Ast.Int (+ n m))) (_ (Ast.Int -999))))
          (_ (Ast.Int -998))))
      (def (tag chunks holes) (sum-holes holes))
      (def
        (main (: k Int64))
        (match
          (tagged-template tag (chunks "" "" "" "") (holes (Ast.Int 5) (Ast.Int 6) (Ast.Int 7)))
          ((Ast.Int n) (+ (Int64.of n) k))
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 18 Int64))
  (call main (: 100 Int64))
  (output (: 118 Int64))
  (live-objects known-leak))

(case
  "a compound Ast SUBTREE rides a template hole into the expansion intact"
  (doc
    "The hole pins splice Ast.Int LEAVES; this hole is a whole `(Ast.List (* 3 4))` SUBTREE the
           tag wraps as `(+ <hole> 10)` — the consumer match destructures BOTH layers and reads the
           inner list's arity, its leaf payloads, and the outer constant: 100·len(inner) + 3 + 4 + 10
           + k = 317 at k=0, 417 at k=100. A splice that flattened the subtree into the outer list
           (arity 5) or re-boxed the inner leaves changes the digits.")
  (input
    (do
      (def
        (tag chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "+") h (Ast.Int 10)))) (_ (Ast.Int -1))))
      (def
        (main (: k Int64))
        (match
          (tagged-template
            tag
            (chunks "" "")
            (holes (Ast.List #list((Ast.Name "*") (Ast.Int 3) (Ast.Int 4)))))
          ((Ast.List parts)
            (match
              parts
              (#list((Ast.Name op) (Ast.List inner) (Ast.Int c))
                (match
                  inner
                  (#list((Ast.Name op2) (Ast.Int a) (Ast.Int b))
                    (+
                      (* 100 (List.len inner))
                      (+ (Int64.of a) (+ (Int64.of b) (+ (Int64.of c) k)))))
                  (_ -4)))
              (_ -2)))
          (_ -3)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 317 Int64))
  (call main (: 100 Int64))
  (output (: 417 Int64)))

(case
  "a tagged template's TAG resolves through an import and expands at the importer's site"
  (doc
    "Metaprogramming × modules: the tag function `wrap` lives in a MODULE and the importer's
           `tagged-template` names it through the import — expansion (a COMPILE-TIME step) must
           resolve the tag binding across the module boundary and run it against the importer's
           chunks/holes (the module-local tag pins never cross a boundary). The expansion
           `(f <hole>)` is destructured at the consumer: hole payload 7 + k (7 at k=0, 107 at
           k=100). A template expander that resolved tags only in the current compilation unit
           reports the unbound-tag scope error instead.")
  (input
    (do
      (import "tags" (wrap))
      (def
        (main (: k Int64))
        (match
          (tagged-template wrap (chunks "" "") (holes (Ast.Int 7)))
          ((Ast.List parts)
            (match parts (#list((Ast.Name f) (Ast.Int n)) (+ (Int64.of n) k)) (_ -2)))
          (_ -3)))
      (export main)))
  (module "tags"
    (do
      (def
        (wrap chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "f") h))) (_ (Ast.Int -1))))
      (export wrap)))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: 100 Int64))
  (output (: 107 Int64)))

; --- Hole CONSUMPTION beyond positional 1:1: duplication (one hole, two slots) and permutation
; (exchanged delivery) with RUNTIME subtrees in the holes. ---
(case
  "a tag that DUPLICATES its single hole splices the same runtime subtree into both slots"
  (doc
    "The hole pins thread positionally 1:1; this tag consumes ONE hole TWICE (`(+ h h)`) — the sharing/copy semantics of splicing one runtime subtree into two slots (21 -> 42; a splice that consumed/moved the hole on first use breaks the second).")
  (input
    (do
      (def
        (twice chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "+") h h))) (_ (Ast.Int 0))))
      (def
        (main (: a Int64))
        (match
          (tagged-template twice (chunks "x" "y") (holes (Ast.Int (BigInt.of a))))
          ((Ast.List #list((Ast.Name o) (Ast.Int x) (Ast.Int y))) (if (= o "+") (+ x y) -2N))
          (_ -1N)))
      (export main)))
  (call main (: 21 Int64))
  (output (: 42 BigInt))
  (live-objects known-leak))

(case
  "a tag that SWAPS its two holes delivers each runtime subtree to the exchanged slot"
  (doc
    "The permutation face: the tag returns (h1 h0), read back asymmetrically (100x - y) so any delivery mix-up diverges ((3,7) -> 697). With the dup case: hole plumbing is a full function of holes, not a positional pass-through.")
  (input
    (do
      (def (swap chunks holes) (match holes (#list(h0 h1) (Ast.List #list(h1 h0))) (_ (Ast.Int 0))))
      (def
        (main (: a Int64) (: b Int64))
        (match
          (tagged-template
            swap
            (chunks "x" "y" "z")
            (holes (Ast.Int (BigInt.of a)) (Ast.Int (BigInt.of b))))
          ((Ast.List #list((Ast.Int x) (Ast.Int y))) (- (* (BigInt.of 100) x) y))
          (_ -1N)))
      (export main)))
  (call main (: 3 Int64) (: 7 Int64))
  (output (: 697 BigInt))
  (live-objects known-leak))

; --- A tag resolving through a re-export chain. ---
(case
  "a tag function resolves through a re-export CHAIN and expands at the entry's site"
  (doc
    "Composes the imported-tag pin (one boundary) with the transitive re-export chain: the
           tag lives in `tags`, `mid` re-exports it untouched, and the ENTRY's template names it
           through two hops — expansion (compile-time) must chase the chain exactly as value
           resolution does (7/107 with the runtime k at the consumer). A tag resolution that only
           looked one import deep reports the unbound-tag scope error at the entry.")
  (input
    (do
      (import "mid" (wrap))
      (def
        (main (: k Int64))
        (match
          (tagged-template wrap (chunks "" "") (holes (Ast.Int 7)))
          ((Ast.List parts)
            (match parts (#list((Ast.Name f) (Ast.Int n)) (+ (Int64.of n) k)) (_ -2)))
          (_ -3)))
      (export main)))
  (module "tags"
    (do
      (def
        (wrap chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "f") h))) (_ (Ast.Int -1))))
      (export wrap)))
  (module "mid"
    (do (import "tags" (wrap)) (export wrap)))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: 100 Int64))
  (output (: 107 Int64)))

; -- breaker batch 514 (2026-08-27): the runtime-param HOLE cell. A hole built from the runtime
; entry param ((Ast.Int (BigInt.of n))) flows through the compile-time tag weave and matches back
; carrying the runtime value — the tag runs at compile time over the hole's AST while the value
; flows at runtime, and the whole reconstruction reclaims. (The compute-with-a-template face is a
; permanent by-design reject with a teaching-quality CDZ0201 — "bind the template and match on it
; rather than computing with it" — already documented by this file's header, not pinned.)
(case
  "ttc1 a hole built from the runtime entry param round-trips through the tag weave"
  (input
    (do
      (def (weave chunks holes) (match holes (#list(h) h) (_ (Ast.Int (BigInt.of 0)))))
      (def
        (main (: n Int64))
        (match
          (tagged-template weave (chunks "") (holes (Ast.Int (BigInt.of n))))
          ((Ast.Int b) (if (= b (BigInt.of n)) 7 -1))
          (_ 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))
