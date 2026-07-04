; Binding, scope, and control flow — witnesses core-semantics.md. Cases are s-expressions
; in the canonical homoiconic representation (options/code-shape/); a result is (: <value> <Type>),
; a rejected program records its diagnostic code (options/diagnostics-schema/), a runtime halt
; records a trap or (exhausted). See README.md for the case vocabulary.

(case "a let binding is in scope in its body"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical — a name resolves to its enclosing binding.")
  (input  (let ((x 10)) x))
  (output (: 10 Int64)))

(case "a name resolves to the nearest enclosing binding"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical.")
  (input  (let ((x 1)) (let ((x 2)) x)))
  (output (: 2 Int64)))

(case "an inner binding shadows an outer one only within its scope"
  (doc    "Witnesses core-semantics.md #Shadowing Is Well-Defined (which defers to the corpus):
           the inner x is 2 inside its let; the outer x is still 1 outside it, so the sum is 3.")
  (input  (+ (let ((x 2)) x) (let ((x 1)) x)))
  (output (: 3 Int64)))

(case "a reference to an unbound name is rejected before running"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical: a reference to a name with no enclosing
           binding is refused. This is a front-end rejection EVERY generation makes, including the
           dynamic seed — scope resolution needs no static typing — so (error CDZ0101) is the primary
           clause, not a (compiler …) divergence.")
  (input  y)
  (error  CDZ0101))

(case "a sequencing block yields the value of its last form"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (2nd sentence:
           a block evaluates to its last form's value). The earlier forms are pure here, so the block's
           only observable result is the last form; ordering of effects is witnessed in
           03-equality-and-observation.sexp.")
  (input  (do 1 2 3))
  (output (: 3 Int64)))

(case "a single-form body admits a sequence by holding a do block"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order in a
           single-form body position: a `let` body is one form, so a sequence of forms is written as a
           `(do …)` there. The prefix form is pure, so the block yields the value of its last form (the
           binding x), showing the do is the sequencing point and let scope is unchanged.")
  (input  (let ((x 4))
            (do
              (+ x 1)
              x)))
  (output (: 4 Int64)))

(case "a conditional evaluates only the selected branch"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch. The unselected branch would
           trap on overflow if it were evaluated; the normal result proves it was not.")
  (input  (if true 1 (+ Int64.max 1)))
  (output (: 1 Int64)))

(case "a conditional selects the false branch when the condition is false"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch.")
  (input  (if false 1 2))
  (output (: 2 Int64)))

(case "a pattern binds a name scoped to its branch"
  (doc    "Witnesses core-semantics.md #Bindings Introduced By A Pattern Are Scoped To Its Branch.
           Option is declared where used as (Some <value> | None) (options/code-shape/); the Some
           branch binds n to the payload, in scope only in that branch. Patterns are uniform:
           (Some n) for unary, (None _) for nullary — both single-arity.")
  (input  (match (Some 5)
            ((Some n) n)
            ((None _) 0)))
  (output (: 5 Int64)))

(case "matching on integer literals"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: a match can branch on
           literal values, not just constructors. Integer literal patterns match by equality. The
           compiler uses this to dispatch on instruction opcodes and section IDs.")
  (input  (match 2
            (0 "zero")
            (1 "one")
            (2 "two")
            (else "many")))
  (output (: "two" String)))

(case "matching on string literals"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns
           match by equality. The compiler uses this heavily to dispatch on instruction tags like
           'i64.const', 'i64.add', etc. — replacing nested if/= chains with readable match.")
  (input  (match "hello"
            ("hello" 1)
            ("world" 2)
            (else    0)))
  (output (: 1 Int64)))

(case "matching falls through to else when no literal matches"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: when no literal pattern
           matches, the else (wildcard) catches it. Without else, a non-exhaustive match traps.")
  (input  (match 99
            (0 "zero")
            (1 "one")
            (else "other")))
  (output (: "other" String)))

(case "nested patterns deconstruct recursively"
  (doc    "Witnesses core-semantics.md #Pattern Matching: patterns can nest — a constructor pattern
           inside another constructor pattern. (Some (tuple a b)) matches a Some whose payload is a
           tuple, binding both elements. The compiler uses this to deconstruct nested AST structures.")
  (input  (match (Some (tuple 3 7))
            ((Some (tuple a b)) (+ a b))
            ((None _)           0)))
  (output (: 10 Int64)))

(case "nested patterns with literals"
  (doc    "Witnesses core-semantics.md #Pattern Matching: nested patterns can combine constructors
           and literals. (Some 0) matches Some carrying exactly 0 — the literal refines the match.")
  (input  (match (Some 0)
            ((Some 0) "zero")
            ((Some _) "nonzero")
            ((None _) "none")))
  (output (: "zero" String)))

(case "deeply nested pattern matching"
  (doc    "The compiler pattern-matches over nested AST: a list node containing a name node.
           Patterns nest arbitrarily deep.")
  (needs  sum-type-declaration)
  (input  (do
            (type Expr (Lit Int64 | Add (Tuple Expr Expr)))
            (let ((e (Expr.Add (tuple (Expr.Lit 1) (Expr.Lit 2)))))
              (match e
                ((Expr.Lit n) n)
                ((Expr.Add (tuple (Expr.Lit a) (Expr.Lit b))) (+ a b))
                ((Expr.Add _) 0)))))
  (output (: 3 Int64)))
