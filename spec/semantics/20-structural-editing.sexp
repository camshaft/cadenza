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
; CORE cases below (no `(needs …)`) demonstrate the realization the seed already runs: a transformation
; over a user-declared syntax-tree sum, walked across function calls, that preserves meaning while it
; rewrites — the peephole/simplifier idiom an agent scripts to refactor code. The ASPIRATIONAL cases
; (tagged `(needs …)`, skipped until a later generation realizes them) pin the fuller surface: the same
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

(case "a program's syntax tree is an ordinary value walked recursively across calls"
  (doc    "self-hosting-surface.md §A Program's Syntax Tree Is An Ordinary Value + §The Language
           Expresses A Compiler Over Its Own Syntax: a syntax tree is an ordinary sum value, and a
           compiler/tool determines a node's kind and recurses over its children by ordinary `match`
           and recursion — the walk flows through function calls. `eval` here is the archetypal
           tree-walk: a mutually-recursive descent over the `Exp` sum evaluating an arithmetic tree.
           `(3*4)+5` evaluates to 17. This is the substrate every structural pass (resolve, fold,
           lower — and a refactoring) is built on.")
  (input  (module case
            (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
            (def (main) (eval (Add (tuple (Mul (tuple (Lit 3) (Lit 4))) (Lit 5)))))
            (def (eval e)
              (match e
                ((Exp.Lit n) n)
                ((Exp.Add (tuple a b)) (+ (eval a) (eval b)))
                ((Exp.Mul (tuple a b)) (* (eval a) (eval b)))))))
  (output (: 17 Int64)))

(case "a transformation maps a syntax tree to a syntax tree and preserves meaning"
  (doc    "The core of spec/learnings/2026-07-04-program-transformation-is-a-program.md: a refactoring
           is an ordinary function from the canonical representation to the canonical representation
           (`Exp → Exp` here, `Ast → Ast` in general). `simp` is a peephole simplifier — it rewrites
           `(+ e 0)→e`, `(+ 0 e)→e`, `(* e 1)→e`, `(* 1 e)→e` bottom-up (children simplified first,
           then the local rule fires). Applied to `(6*1)+0`, it rewrites to `6`. The case asserts the
           transformation is SEMANTICS-PRESERVING: the rewritten tree evaluates to the SAME value as
           the original (`(= (eval e) (eval (simp e)))` is true) — the property that makes a refactor a
           refactor. (Payload literals are compared by binding then `=` via `is-lit`, since a
           constructor pattern binds its payload rather than matching a nested literal directly.)")
  (input  (module case
            (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
            (def (main)
              (let ((e (Add (tuple (Mul (tuple (Lit 6) (Lit 1))) (Lit 0)))))
                (= (eval e) (eval (simp e)))))
            (def (is-lit e k) (match e ((Exp.Lit n) (= n k)) (_ false)))
            (def (simp e)
              (match e
                ((Exp.Lit n) (Exp.Lit n))
                ((Exp.Add (tuple a b))
                  (let ((x (simp a)))
                  (let ((y (simp b)))
                    (if (is-lit y 0) x (if (is-lit x 0) y (Exp.Add (tuple x y)))))))
                ((Exp.Mul (tuple a b))
                  (let ((x (simp a)))
                  (let ((y (simp b)))
                    (if (is-lit y 1) x (if (is-lit x 1) y (Exp.Mul (tuple x y)))))))))
            (def (eval e)
              (match e
                ((Exp.Lit n) n)
                ((Exp.Add (tuple a b)) (+ (eval a) (eval b)))
                ((Exp.Mul (tuple a b)) (* (eval a) (eval b)))))))
  (output (: true Bool)))

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

(case "a bottom-up fold matches a tuple of its recursive results with constructor patterns"
  (doc    "The natural constant-fold pass: `fold` recursively simplifies an expression's two children,
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
  (input  (module case
            (type E (Lit Int64) (Add (Tuple E E)))
            (def (fold e)
              (match e
                ((E.Lit n) (E.Lit n))
                ((E.Add (tuple a b))
                  (match (tuple (fold a) (fold b))
                    ((tuple (E.Lit x) (E.Lit y)) (E.Lit (+ x y)))
                    ((tuple fa fb)               (E.Add (tuple fa fb)))))))
            (def (ev e)
              (match e
                ((E.Lit n) n)
                ((E.Add (tuple a b)) (+ (ev a) (ev b)))))
            (def (main)
              (ev (fold (E.Add (tuple (E.Lit 3) (E.Add (tuple (E.Lit 4) (E.Lit 5))))))))))
  (output (: 12 Int64)))

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

(case "a tuple of calls to a sibling function on recursive-sum values matches with constructor patterns"
  (doc    "The sibling of the self-recursive fold above: `comb` recurses over a recursive sum `E` and, in
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
  (input  (module case
            (type E (Lit Int64) (Add (Tuple E E)))
            (def (classify e)
              (match e
                ((E.Lit n)  (Some n))
                ((E.Add _)  (None unit))))
            (def (comb e)
              (match e
                ((E.Lit n) n)
                ((E.Add (tuple a b))
                  (match (tuple (classify a) (classify b))
                    ((tuple (Some x) (Some y)) (+ x y))
                    (_                         -1)))))
            (def (main)
              (comb (E.Add (tuple (E.Lit 3) (E.Lit 4)))))))
  (output (: 7 Int64)))

(case "a transformation observably rewrites the tree, not just its value"
  (doc    "The companion to the meaning-preservation case: the transformation is not a no-op — it
           changes the STRUCTURE. `size` counts nodes; the redundant tree `(6*1)+0` has 5 nodes
           (Add, Mul, Lit 6, Lit 1, Lit 0) and `simp` collapses it to the single node `6`, so the
           rewrite eliminates 4 nodes. Together with the previous case (meaning preserved) this is the
           full statement of a sound refactor: the tree changed, the meaning did not. An agent scripts
           exactly this — a function over the syntax tree whose result it can measure and re-check.")
  (input  (module case
            (type Exp (Lit Int64) (Add (Tuple Exp Exp)) (Mul (Tuple Exp Exp)))
            (def (main)
              (let ((e (Add (tuple (Mul (tuple (Lit 6) (Lit 1))) (Lit 0)))))
                (- (size e) (size (simp e)))))
            (def (is-lit e k) (match e ((Exp.Lit n) (= n k)) (_ false)))
            (def (simp e)
              (match e
                ((Exp.Lit n) (Exp.Lit n))
                ((Exp.Add (tuple a b))
                  (let ((x (simp a)))
                  (let ((y (simp b)))
                    (if (is-lit y 0) x (if (is-lit x 0) y (Exp.Add (tuple x y)))))))
                ((Exp.Mul (tuple a b))
                  (let ((x (simp a)))
                  (let ((y (simp b)))
                    (if (is-lit y 1) x (if (is-lit x 1) y (Exp.Mul (tuple x y)))))))))
            (def (size e)
              (match e
                ((Exp.Lit n) 1)
                ((Exp.Add (tuple a b)) (+ 1 (+ (size a) (size b))))
                ((Exp.Mul (tuple a b)) (+ 1 (+ (size a) (size b))))))))
  (output (: 4 Int64)))

(case "the built-in Ast is transformed as an ordinary value"
  (doc    "metaprogramming.md §Quote Produces An AST Value + type-system.md §The Abstract Syntax Tree
           Type Is An Ordinary Sum Type: a `quote`d program is an ordinary `Ast` sum value, transformed
           by the same `match`/construct mechanism as any sum. Here a one-node rewrite maps the integer
           literal `5` to `105` by matching `(Ast.Int n)` and reconstructing `(Ast.Int (+ n 100))` —
           the built-in-`Ast` analogue of the `Exp` rewrites above. Demonstrated INLINE (the seed
           realizes the built-in `Ast` within a single definition); the ASPIRATIONAL companion below
           composes the same rewrite across a function boundary, the shape a real pass takes.")
  (input  (module case
            (def (main)
              (let ((e (quote 5)))
                (match (match e ((Ast.Int n) (Ast.Int (+ n 100))) (o o))
                  ((Ast.Int r) r)
                  (_ 0))))))
  (output (: 105 Int64)))

; ============================================================================================
; ASPIRATIONAL — the fuller structural-editing surface (a later generation realizes these)
; ============================================================================================
; These pin the contract the realization must meet; they are NOT seed declines. The seed skips a case
; whose `(needs …)` capability it does not realize (conformance-gate.md §A Generation Is Judged Against
; The Capabilities It Realizes), so these document the target without gating on it — exactly as the
; `(needs quote-patterns)` cases in 12-metaprogramming.sexp do.

; The CORE cases walk a USER sum across calls (the seed's realized path). The built-in `Ast` is an
; ordinary sum type of the same shape (type-system.md §The Abstract Syntax Tree Type Is An Ordinary Sum
; Type), so a transformation over it MUST compose across function calls identically: a pass is a
; recursive `Ast → Ast` function, and the top-level driver hands each subtree to the same function. The
; seed today realizes the built-in `Ast` only INLINE within a definition (the core `quote 5` case
; above), not flowing a `quote`d value through a call — so this companion, which factors the rewrite
; into a reusable `bump` applied via a call, is tagged for the generation that closes that gap.

(case "a transformation over the built-in Ast composes across a function call"
  (doc    "The built-in-`Ast` companion to the core user-sum cases: a syntax-tree pass is a recursive
           function that flows through calls, whether the tree is a user sum (the core cases) or the
           built-in `Ast` (here). `bump` maps every integer literal node to its successor and rebuilds
           every other node; applied to `(quote 7)` through a call it yields `(Ast.Int 8)`. This is the
           built-in-`Ast` realization of program-transformation-is-a-program; the seed realizes the
           built-in `Ast` only inline (core case above), so composing it across a boundary is the
           increment a later generation lands. `(needs builtin-ast-across-calls)`.")
  (needs  builtin-ast-across-calls)
  (input  (module case
            (def (main) (= (bump (quote 7)) (Ast.Int 8)))
            (def (bump node)
              (match node
                ((Ast.Int n) (Ast.Int (+ n 1)))
                (other other)))))
  (output (: true Bool)))

; A REWRITE RULE reads in the shape of the code it rewrites. The quote pattern `` `(+ ,x 0) `` IS the
; constructor pattern `(Ast.List (list (Ast.Name "+") x (Ast.Int 0)))` (metaprogramming.md §A
; Quasiquote In Pattern Position Destructures An AST; the equivalence is witnessed in
; 12-metaprogramming.sexp), but an agent writes it in the surface shape of the arithmetic identity it
; encodes — so the paren-bookkeeping is READ ONCE by the reader, never counted against a live buffer.
; This is the "scriptable editing" sweet spot: a peephole rule set that looks like the algebra it
; performs. Tagged `(needs quote-patterns)` (the pattern-position quote lowering) — and it also relies
; on the built-in `Ast` flowing across a call, so it is realized no earlier than the companion above.

(case "a peephole rewrite rule reads in the shape of the code it rewrites"
  (doc    "The scriptable-refactor payoff: a rewrite rule written with quote patterns reads as the
           identities it encodes. `` `(+ ,x 0) `` matches an addition whose second operand is the
           literal 0 and binds the first operand `x` (a literal subterm — the `0` — matches
           `(Ast.Int 0)` by equality; `,x` binds the sub-tree — metaprogramming.md §A Quasiquote In
           Pattern Position Destructures An AST). So `simp` rewrites `(+ x 0) ⇒ x` and `(* x 1) ⇒ x`;
           applied to `(quote (+ x 0))` it yields `(quote x)`. The rule set looks like the algebra it
           performs — the agent authors intent, not delimiter bookkeeping. `(needs quote-patterns)`;
           also relies on the built-in `Ast` composing across a call (companion above).")
  (needs  quote-patterns)
  (input  (module case
            (def (main) (= (simp (quote (+ x 0))) (quote x)))
            (def (simp node)
              (match node
                (`(+ ,x 0) x)
                (`(* ,x 1) x)
                (other     other)))))
  (output (: true Bool)))
