; Structural editing — a program transformation is a PROGRAM over the canonical representation, not a
; text patch. Witnesses agent-authoring.md §Structural Editing (the language exposes an interface to
; read and rewrite a program's canonical representation without textual patching) and the learning
; spec/learnings/2026-07-04-program-transformation-is-a-program.md (the structural read/rewrite
; interface is realized as ordinary Cadenza functions over a syntax-tree sum type; a refactoring is an
; ordinary Cadenza component whose input and output are canonical representations — the same rep→rep
; seam as `compile`, `Ast → Ast`).
;
; Why this belongs in the executable corpus and not only in prose: the whole affordance rests on the
; abstract syntax tree being an ORDINARY VALUE the language already manipulates (self-hosting-surface.md
; §A Program's Syntax Tree Is An Ordinary Value; type-system.md §The Abstract Syntax Tree Type Is An
; Ordinary Sum Type). So "edit a program structurally" is not a bespoke external protocol — it is
; "write a recursive function that maps a syntax-tree value to a syntax-tree value," exactly the shape
; a compiler's own passes take (compiler.cdz's `resolve`/`fold`/`lower` are each such a walk). The
; CORE cases below demonstrate the realization the seed already runs: a transformation
; over a user-declared syntax-tree sum, walked across function calls, that preserves meaning while it
; rewrites — the peephole/simplifier idiom an agent scripts to refactor code. The ASPIRATIONAL cases
; (declined until a later generation realizes them) pin the fuller surface: the same
; walk over the BUILT-IN `Ast` composed across calls, and a rewrite RULE written with a quote pattern
; that reads in the shape of the code it rewrites.
;
; What is NOT witnessed here, deliberately: the addressed edit-operation library
; (insert/replace/delete/move over a node addressed by path + content-derived id — the
; content-addressed-nodes default in options/structural-interface/) is a LIBRARY over the syntax-tree
; sum, layered above these primitives; its surface is pinned in that option and in agent-authoring.md,
; not invented here as executable syntax. A structural edit either yields a well-formed program or a
; machine-readable rejection (agent-authoring.md §Structural Edits Preserve Well-Formedness Or Report);
; the CORE cases show the "well-formed program" direction — a transformation whose result is itself a
; valid syntax tree that runs.
; ============================================================================================
; CORE — a transformation is a recursive function over a syntax-tree sum (the seed realizes these)
; ============================================================================================
; The foundation: a program's syntax is an ordinary sum value, and a pass over it is an ordinary
; recursive function that flows THROUGH function calls (compiler.cdz's `resolve` docstring records
; this exact property — "a recursive walk over a user sum, so it flows through calls"). These use a
; user-declared `Exp` sum standing in for a program's syntax tree; the built-in `Ast` is the same
; shape (the ASPIRATIONAL section carries its companion), so a transformation authored over either is
; the same construct — an `Ast → Ast` (here `Exp → Exp`) function.
(diagnostic-quality)

(case
  "a program's syntax tree is an ordinary value walked recursively across calls"
  (doc
    "self-hosting-surface.md §A Program's Syntax Tree Is An Ordinary Value + §The Language
           Expresses A Compiler Over Its Own Syntax: a syntax tree is an ordinary sum value, and a
           compiler/tool determines a node's kind and recurses over its children by ordinary `match`
           and recursion — the walk flows through function calls. `eval` here is the archetypal
           tree-walk: a mutually-recursive descent over the `Exp` sum evaluating an arithmetic tree.
           `(3*4)+5` evaluates to 17. This is the substrate every structural pass (resolve, fold,
           lower — and a refactoring) is built on.")
  (input
    (do
      (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
      (def (main) (eval (Add #tuple((Mul #tuple((Lit 3) (Lit 4))) (Lit 5)))))
      (def
        (eval e)
        (match
          e
          ((Exp.Lit n) n)
          ((Exp.Add #tuple(a b)) (+ (eval a) (eval b)))
          ((Exp.Mul #tuple(a b)) (* (eval a) (eval b)))))
      (export main)))
  (output (: 17 Int64))
  (live-objects 0))

; The BUILT-IN-Ast companion of the user-`Exp` recursive walk above: a recursive function descends the
; built-in `Ast` sum by matching `(Ast.List es)` and recursing into its `(List Ast)` payload via a
; list-rest pattern `(list h .. _)`. `depth (quote (f (g 1)))` = 2 (the `Ast.List` → head `f` is an
; `Ast.Name`, so `1 + depth(f)` = 2). Pins that explicit head-recursion over the built-in Ast's recursive
; `List` payload COMPILES + runs on all three backends — the idiomatic tree-walk substrate. (NB: the
; List.fold-CLOSURE form of this walk — a recursive closure re-entering the fn over an Ast fold element —
; currently compile-stack-overflows, filed as a bug routed to v-inference; this explicit-recursion form is
; the working boundary beside it, and a compiles-cleanly guard for the fold form lands with that fix.)
(case
  "a recursive walk over the built-in Ast recurses into its List payload via a rest pattern"
  (doc
    "A recursive `depth` over the built-in `Ast`: `(Ast.List es)` matches and recurses into the
           `(List Ast)` payload via the list-rest pattern `(list h .. _)`, taking the head child. Over
           `(quote (f (g 1)))` the outer `Ast.List`'s head `f` is an `Ast.Name` (a leaf, depth 1), so the
           result is `1 + 1 = 2`. Pins the idiomatic explicit-recursion tree-walk over the built-in Ast's
           recursive payload — the built-in-Ast companion of the user-`Exp` recursive eval above.")
  (input
    (do
      (def
        (depth node)
        (match node ((Ast.List es) (match es (#list() 1) (#list(h (.. _)) (+ 1 (depth h))))) (_ 1)))
      (def (main) (depth (quote (f (g 1)))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a transformation maps a syntax tree to a syntax tree and preserves meaning"
  (doc
    "The core of spec/learnings/2026-07-04-program-transformation-is-a-program.md: a refactoring
           is an ordinary function from the canonical representation to the canonical representation
           (`Exp → Exp` here, `Ast → Ast` in general). `simp` is a peephole simplifier — it rewrites
           `(+ e 0)→e`, `(+ 0 e)→e`, `(* e 1)→e`, `(* 1 e)→e` bottom-up (children simplified first,
           then the local rule fires). Applied to `(6*1)+0`, it rewrites to `6`. The case asserts the
           transformation is SEMANTICS-PRESERVING: the rewritten tree evaluates to the SAME value as
           the original (`(= (eval e) (eval (simp e)))` is true) — the property that makes a refactor a
           refactor. (Payload literals are compared by binding then `=` via `is-lit`, since a
           constructor pattern binds its payload rather than matching a nested literal directly.)")
  (input
    (do
      (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
      (def
        (main)
        (let ((e (Add #tuple((Mul #tuple((Lit 6) (Lit 1))) (Lit 0))))) (= (eval e) (eval (simp e)))))
      (def (is-lit e k) (match e ((Exp.Lit n) (= n k)) (_ false)))
      (def
        (simp e)
        (match
          e
          ((Exp.Lit n) (Exp.Lit n))
          ((Exp.Add #tuple(a b))
            (let
              ((x (simp a)))
              (let ((y (simp b))) (if (is-lit y 0) x (if (is-lit x 0) y (Exp.Add #tuple(x y)))))))
          ((Exp.Mul #tuple(a b))
            (let
              ((x (simp a)))
              (let ((y (simp b))) (if (is-lit y 1) x (if (is-lit x 1) y (Exp.Mul #tuple(x y)))))))))
      (def
        (eval e)
        (match
          e
          ((Exp.Lit n) n)
          ((Exp.Add #tuple(a b)) (+ (eval a) (eval b)))
          ((Exp.Mul #tuple(a b)) (* (eval a) (eval b)))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

; The simp case above WORKS AROUND a limitation, worth pinning directly: it simplifies children with
; `let`-bound `x`/`y` and probes them with the single-scrutinee `is-lit` helper (its doc notes "a
; constructor pattern binds its payload rather than matching a nested literal directly") — rather than the
; natural bottom-up fold that matches a TUPLE of its two recursive results with constructor patterns in
; the tuple positions. That natural form — `(match (tuple (fold a) (fold b)) ((tuple (E.Lit x) (E.Lit y))
; …) ((tuple fa fb) …))` — is the archetypal constant-fold pass (recursively fold the children, then
; combine two constant leaves), and it is a valid program: the same fold written with SEPARATE nested
; single-scrutinee matches (`(match (fold a) ((E.Lit x) (match (fold b) …)) …)`) compiles and yields the
; same result (12 for the tree below). But the tuple-of-recursive-self-calls form is DECLINED
; ("constructor pattern against unresolved scrutinee" / "match scrutinee is not compile-time-resolvable"):
; the compiler cannot resolve the shape of a value produced by a recursive self-call of the enclosing
; function within that function's own body, so it cannot check the constructor pattern against the tuple
; element. The general capability exists — a tuple of two RUNTIME sums from NON-recursive producers matches
; with constructor patterns fine, and a single recursive-self-call result matches fine — only the
; combination (a TUPLE whose elements are recursive self-calls, matched with CONSTRUCTOR patterns) is not
; yet realized. It is decline-don't-miscompile-safe (an honest decline with a working equivalent), so a
; generation that does not yet resolve a recursive self-call's shape at the pattern site declines rather
; than miscompiling; the recorded output is the value the realized form produces (matching the is-lit
; workaround's semantics).
(case
  "a bottom-up fold matches a tuple of its recursive results with constructor patterns"
  (doc
    "The natural constant-fold pass: `fold` recursively simplifies an expression's two children,
           then matches the TUPLE of the two recursive results `(tuple (fold a) (fold b))` with
           constructor patterns `(tuple (E.Lit x) (E.Lit y))` to combine two constant leaves into one
           (`(Add (Lit x) (Lit y)) → (Lit (x+y))`), falling through to rebuild `(Add (tuple fa fb))`
           otherwise. Folding `(Add (Lit 3) (Add (Lit 4) (Lit 5)))` collapses to `(Lit 12)`, so `ev` of
           the result is 12. The archetypal optimizer idiom a self-hosted compiler is written in — fold
           the children, then pattern-match the folded pair to fire a rewrite rule. This pins that the
           compiler resolves a recursive SELF-call's result shape at a tuple pattern site with constructor
           patterns (a generation that does not yet do so declines \"constructor pattern against unresolved
           scrutinee\" rather than miscompiling; the sibling case below pins the same shape where the tuple
           elements are calls to a DIFFERENT function taking a recursive-sum argument).")
  (input
    (do
      (type E (Lit Int64) (Add (Tuple E E)))
      (def
        (fold e)
        (match
          e
          ((E.Lit n) (E.Lit n))
          ((E.Add #tuple(a b))
            (match
              #tuple((fold a) (fold b))
              (#tuple((E.Lit x) (E.Lit y)) (E.Lit (+ x y)))
              (#tuple(fa fb) (E.Add #tuple(fa fb)))))))
      (def (ev e) (match e ((E.Lit n) n) ((E.Add #tuple(a b)) (+ (ev a) (ev b)))))
      (def (main) (ev (fold (E.Add #tuple((E.Lit 3) (E.Add #tuple((E.Lit 4) (E.Lit 5))))))))
      (export main)))
  (output (: 12 Int64))
  (live-objects known-leak))

; The tuple-of-recursive-results constructor match (above) is realized for SELF-recursive calls; the
; SIBLING shape — the tuple elements are calls to a DIFFERENT function whose argument is a value of the
; recursive sum type — must work too, since a call's result shape is unresolved for the same reason
; (its argument's shape, a recursive sum, is not statically resolvable) whether the call is to the
; enclosing function or to another. `classify` takes a recursive `E` and returns an `Option Int64`
; (`Some n` for a leaf, `None` for a compound); `comb` recurses over `E` and, in the `Add` arm, matches
; the TUPLE of `(classify a)` and `(classify b)` with constructor patterns `(tuple (Some x) (Some y))`.
; This is a valid program — the SAME logic written with separate nested single-scrutinee matches on
; `(classify a)` then `(classify b)` yields the same value (7 for `(Add (Lit 3) (Lit 4))`), and the
; identical tuple-of-two-Option-producers matched with constructor patterns works when the producer takes
; a NON-recursive (e.g. Int64) argument. Only the combination — a tuple whose elements are calls to a
; function taking a RECURSIVE-SUM argument, matched with constructor patterns — is unrealized: the
; compiler cannot resolve the call's result shape at the pattern site because the recursive-sum argument's
; shape is not statically resolvable. It is the same self-hosting optimizer idiom as the self-call case
; above (a rewrite that dispatches on the classification of its recursively-processed children). A
; generation that does not yet resolve such a call's result shape at the pattern site declines
; ("constructor pattern against unresolved scrutinee") rather than miscompiling.
(case
  "a tuple of calls to a sibling function on recursive-sum values matches with constructor patterns"
  (doc
    "The sibling of the self-recursive fold above: `comb` recurses over a recursive sum `E` and, in
           its `Add` arm, matches the TUPLE of `(classify a)` and `(classify b)` — calls to a DIFFERENT
           function `classify : E → Option Int64` — with constructor patterns `(tuple (Some x) (Some y))`.
           For `(Add (Lit 3) (Lit 4))` both children classify to `Some`, so it yields 3+4 = 7. This is a
           valid program: the same logic with separate nested single-scrutinee matches on `(classify a)`
           then `(classify b)` yields 7, and the identical tuple-of-Option-producers with constructor
           patterns works when the producer takes a NON-recursive argument. Only the combination — a tuple
           whose elements are calls to a function taking a RECURSIVE-SUM argument, matched with constructor
           patterns — is unrealized: the compiler cannot resolve the call's result shape at the pattern
           site because the recursive-sum argument's shape is not statically resolvable, the same reason
           the self-call case needed. A generation that does not yet resolve such a call's result shape at
           the pattern site declines rather than miscompiling.")
  (input
    (do
      (type E (Lit Int64) (Add (Tuple E E)))
      (def (classify e) (match e ((E.Lit n) (Some n)) ((E.Add _) (None unit))))
      (def
        (comb e)
        (match
          e
          ((E.Lit n) n)
          ((E.Add #tuple(a b))
            (match #tuple((classify a) (classify b)) (#tuple((Some x) (Some y)) (+ x y)) (_ -1)))))
      (def (main) (comb (E.Add #tuple((E.Lit 3) (E.Lit 4)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a transformation observably rewrites the tree, not just its value"
  (doc
    "The companion to the meaning-preservation case: the transformation is not a no-op — it
           changes the STRUCTURE. `size` counts nodes; the redundant tree `(6*1)+0` has 5 nodes
           (Add, Mul, Lit 6, Lit 1, Lit 0) and `simp` collapses it to the single node `6`, so the
           rewrite eliminates 4 nodes. Together with the previous case (meaning preserved) this is the
           full statement of a sound refactor: the tree changed, the meaning did not. An agent scripts
           exactly this — a function over the syntax tree whose result it can measure and re-check.")
  (input
    (do
      (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
      (def
        (main)
        (let ((e (Add #tuple((Mul #tuple((Lit 6) (Lit 1))) (Lit 0))))) (- (size e) (size (simp e)))))
      (def (is-lit e k) (match e ((Exp.Lit n) (= n k)) (_ false)))
      (def
        (simp e)
        (match
          e
          ((Exp.Lit n) (Exp.Lit n))
          ((Exp.Add #tuple(a b))
            (let
              ((x (simp a)))
              (let ((y (simp b))) (if (is-lit y 0) x (if (is-lit x 0) y (Exp.Add #tuple(x y)))))))
          ((Exp.Mul #tuple(a b))
            (let
              ((x (simp a)))
              (let ((y (simp b))) (if (is-lit y 1) x (if (is-lit x 1) y (Exp.Mul #tuple(x y)))))))))
      (def
        (size e)
        (match
          e
          ((Exp.Lit n) 1)
          ((Exp.Add #tuple(a b)) (+ 1 (+ (size a) (size b))))
          ((Exp.Mul #tuple(a b)) (+ 1 (+ (size a) (size b))))))
      (export main)))
  (output (: 4 Int64))
  (live-objects 0))

(case
  "the built-in Ast is transformed as an ordinary value"
  (doc
    "metaprogramming.md §Quote Produces An AST Value + type-system.md §The Abstract Syntax Tree
           Type Is An Ordinary Sum Type: a `quote`d program is an ordinary `Ast` sum value, transformed
           by the same `match`/construct mechanism as any sum. Here a one-node rewrite maps the integer
           literal `5` to `105` by matching `(Ast.Int n)` and reconstructing `(Ast.Int (+ n 100))` —
           the built-in-`Ast` analogue of the `Exp` rewrites above. Demonstrated INLINE (the seed
           realizes the built-in `Ast` within a single definition); the ASPIRATIONAL companion below
           composes the same rewrite across a function boundary, the shape a real pass takes.")
  (input
    (do
      (def
        (main)
        (let
          ((e (quote 5)))
          (match (match e ((Ast.Int n) (Ast.Int (+ n 100N))) (o o)) ((Ast.Int r) r) (_ 0N))))
      (export main)))
  (output (: 105 BigInt))
  (live-objects known-leak))

; ============================================================================================
; ASPIRATIONAL — the fuller structural-editing surface (a later generation realizes these)
; ============================================================================================
; These pin the contract the realization must meet. The seed declines a case whose capability it does
; not realize (conformance-gate.md §A Generation Is Judged Against The Capabilities It Realizes), so
; these document the target and are scored todo until realized — exactly as the quote-pattern cases in
; 12-metaprogramming.sexp are.
; The CORE cases walk a USER sum across calls (the seed's realized path). The built-in `Ast` is an
; ordinary sum type of the same shape (type-system.md §The Abstract Syntax Tree Type Is An Ordinary Sum
; Type), so a transformation over it MUST compose across function calls identically: a pass is a
; recursive `Ast → Ast` function, and the top-level driver hands each subtree to the same function. The
; seed today realizes the built-in `Ast` only INLINE within a definition (the core `quote 5` case
; above), not flowing a `quote`d value through a call — so this companion, which factors the rewrite
; into a reusable `bump` applied via a call, is tagged for the generation that closes that gap.
(case
  "a transformation over the built-in Ast composes across a function call"
  (doc
    "The built-in-`Ast` companion to the core user-sum cases: a syntax-tree pass is a recursive
           function that flows through calls, whether the tree is a user sum (the core cases) or the
           built-in `Ast` (here). `bump` maps every integer literal node to its successor and rebuilds
           every other node; applied to `(quote 7)` through a call it yields `(Ast.Int 8)`. This is the
           built-in-`Ast` realization of program-transformation-is-a-program; the seed realizes the
           built-in `Ast` only inline (core case above), so composing it across a boundary is the
           increment a later generation lands; until then the seed declines it.")
  (input
    (do
      (def (main) (= (bump (quote 7)) (Ast.Int 8)))
      (def (bump node) (match node ((Ast.Int n) (Ast.Int (+ n 1N))) (other other)))
      (export main)))
  (output (: true Bool)))

; A REWRITE RULE reads in the shape of the code it rewrites. The quote pattern `` `(+ ,x 0) `` IS the
; constructor pattern `(Ast.List (list (Ast.Name "+") x (Ast.Int 0)))` (metaprogramming.md §A
; Quasiquote In Pattern Position Destructures An AST; the equivalence is witnessed in
; 12-metaprogramming.sexp), but an agent writes it in the surface shape of the arithmetic identity it
; encodes — so the paren-bookkeeping is READ ONCE by the reader, never counted against a live buffer.
; This is the "scriptable editing" sweet spot: a peephole rule set that looks like the algebra it
; performs. It requires quote-patterns (the pattern-position quote lowering) — and it also relies
; on the built-in `Ast` flowing across a call, so it is realized no earlier than the companion above.
(case
  "a peephole rewrite rule reads in the shape of the code it rewrites"
  (doc
    "The scriptable-refactor payoff: a rewrite rule written with quote patterns reads as the
           identities it encodes. `` `(+ ,x 0) `` matches an addition whose second operand is the
           literal 0 and binds the first operand `x` (a literal subterm — the `0` — matches
           `(Ast.Int 0)` by equality; `,x` binds the sub-tree — metaprogramming.md §A Quasiquote In
           Pattern Position Destructures An AST). So `simp` rewrites `(+ x 0) ⇒ x` and `(* x 1) ⇒ x`;
           applied to `(quote (+ x 0))` it yields `(quote x)`. The rule set looks like the algebra it
           performs — the agent authors intent, not delimiter bookkeeping. It requires quote-patterns;
           also relies on the built-in `Ast` composing across a call (companion above).")
  (input
    (do
      (def (main) (= (simp (quote (+ x 0))) (quote x)))
      (def
        (simp node)
        (match
          node
          ((quasiquote (+ (unquote x) 0)) x)
          ((quasiquote (* (unquote x) 1)) x)
          (other other)))
      (export main)))
  (output (: true Bool)))

; A quote pattern NESTS structurally in pattern position: a template with a compound INSIDE a compound
; matches a two-level-deep tree in ONE pattern, binding an unquote at the inner level. This is the
; pattern-position dual of a nested quote CONSTRUCTION, and is distinct from (a) the root-only peephole
; rule above (which matches only at the top and would need a recursive `simp` self-call to reach a nested
; redex — a shape the rust backend does not yet fold, so pinned non-recursively here) and (b) the eval-
; based recursive descent in 12-metaprogramming (which EVALUATES rather than matches). Pins that
; `` `(+ (+ ,x 0) 0) `` reads as the doubly-nested `(Ast.List (list (Ast.Name "+") (Ast.List (list
; (Ast.Name "+") x (Ast.Int 0))) (Ast.Int 0)))` pattern and binds `x` at depth 2 — a printer/lowering
; that flattened the nesting or mis-scoped the inner unquote would break it. Green on all backends.
(case
  "OVERLAPPING quote patterns dispatch first-match-wins on the ambiguous scrutinee"
  (doc
    "The peephole rule set above is DISJOINT; a real simplifier's rules OVERLAP, and rule
           PRIORITY is semantics: `(+ ,_x 0) and `(+ 0 ,_x) BOTH match (+ 0 0) — arm 1 must win.
           An engine that reordered or unified overlapping quote rows rewrites with the wrong rule.
           (+ 0 5) matches only arm 2; (+ 5 3) falls through. Encoded 1·100 + 2·10 + 0 = 120.
           All pattern literals are 0 (the nonzero row is the HELD recursive-BigInt miscompile).")
  (input
    (do
      (def
        (classify node)
        (match node ((quasiquote (+ (unquote _x) 0)) 1) ((quasiquote (+ 0 (unquote _x))) 2) (_ 0)))
      (def
        (main)
        (+
          (* (classify (quote (+ 0 0))) 100)
          (+ (* (classify (quote (+ 0 5))) 10) (classify (quote (+ 5 3))))))
      (export main)))
  (output (: 120 Int64)))

(case
  "a nested quote pattern matches a compound-within-a-compound two levels deep"
  (doc
    "A single quote pattern with a compound inside a compound — `` `(+ (+ ,x 0) 0) `` — matches a
           two-level-deep addition `(+ (+ y 0) 0)` in ONE pattern, binding the inner operand `x` (= the
           name `y`) at depth 2, so `simp` returns `(quote y)`. Pins that quote patterns nest structurally
           in pattern position (the dual of nested quote construction), reaching an unquote below the top
           level. Distinct from the root-only peephole rule above and the eval-descent case in
           12-metaprogramming — this is a pure structural MATCH two levels down, no recursion, no eval.")
  (input
    (do
      (def (main) (= (simp (quote (+ (+ y 0) 0))) (quote y)))
      (def (simp node) (match node ((quasiquote (+ (+ (unquote x) 0) 0)) x) (other other)))
      (export main)))
  (output (: true Bool)))

; --- Runtime-valued and runtime-SHAPED trees through the same walks ---------------------------------
; Every case above transforms a tree that is fully known at compile time, so in principle the whole
; walk could grade by const-folding alone. These pin the transformations as RESIDUAL CODE: the tree's
; leaf value — and, stronger, its SHAPE — comes from a boundary parameter, so one compiled walk must
; dispatch per call. This is the seam an agent-authored refactoring actually runs on (a tree read at
; run time, not a literal in the source), and the shape a compiler pass takes when its input arrives
; as a value.
(case
  "a one-node Ast rewrite over a runtime leaf computes per call"
  (doc
    "The built-in-Ast rewrite case above, with the leaf RUNTIME: `(Ast.Int a)` is constructed from
           the boundary parameter, rewritten by the same `(Ast.Int n) → (Ast.Int (+ n 100))` match, and
           read back. a=5 → 105 exactly as the const case; a=-100 → 0 (the rewrite's output leaf, not the
           read-back match's fall-through — `-100+100 = 0` flows through the `(Ast.Int r)` arm). Pins that
           the rewrite is residual code over a runtime Ast value, not a compile-time fold that memoized
           the const case's answer.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((e (Ast.Int (BigInt.of a))))
          (match (match e ((Ast.Int n) (Ast.Int (+ n 100N))) (o o)) ((Ast.Int r) r) (_ 0N))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 BigInt))
  (call main (: -100 Int64))
  (output (: 0 BigInt))
  (live-objects known-leak))

(case
  "a recursive walk over a runtime-SHAPED tree dispatches per call"
  (doc
    "Stronger than a runtime leaf: the tree's SHAPE is chosen at run time — `build` returns the
           two-node `(Add (Lit a) (Lit 10))` for positive `a` and the single-leaf `(Lit a)` otherwise, so
           the recursive `sum-lits` walk cannot be unrolled against a known spine. a=5 walks two leaves
           (15); a=-7 walks one (-7). Pins that a syntax-tree walk is genuinely recursive residual code:
           its depth and arm sequence are decided by the value that arrives, the situation every
           agent-authored transformation over a read-at-runtime program is in.")
  (input
    (do
      (type Exp (Lit Int64) (Add Exp Exp))
      (def (sum-lits (: e Exp)) (match e ((Lit n) n) ((Add l r) (+ (sum-lits l) (sum-lits r)))))
      (def (build (: a Int64)) (if (> a 0) (Add (Lit a) (Lit 10)) (Lit a)))
      (def (main (: a Int64)) (sum-lits (build a)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64))
  (call main (: -7 Int64))
  (output (: -7 Int64))
  (live-objects 0))

(case
  "a rewrite-then-eval pipeline over a runtime tree preserves meaning through the rewrite"
  (doc
    "The full pipeline at run time: `build` assembles `(Add (Lit 0) (Add (Lit a) (Lit 2)))` around
           the boundary parameter, `simplify` rewrites away the `(Add (Lit 0) r)` identity (recursing into
           the survivor), and `eval-exp` evaluates the rewritten tree — a=40 → 42. The rewrite runs BEFORE
           evaluation on a tree whose leaf is unknown to it; dropping the zero node must not drop the
           runtime leaf beneath it. Pins the meaning-preservation contract (this file's opening: a
           transformation preserves meaning while it rewrites) as residual code — the peephole idiom
           applied to a value, not a literal.")
  (input
    (do
      (type Exp (Lit Int64) (Add Exp Exp))
      (def
        (simplify (: e Exp))
        (match
          e
          ((Add (Lit 0) r) (simplify r))
          ((Add l r) (Add (simplify l) (simplify r)))
          ((Lit n) (Lit n))))
      (def (eval-exp (: e Exp)) (match e ((Lit n) n) ((Add l r) (+ (eval-exp l) (eval-exp r)))))
      (def (build (: a Int64)) (Add (Lit 0) (Add (Lit a) (Lit 2))))
      (def (main (: a Int64)) (eval-exp (simplify (build a))))
      (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a rewrite whose fall-through arm reads a switched slot WHOLE round-trips through the cadenza backend"
  (doc
    "The Neg-elimination pass `drop-negs`: its first arm `(Add (Neg x) r)` DESTRUCTURES slot 0 to
           its `Neg` payload, so the decision tree switches slot 0 on the discriminant; the SECOND arm
           `(Add l r)` is the fall-through for a non-`Neg` slot 0 and its body reads `l` — the WHOLE first
           slot. Meaning: `(Add (Neg (Lit a)) (Lit 2))` → strip one negation → `(Add (Lit a) (Lit 2))`,
           eval a+2 = 42 at a=40. This pins the `--target cadenza` re-emit (M4a): the flat sum-match loop
           REGISTERS a whole-slot binder for slot 0 but the flattened default arm would emit `_`, dropping
           it — so a `Core::SumPayload` read of the whole slot dangled to a never-emitted binder → CDZ0101
           `unbound name _cdz_m0` on recompile. The fall-through wildcard now binds the pre-registered
           whole-slot binder (a binder matches anything, semantically identical to `_`), so the body's read
           resolves and the emitted cadenza AST recompiles. Green on all backends; the cadenza round-trip is
           the witness this case protects.")
  (input
    (do
      (type Exp (Lit Int64) (Neg Exp) (Add Exp Exp))
      (def
        (eval-exp (: e Exp))
        (match
          e
          ((Lit n) n)
          ((Neg x) (- 0 (eval-exp x)))
          ((Add l r) (+ (eval-exp l) (eval-exp r)))))
      (def
        (drop-negs (: e Exp))
        (match
          e
          ((Add (Neg x) r) (drop-negs (Add x r)))
          ((Add l r) (Add (drop-negs l) (drop-negs r)))
          ((Neg x) (Neg (drop-negs x)))
          ((Lit n) (Lit n))))
      (def (main (: a Int64)) (eval-exp (drop-negs (Add (Neg (Lit a)) (Lit 2)))))
      (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64))
  (live-objects known-leak))

; --- The NONZERO recursive-BigInt-literal-probe row (breaker FINDING #22), now closed ---------------
; The peephole cases above use only literal-0 patterns; the doc at "OVERLAPPING quote patterns" notes the
; NONZERO row was a HELD wasm miscompile. Root (v-wasm-opt): a BigInt sum-payload literal-test's collect
; walk resolved the payload via a hardcoded variant-0 import while emit used the ENTERED variant, so a
; recursive self-call inside a quasiquote-pattern arm that matched a const-folded quote emitted a call to
; an unresolved func index (u32::MAX sentinel) → invalid wasm module. FIXED wasm by a2e7bea0d (collect
; uses ty_at_path_recorded, the entered variant); the runtime-scrutinee compare by 5505b5010; and the
; RUST literal-probe render (was E0605 `Big as i64`) by ecadf1221 (compares by BigInt equality). These pin
; the row the corpus never covered because every landed peephole used literal 0. (The plain-sum CONSTANT
; `(Mk 1)` face — FACE-B — is separately HELD for rust: af3e8531f, the const-nominal-peel, is queued.)
(case
  "a NONZERO BigInt literal probe in a recursive quasiquote-pattern simp matches its own constructor"
  (doc
    "The nonzero row of the peephole simp (breaker FINDING #22): `simp`'s quasiquote arm
           `` `(* ,x 1) `` carries the NONZERO literal `1` (an `Ast.Int` = BigInt payload) and RECURSES
           `(simp x)` on a match. Applied to `(quote (* y 1))` it rewrites to `(quote y)`, an `Ast.Name`,
           so the probe returns 40. This was a wasm invalid-module (the recursive self-call in a
           quasiquote arm matching a const-folded quote emitted an unresolved func index) until a2e7bea0d
           made collect resolve the BigInt payload via the ENTERED variant, not a hardcoded variant-0
           import. Rust declines the non-scalar literal-payload probe honestly (todo), so this is also a
           rust-coverage marker for when that renders. wasm 40.")
  (input
    (do
      (def (simp node) (match node ((quasiquote (* (unquote x) 1)) (simp x)) (other other)))
      (def (main) (match (simp (quote (* y 1))) ((Ast.Name _n) 40) (_ -1)))
      (export main)))
  (output (: 40 Int64))
  (live-objects known-leak))

(case
  "a runtime-built BigInt sum-payload literal probe matches its constructor"
  (doc
    "The runtime-scrutinee companion (no quote/Ast): a plain sum `(type W (Mk BigInt))` whose payload
           is built from a RUNTIME parameter `(Mk (BigInt.of k))`, probed against the nonzero literal
           `(Mk 1)`. At k=1 the probe matches → 40. This exercises the BigInt-payload literal-compare on the
           emitted path (fixed on wasm by 5505b5010's entered-variant compare, and on rust by ecadf1221's
           BigInt-equality render, replacing the prior `Big as i64` E0605 build-fail). Green on all
           backends — the runtime-value analogue of the const-quote FACE above.")
  (input
    (do
      (type W (Mk BigInt))
      (def (main (: k Int64)) (match (Mk (BigInt.of k)) ((Mk 1) 40) (_ -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 40 Int64))
  (live-objects 0))

(case
  "a BigInt-newtype payload built from BigInt ARITHMETIC round-trips through the cadenza backend"
  (doc
    "The arithmetic sibling of the runtime-built BigInt sum-payload probe above: `(type W (Mk
           BigInt))`'s payload is built by BigInt ADDITION `(Mk (+ (BigInt.of k) (BigInt.of 1)))`, then
           probed against the literal `(Mk 5)`. At k=4 the sum is 5 → matches → 40. This pins the
           `--target cadenza` re-emit of a newtype whose erased payload is a `Core::BigIntBinOp` (the
           arbitrary-precision arithmetic tower's `+`): the newtype value's Core IS the bignum add, an
           intrinsic BigInt PRODUCER, so it is a CONSTRUCTION site here exactly like `(BigInt.of k)` or a
           default-width `Arith` — without it the erased value fell to nominal_disposition's ambiguous
           `_ => Decline` → CDZ0900. Green on all backends; the cadenza round-trip is the witness.")
  (input
    (do
      (type W (Mk BigInt))
      (def (main (: k Int64)) (match (Mk (+ (BigInt.of k) (BigInt.of 1))) ((Mk 5) 40) (_ -1)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 40 Int64))
  (live-objects 0))

(case
  "a runtime-built BigInt sum-payload literal probe falls through on a non-matching payload"
  (doc
    "The miss companion: the same `(match (Mk (BigInt.of k)) ((Mk 1) 40) (_ -1))` at k=2 does NOT
           match the literal `1` and takes the wildcard → -1. Confirms the emitted BigInt literal compare
           is a genuine value test (not a zeroed/always-true probe — the falsified pattern-1-vs-input-2
           case that a hardcoded-variant-0 or `Big as i64` cast would have mis-decided). Green on all
           backends, the negative twin of the match case above.")
  (input
    (do
      (type W (Mk BigInt))
      (def (main (: k Int64)) (match (Mk (BigInt.of k)) ((Mk 1) 40) (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a plain-sum recursive CONSTANT BigInt literal probe matches its own constructor"
  (doc
    "The CONSTANT-scrutinee companion of the runtime probes above (breaker FINDING #22 FACE-B): a
           minimal plain sum `(type W (Mk BigInt))` recursively walked, probing a CONSTANT `(Mk 1)` against
           the nonzero literal `(Mk 1)` → 40. This was originally an INVALID wasm module (the const (Mk 1)
           materialized its BigInt payload as a raw i64, not a heap leaf) AND a rust build-fail
           (error[E0308]: mismatched types — is_bigint_valued didn't strip the erased newtype's nominal, so
           the const fell to the int-literal path). FIXED across the stack: wasm const-fold by v-inference
           77e8ca8b1, rust const path by v-rust-backend add3eca3a (is_bigint_valued strips the nominal so
           the const goes through const_big_expr). Green on all three backends — completes the
           nonzero-recursive-BigInt-probe matrix (FACE-A quote + runtime probe + this const face).")
  (input
    (do
      (type W (Mk BigInt))
      (def
        (walk (: n Int64) (: w W))
        (if (< n 1) -1 (match w ((Mk 1) 40) (_ (walk (- n 1) w)))))
      (def (main) (walk 2 (Mk 1)))
      (export main)))
  (output (: 40 Int64)))

(case
  "TWO distinct nonzero BigInt literal probes dispatch in one recursive fn"
  (doc
    "#22-fix perimeter: the pins above are single-literal; TWO distinct nonzero probes
           ((Mk 1)→10, (Mk 5)→50) in one recursive fn share the entered-variant collect and the
           const pool — a fix that tracked one literal per fn would misdispatch one arm. Runtime
           scrutinee via (Mk (BigInt.of k)); miss face -1.")
  (input
    (do
      (type W (Mk BigInt))
      (def
        (walk (: n Int64) (: w W))
        (match
          w
          ((Mk 1) (if (= n 0) 10 (walk (- n 1) w)))
          ((Mk 5) (if (= n 0) 50 (walk (- n 1) w)))
          (_ -1)))
      (def (main (: k Int64)) (walk 2 (Mk (BigInt.of k))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10 Int64))
  (call main (: 5 Int64))
  (output (: 50 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64)))

(case
  "a nonzero BigInt literal probe dispatches across MUTUAL recursion post-fix"
  (doc
    "#22-fix perimeter: the pre-fix matrix showed wrapper-recursion breaking; this pins the
           fixed entered-variant collect crossing FUNCTION boundaries (fa⇄fb, parity-dependent
           10/20 by depth) — not just self-calls. Miss face -1.")
  (input
    (do
      (type W (Mk BigInt))
      (def (fa (: n Int64) (: w W)) (match w ((Mk 1) (if (= n 0) 10 (fb (- n 1) w))) (_ -1)))
      (def (fb (: n Int64) (: w W)) (match w ((Mk 1) (if (= n 0) 20 (fa (- n 1) w))) (_ -1)))
      (def (main (: n Int64) (: k Int64)) (fa n (Mk (BigInt.of k))))
      (export main)))
  (call main (: 2 Int64) (: 1 Int64))
  (output (: 10 Int64))
  (call main (: 3 Int64) (: 1 Int64))
  (output (: 20 Int64))
  (call main (: 2 Int64) (: 9 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a nonzero BigInt probe over a scrutinee REBUILT each recursive frame dispatches every time"
  (doc
    "#22-fix perimeter: the scrutinee (Mk (BigInt.of k)) is allocated FRESH each recursive
           frame (not threaded), so the probe compares a NEW heap value against the const-pool
           literal per iteration — per-frame allocation + probe + reclaim under recursion. Base-case
           + miss faces.")
  (input
    (do
      (type W (Mk BigInt))
      (def
        (walk (: n Int64) (: k Int64))
        (match (Mk (BigInt.of k)) ((Mk 1) (if (= n 0) 40 (walk (- n 1) k))) (_ -1)))
      (def (main (: n Int64) (: k Int64)) (walk n k))
      (export main)))
  (call main (: 2 Int64) (: 1 Int64))
  (output (: 40 Int64))
  (call main (: 0 Int64) (: 1 Int64))
  (output (: 40 Int64))
  (call main (: 2 Int64) (: 9 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a recursive RENAME pass rewrites every matching Name leaf at any depth and counts them"
  (doc
    "The alpha-rename pass shape: mutually recursive ren/ren-list REBUILD the tree while
           COUNTING rewrites (count·10 + whole-tree structural eq vs the hand-built expectation —
           shape preservation AND leaf rewrites both witnessed; no-match control). Uses a BINDER +
           string-compare rather than a Name-literal probe — the string-payload literal probe is
           the rust not-yet; binder+= is the portable spelling.")
  (input
    (do
      (def
        (ren node)
        (match
          node
          ((Ast.Name nm) (if (= nm "x") #tuple((Ast.Name "y") 1) #tuple((Ast.Name nm) 0)))
          ((Ast.List xs) (ren-list xs #list() 0))
          (other #tuple(other 0))))
      (def
        (ren-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64))
        (match
          xs
          (#list() #tuple((Ast.List acc) k))
          (#list(h (.. t)) (match (ren h) (#tuple(h2 k2) (ren-list t (List.push acc h2) (+ k k2)))))))
      (def
        (main (: mode Int64))
        (do
          (def
            t
            (if
              (= mode 1)
              (Ast.List
                #list((Ast.Name "f") (Ast.Name "x") (Ast.List #list((Ast.Name "g") (Ast.Name "x")))))
              (Ast.List #list((Ast.Name "f") (Ast.Name "z")))))
          (match
            (ren t)
            (#tuple(t2 k)
              (+
                (* k 10)
                (if
                  (=
                    t2
                    (if
                      (= mode 1)
                      (Ast.List
                        #list((Ast.Name "f")
                          (Ast.Name "y")
                          (Ast.List #list((Ast.Name "g") (Ast.Name "y")))))
                      (Ast.List #list((Ast.Name "f") (Ast.Name "z")))))
                  1
                  0))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; A mutually-recursive fold that rebuilds an Ast list, then reads a payload derived from the
; rebuilt-list binder (`xs2`) while a sibling arm reuses that same binder, MUST build on every
; backend. Regression: on the rust/rust-async backends this used to fail to build with
; `E0382: borrow of moved value` (the non-Copy tuple/record field read moved the value out from
; under the sibling reuse); wasm already computed 102. Fixed by cloning the non-Copy field read
; so the sibling arm re-reads it intact. (breaker/corpus-bugfix routed → v-rust-backend f4ae338d1.)
(case
  "a mutually-recursive fold matching a rebuilt list with a payload binder builds and computes"
  (input
    (do
      (def
        (fold node)
        (match
          node
          ((Ast.List xs)
            (match
              (fold-list xs #list() 0)
              (#tuple(xs2 k)
                (match
                  xs2
                  (#list((Ast.Int a)) #tuple((Ast.Int a) (+ k 1)))
                  (_ #tuple((Ast.List xs2) k))))))
          (other #tuple(other 0))))
      (def
        (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64))
        (match
          xs
          (#list() #tuple(acc k))
          (#list(h (.. t))
            (match (fold h) (#tuple(h2 k2) (fold-list t (List.push acc h2) (+ k k2)))))))
      (def
        (main (: n Int64))
        (match (fold (Ast.List #list((Ast.Int (BigInt.of n))))) (#tuple(_r k) (+ (* k 100) n))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 102 Int64))
  (live-objects known-leak))

(case
  "Record.with on a map-extracted record leaves the stored original untouched"
  (doc
    "Structural edit through a collection extraction: the record comes OUT of a map (lookup +
           expect), `Record.with` replaces its x, and the edited copy is re-inserted into a NEW map —
           three observables: the ORIGINAL map still reads x=10 (persistence through the with), the
           new map reads the edited x=k, and the edited record's untouched y rides along (encoded
           1000·10 + 10·k + 20: 10090 at k=7, 10020 at k=0). A with that edited the extracted record
           in place (sharing the map's slot) corrupts the first digit block.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 #record((= x 10) (= y 20)))))
          (def r (Option.expect (Map.lookup m 1) "present"))
          (def r2 (Record.with r #"x" k))
          (def m2 (Map.insert m 1 r2))
          (+
            (* 1000 (. (Option.expect (Map.lookup m 1) "p") x))
            (+ (* 10 (. (Option.expect (Map.lookup m2 1) "p") x)) r2.y))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 10090 Int64))
  (call main (: 0 Int64))
  (output (: 10020 Int64))
  (live-objects known-leak))

; --- Fixpoint rewriting and payload-derived renames. ---
(case
  "a rewrite iterates to a FIXPOINT detected by structural equality and fully collapses a nest"
  (doc
    "The pass-manager idiom over the single-pass walks above: an identity-elimination step iterated by `fix` whose CONVERGENCE TEST is structural = on the recursive sum (heap value-eq in the loop condition — an = that missed a difference stops early leaving a redex; one that never says equal burns fuel and returns a non-Lit). 1·(0+(1·k)) collapses to Lit k.")
  (input
    (do
      (type Ex (Add (Tuple Ex Ex)) (Mul (Tuple Ex Ex)) (Lit Int64))
      (def
        (step (: e Ex))
        (match
          e
          ((Ex.Add #tuple(a b))
            (match
              #tuple((step a) (step b))
              (#tuple((Ex.Lit x) sb) (if (= x 0) sb (Ex.Add #tuple((Ex.Lit x) sb))))
              (#tuple(sa sb) (Ex.Add #tuple(sa sb)))))
          ((Ex.Mul #tuple(a b))
            (match
              #tuple((step a) (step b))
              (#tuple((Ex.Lit x) sb) (if (= x 1) sb (Ex.Mul #tuple((Ex.Lit x) sb))))
              (#tuple(sa sb) (Ex.Mul #tuple(sa sb)))))
          ((Ex.Lit n) (Ex.Lit n))))
      (def
        (fix (: e Ex) (: fuel Int64))
        (do (def e2 (step e)) (if (= fuel 0) e2 (if (= e2 e) e (fix e2 (- fuel 1))))))
      (def
        (ev (: e Ex))
        (match
          e
          ((Ex.Add #tuple(a b)) (+ (ev a) (ev b)))
          ((Ex.Mul #tuple(a b)) (* (ev a) (ev b)))
          ((Ex.Lit n) n)))
      (def
        (main (: k Int64))
        (do
          (def
            prog
            (Ex.Mul
              #tuple((Ex.Lit 1) (Ex.Add #tuple((Ex.Lit 0) (Ex.Mul #tuple((Ex.Lit 1) (Ex.Lit k))))))))
          (def slim (fix prog 10))
          (+ (* 100 (ev slim)) (match slim ((Ex.Lit _v) 1) (_ 0)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 701 Int64))
  (live-objects known-leak))

(case
  "a rename pass DERIVES each new Name from the old payload (concat suffix), quote-verified deep"
  (doc
    "The :572 rename swaps in a FIXED name; this DERIVES the new name from the OLD payload ((Ast.Name (String.concat n \"_v2\")) — the suffix-refactor idiom: the Name payload String is read AND a fresh derived String re-wrapped at every depth). Deep face quote-verified; non-Name leaf control.")
  (input
    (do
      (def
        (rename node)
        (match
          node
          ((Ast.Name n) (Ast.Name (String.concat n "_v2")))
          ((Ast.List xs) (Ast.List (ren-all xs #list())))
          (other other)))
      (def
        (ren-all (: xs (List Ast)) (: acc (List Ast)))
        (match xs (#list() acc) (#list(h (.. t)) (ren-all t (List.push acc (rename h))))))
      (def
        (main)
        (+
          (* 10 (if (= (rename (quote (f x (g y)))) (quote (f_v2 x_v2 (g_v2 y_v2)))) 1 0))
          (if (= (rename (quote 42)) (quote 42)) 1 0)))
      (export main)))
  (output (: 11 Int64))
  (live-objects 0))

; --- Guards over quote patterns (subtree-equality rewrites). ---
(case
  "a GUARD over a QUOTE pattern compares two bound subtrees — the x=x algebraic rewrite"
  (doc
    "Quote patterns and guards composed (the algebraic-rewrite idiom needs both): the guard compares TWO quote-bound subtrees by Ast value-equality — x+x rewrites to 2*x, x+y falls through untouched. Both directions quote-verified.")
  (input
    (do
      (def
        (simp node)
        (match
          node
          ((guard (quasiquote (+ (unquote x) (unquote y))) (= x y))
            (Ast.List #list((Ast.Name "*") (Ast.Int 2) x)))
          (other other)))
      (def
        (main)
        (+
          (* 10 (if (= (simp (quote (+ z z))) (quote (* 2 z))) 1 0))
          (if (= (simp (quote (+ a b))) (quote (+ a b))) 1 0)))
      (export main)))
  (output (: 11 Int64)))

; --- A guard destructuring its quote-bound subtree (nested match + splice-back). ---
(case
  "a guard runs a NESTED MATCH on the quote-bound subtree — value-dependent rewriting"
  (doc
    "Deeper than the subtree-eq guard: the guard DESTRUCTURES the bound subtree with a whole nested match ((match x ((Ast.Int n) (< n limit)) …)) gating the arm, and the arm SPLICES x back into a new template. lit-5 rewrites (payload under limit); lit-50 falls through. Ast.Int payload is BigInt, so the compare widens via BigInt.of.")
  (input
    (do
      (def
        (rewrite node (: limit Int64))
        (match
          node
          ((guard
              (quasiquote (lit (unquote x)))
              (match x ((Ast.Int n) (< n (BigInt.of limit))) (_ false)))
            (quasiquote (small (unquote x))))
          (other other)))
      (def
        (main)
        (+
          (* 10 (if (= (rewrite (quote (lit 5)) 10) (quote (small 5))) 1 0))
          (if (= (rewrite (quote (lit 50)) 10) (quote (lit 50))) 1 0)))
      (export main)))
  (output (: 11 Int64)))

(case
  "a simplifier pass applied TWICE equals one application (idempotence at the fixpoint)"
  (doc
    "The pipeline property the refactor-is-a-program story rests on: `simp` (the peephole
           simplifier from the meaning-preservation case above) reaches its FIXPOINT in one bottom-up
           application on this tree, so `(simp (simp e)) = (simp e)` as TREE values (deep sum
           equality) — and the fixpoint is the fully-reduced `(Lit k)`. A rewrite that left a residual
           reducible node (a dropped child rewrite, a top-only rule) re-fires on the second pass and
           splits the equality; the once-result being `(Lit k)` pins that one pass fully reduces this
           tree rather than both passes being equally lazy.")
  (input
    (do
      (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
      (def (is-lit e k) (match e ((Exp.Lit n) (= n k)) (_other false)))
      (def
        (simp e)
        (match
          e
          ((Exp.Lit n) (Exp.Lit n))
          ((Exp.Add #tuple(a b))
            (let
              ((x (simp a)))
              (let ((y (simp b))) (if (is-lit y 0) x (if (is-lit x 0) y (Exp.Add #tuple(x y)))))))
          ((Exp.Mul #tuple(a b))
            (let
              ((x (simp a)))
              (let ((y (simp b))) (if (is-lit y 1) x (if (is-lit x 1) y (Exp.Mul #tuple(x y)))))))))
      (def
        (main (: k Int64))
        (let
          ((e (Add #tuple((Mul #tuple((Lit k) (Lit 1))) (Lit 0)))))
          (let ((once (simp e))) (+ (* 10 (if (= (simp once) once) 1 0)) (if (= once (Lit k)) 1 0)))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 11 Int64))
  (live-objects known-leak))

; ── breaker batch 570: the TREE face of the sum-spine leak family (the ss/sp cells are LINEAR
; chains; these are BRANCHING walks — the structural-editing substrate shape). A depth-4 binary
; Exp tree's eval leaks the ENTIRE tree (46 = 15 Add+tuple pairs + 16 Lit boxes, exact); a
; transform-then-eval leaks BOTH trees (44 = input 22 + output 22 at depth 3). Values exact.
; Calibration for the reclaim arc: the two-shell fix targets self-loop-TAIL chains; branching
; non-tail recursion is the harder later face — these clauses key its landing.
(case
  "stt1 a runtime-built binary Exp tree evals exactly and RECLAIMS the whole tree (branching-walk calibration)"
  (input
    (do
      (type Exp (Lit Int64) (Add (Tuple Exp Exp)))
      (def (mk (: d Int64)) (if (= d 0) (Exp.Lit 1) (Exp.Add #tuple((mk (- d 1)) (mk (- d 1))))))
      (def (eval (: e Exp)) (match e ((Exp.Lit n) n) ((Exp.Add #tuple(a b)) (+ (eval a) (eval b)))))
      (def (main (: n Int64)) (eval (mk n)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 16 Int64))
  (live-objects 0))

(case
  "stt2 a tree-to-tree transform then eval preserves meaning and RECLAIMS BOTH trees (transform calibration)"
  (input
    (do
      (type Exp (Lit Int64) (Add (Tuple Exp Exp)))
      (def (mk (: d Int64)) (if (= d 0) (Exp.Lit 1) (Exp.Add #tuple((mk (- d 1)) (mk (- d 1))))))
      (def
        (dbl (: e Exp))
        (match
          e
          ((Exp.Lit n) (Exp.Lit (* n 2)))
          ((Exp.Add #tuple(a b)) (Exp.Add #tuple((dbl a) (dbl b))))))
      (def (eval (: e Exp)) (match e ((Exp.Lit n) n) ((Exp.Add #tuple(a b)) (+ (eval a) (eval b)))))
      (def (main (: n Int64)) (eval (dbl (mk n))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 16 Int64))
  (live-objects 0))

; ── breaker batch 571: constant QUOTES join the build-once family (verified: 3 static globals —
; previously undocumented in the constant-kind matrix) but the WALK over a hoisted immortal Ast
; leaks ~10 mortal cells PER WALK (the sum-payload extraction dups never release — the Ast face
; of the walk-leak family). aq1 = hoist + two walks (20); aq2 = fifty walks, exactly linear (500).
; Both flip with the reclaim arc; the hoist facts hold regardless.
(case
  "aq1 constant quotes hoist build-once and each depth-walk over the immortal Ast leaks its extraction dups"
  (input
    (do
      (def
        (depth (: node Ast))
        (match
          node
          ((Ast.List es) (match es (#list() 1) (#list(h (.. rest)) (+ 1 (depth h)))))
          (_ 1)))
      (def
        (main (: n Int64))
        (let
          ((a (if (> n 0) (quote (f (g 1))) (quote x))) (b (quote (f (g 1)))))
          (+ (* 100 (depth a)) (depth b))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 202 Int64))
  (live-objects known-leak))

(case
  "aq2 fifty depth-walks over a hoisted constant quote leak LINEARLY (per-walk extraction dups)"
  (input
    (do
      (def
        (depth (: node Ast))
        (match
          node
          ((Ast.List es) (match es (#list() 1) (#list(h (.. rest)) (+ 1 (depth h)))))
          (_ 1)))
      (def
        (frames (: k Int64))
        (if (= k 0) 0 (+ (depth (if (> k 0) (quote (f (g 1))) (quote y))) (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 100 Int64))
  (live-objects known-leak))

; ── breaker batch 573: runtime Ast CONSTRUCTION cells (the constructor face; quotes covered by
; aq1/2). ac1 = the identity contract: a runtime-built Ast (constructors, BigInt payload from the
; arg) is structurally EQUAL to the quoted constant, discriminates against the wrong quote, and
; fully reclaims. ac2 = the built-Ast walk calibration: build+walk leaks ~17/frame (linear ×10) —
; the runtime-construction face of the walk-leak family (with aq2 = the hoisted-quote face).
(case
  "ac1 a runtime-BUILT Ast equals the quoted constant structurally and reclaims clean (the construction/quote identity contract)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((built (Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of n))))))
          (+ (if (= built (quote (f 1))) 1000 0) (if (= built (quote (f 2))) 100 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1000 Int64))
  (live-objects 0))

(case
  "ac2 ten build+walk frames over runtime-constructed Ast chains leak linearly (the construction face of the walk-leak)"
  (input
    (do
      (def
        (depth (: node Ast))
        (match
          node
          ((Ast.List es) (match es (#list() 1) (#list(h (.. rest)) (+ 1 (depth h)))))
          (_ 1)))
      (def
        (mk (: d Int64))
        (if (= d 0) (Ast.Int (BigInt.of d)) (Ast.List #list((mk (- d 1)) (Ast.Int (BigInt.of d))))))
      (def (frames (: k Int64)) (if (= k 0) 0 (+ (depth (mk 3)) (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 10 Int64))
  (output (: 40 Int64))
  (live-objects known-leak))

; sex1: a peephole REWRITE over the built-in Ast that RETURNS an Ast — the rewrite companion of the
; built-in-Ast walk above (which only reads a depth). `simp` maps Ast->Ast, stripping `(+ e 0)` to
; `e` bottom-up (children simplified first) while leaving every other head untouched. Distinct from
; the `simp` over the user `Exp` sum in the CORE section: this is the SAME transformation over the
; language's own `Ast` value, the shape a real refactor/agent script takes. Witnessed structurally
; via the rewritten tree's DEPTH (a value the fold can compare): `(+ (+ a 0) 0)` collapses to the
; bare `a` (Ast.Name, depth 0), while `(+ (* a 0) 0)` collapses only the outer `+0` — the inner
; `(* a 0)` is a different head, preserved (depth 1). A rewrite that folded the wrong head, or failed
; to recurse, diffs the depth. VALUE is correct on every backend; the wasm live-objects census shows
; the rebuilt Ast.List intermediates LEAK (missing-Perceus-drop in the Ast-rewrite path — the same
; class as the chapter-16 utf8-decode baselines), so pinned `(live-objects known-leak)` until the
; drop lands. (breaker probe se1/se2, tick 1514; verified tri-target VALUE-exact + byte-idempotent;
; leak filed to v-memory-safety.)
(case
  "a peephole rewrite over the built-in Ast returns a simplified Ast"
  (input
    (do
      (def
        (simp (: a Ast))
        (match
          a
          ((Ast.List es)
            (match
              es
              (#list(op l r)
                (match
                  r
                  ((Ast.Int z)
                    (if
                      (= z (BigInt.of 0))
                      (match
                        op
                        ((Ast.Name n)
                          (if (= n "+") (simp l) (Ast.List #list(op (simp l) (simp r)))))
                        (_ (Ast.List #list(op (simp l) (simp r)))))
                      (Ast.List #list(op (simp l) (simp r)))))
                  (_ (Ast.List #list(op (simp l) (simp r))))))
              (_ a)))
          (_ a)))
      (def
        (depth (: a Ast))
        (match a ((Ast.List es) (match es (#list(h (.. t)) (+ 1 (fold-max t))) (_ 1))) (_ 0)))
      (def
        (fold-max (: xs (List Ast)))
        (match xs (#list(h (.. t)) (let ((d (depth h)) (r (fold-max t))) (if (> d r) d r))) (_ 0)))
      (def
        (main (: n Int64))
        (+ (depth (simp (quote (+ (+ a 0) 0)))) (* 10 (depth (simp (quote (+ (* a 0) 0)))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))
