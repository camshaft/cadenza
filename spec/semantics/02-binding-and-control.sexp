; Binding, scope, and control flow — witnesses core-semantics.md. Cases are s-expressions
; in the canonical homoiconic representation (options/code-shape/); a result is (: <value> <Type>),
; a rejected program records its diagnostic code (options/diagnostics-schema/), a runtime halt
; records a trap. See README.md for the case vocabulary.

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

; --- The bindings of one `let` take effect in order (let*, not parallel) --------------------
; core-semantics.md #The Bindings Of One `let` Take Effect In Order: each binding's initializer sees
; the bindings written before it in the SAME let, so `(let ((x 1) (y (+ x 1))) y)` is 2 — `y`'s
; initializer observes `x`. Under a PARALLEL reading `y`'s initializer would evaluate in the enclosing
; scope where `x` is unbound (a CDZ0101 rejection); the sequential reading, which the seed realizes,
; is the recorded oracle.

(case "a later let binding sees an earlier one in the same let"
  (doc    "`(let ((x 1) (y (+ x 1))) y)` = 2: the second binding's initializer `(+ x 1)` observes the
           first binding `x`, so the bindings of one `let` take effect in order (core-semantics.md
           #The Bindings Of One `let` Take Effect In Order), not in parallel where `x` would be unbound
           in `y`'s initializer.")
  (input  (let ((x 1) (y (+ x 1))) y))
  (output (: 2 Int64)))

(case "a repeated let binding shadows the earlier one for what follows"
  (doc    "`(let ((x 1) (x (+ x 10))) x)` = 11: the second binding of `x` shadows the first for the
           initializers and body that follow, and its initializer `(+ x 10)` sees the first `x` = 1
           (core-semantics.md #The Bindings Of One `let` Take Effect In Order + #Shadowing Is
           Well-Defined). The sequential companion of the case above at a repeated name.")
  (input  (let ((x 1) (x (+ x 10))) x))
  (output (: 11 Int64)))

(case "resolving a name in a shadowing environment returns the innermost binding's slot"
  (doc    "The compiler-internal SCOPE-RESOLUTION idiom behind lexical shadowing (the value-level cases
           above pin the observable; this pins how a name resolver realizes it). A name environment is a
           list of bound names in scope order (a self-hosted compiler holds parameters and `let`
           bindings this way, resolving a name reference to a local slot). When a name is bound twice —
           an inner `let` shadowing an outer binding of the same name — resolution must return the
           INNERMOST (latest, highest-slot) binding, not the first. `pos` searches the environment
           deepest-first and returns the last matching position: for env `[5, 7, 5]` (name 5 bound at
           slot 0, shadowed at slot 2), looking up 5 yields 2 — the shadowing binding — not 0. Pins that
           a recursive deepest-first environment search realizes lexical shadowing correctly (a
           first-match search would wrongly return the shadowed outer slot 0). An absent name yields -1.
           This is the `bytes → local-slot` name resolution a reader performs, the runtime dual of the
           `let`-shadowing value semantics above.")
  (input  (module m
            (type Env (ENil | ECons (Tuple Int64 Env)))
            (def (pos xs target k)
              (match xs
                ((Env.ENil _) (- 0 1))
                ((Env.ECons (tuple h t))
                  (let ((deeper (pos t target (+ k 1))))
                    (if (= deeper (- 0 1))
                        (if (= h target) k (- 0 1))
                        deeper)))))
            (def (main) (pos (Env.ECons (tuple 5 (Env.ECons (tuple 7 (Env.ECons (tuple 5 (Env.ENil ()))))))) 5 0))))
  (output (: 2 Int64)))

(case "a reference to an unbound name is rejected before running"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical: a reference to a name with no enclosing
           binding is refused. This is a front-end rejection every generation makes — scope resolution
           needs no static typing — so (error CDZ0101) is the recorded outcome.")
  (input  y)
  (error  CDZ0101))

(case "a sequencing block yields the value of its last form"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (2nd sentence:
           a block evaluates to its last form's value). The earlier forms are pure here, so the block's
           only observable result is the last form; ordering of effects is witnessed in
           03-equality-and-observation.sexp.")
  (input  (do 1 2 3))
  (output (: 3 Int64)))

(case "a sequencing block discards a pure compound intermediate"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (\"evaluate each
           of its forms\" then \"evaluate to the value of its last form\"): a non-final form is
           evaluated and its value discarded, whatever its type. A pure compound value — a record here —
           in a non-final position has no observable effect, so the block yields its last form (42). The
           earlier `do` cases only drop scalars; this pins that a COMPOUND intermediate is dropped the
           same way rather than blocking the block.")
  (needs  collections)
  (input  (do (record (a 1)) 42))
  (output (: 42 Int64)))

(case "a sequencing block discards a pure list intermediate"
  (doc    "Companion of the case above with a list intermediate: `(do (list 1 2 3) 7)` evaluates the
           list, discards it (no effect), and yields the last form 7.")
  (needs  collections)
  (input  (do (list 1 2 3) 7))
  (output (: 7 Int64)))

; --- A declaration in a sequencing block binds for the following forms -------------------
; core-semantics.md #A Declaration In A Sequencing Block Is Scoped To The Forms That Follow It:
; "A declaration form in a sequencing block MUST bind its name for the forms that follow it in
; that block, so that a name a declaration introduces is in scope without a separate binding
; form." This is how a module declaration binds its name (11-modules.sexp relies on it for
; `(do (module m …) <uses-m>)`), and it applies to a `def` declaration too — a `def` in a `do`
; binds its name for the later forms, no enclosing `let` needed. The seed does not yet recognize
; `def` as a declaration in do-block position: it treats the `def` head as a name to resolve and
; declines "unbound name: def" (a misleading code — `def` is a declaration keyword, not a name).

(case "a value declaration in a do block is in scope for the following forms"
  (doc    "Witnesses core-semantics.md #A Declaration In A Sequencing Block Is Scoped To The Forms
           That Follow It: `(def x 5)` as a form of a `do` binds `x` for the following form, so
           `(+ x 1)` sees it without a `let`. The block yields the last form's value, 6. This is the
           same declaration-binds-its-name rule a module declaration uses; a `def` declaration in a
           sequencing block is in scope exactly like one.")
  (input  (do (def x 5) (+ x 1)))
  (output (: 6 Int64)))

(case "a function declaration in a do block is callable by the following forms"
  (doc    "The function-declaration companion: `(def (f n) (+ n 1))` in a `do` binds `f` for the
           following forms, so `(f 9)` calls it and the block yields 10. A declaration introduces its
           name into the rest of the block without a separate binding form, whether it declares a
           value or a function.")
  (input  (do (def (f n) (+ n 1)) (f 9)))
  (output (: 10 Int64)))

; The two cases above declare ONE name and use it in a later form. The scoping rule is that a
; declaration binds its name for EVERY following form — including a LATER DECLARATION, so a chain of
; `def`s each sees the ones before it (core-semantics.md #A Declaration In A Sequencing Block Is Scoped
; To The Forms That Follow It). These pin the chain: a `def` whose value references an earlier `def`, a
; `def`-fn whose body calls an earlier sibling `def`, and a `def` that shadows an outer `let` binding —
; the declaration-scope behavior a prelude or a group of top-level helpers relies on.

(case "a later declaration in a do block sees an earlier one"
  (doc    "`(do (def x 5) (def y (+ x 1)) y)`: the second declaration's value `(+ x 1)` references `x`
           from the first declaration, so `y` = 6 and the block yields 6. Pins that a declaration is in
           scope for a LATER DECLARATION, not only for a plain expression form — the chaining that makes
           a sequence of `def`s (a prelude) resolve.")
  (input  (do (def x 5) (def y (+ x 1)) y))
  (output (: 6 Int64)))

(case "a function declaration in a do block calls an earlier sibling declaration"
  (doc    "`(do (def base 10) (def (add-base n) (+ n base)) (add-base 5))`: the function `add-base`
           closes over the earlier declaration `base`, so `(add-base 5)` = 15. Pins that a `def`-fn's
           body sees the declarations that precede it in the block, exactly as a module function sees
           its siblings.")
  (input  (do (def base 10) (def (add-base n) (+ n base)) (add-base 5)))
  (output (: 15 Int64)))

(case "a declaration in a do block shadows an outer binding"
  (doc    "`(let ((x 1)) (do (def x 99) x))`: the `def x 99` inside the `do` shadows the outer `let`
           binding of `x` for the forms that follow it, so the block yields 99. Pins that a do-block
           declaration follows the same lexical shadowing rules as any other binding (core-semantics.md
           #Shadowing Is Well-Defined), taking effect for references in its scope.")
  (input  (let ((x 1)) (do (def x 99) x)))
  (output (: 99 Int64)))

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

(case "a sequencing block whose last form is unit yields unit"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order together with
           #An Effect-Only Expression Yields The Unit Value: a `do` yields its last form's value, and
           when that is `unit` the block — and the program — yields the unit value. The earlier form is
           pure and dropped. This is the shape of every effect-only body: a sequence of effects ending
           in unit; it must run and yield unit as the normal-termination value.")
  (input  (do 1 unit))
  (output (: unit Unit)))

(case "a let body of unit yields unit"
  (doc    "Witnesses core-semantics.md #An Effect-Only Expression Yields The Unit Value: binding a
           value and then yielding `unit` produces the unit value as the program result. Unit is an
           ordinary value that a binding form can carry to the run boundary.")
  (input  (let ((x 1)) unit))
  (output (: unit Unit)))

(case "a conditional whose branches are unit yields unit"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch with a unit result: both
           branches yield the unit value, so the conditional yields unit whichever is taken. Pins that
           the unit value flows through `if` and crosses the run boundary as the program's result.")
  (input  (if true unit unit))
  (output (: unit Unit)))

(case "a conditional evaluates only the selected branch"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch. The unselected branch would
           trap on overflow if it were evaluated; the normal result proves it was not.")
  (input  (if true 1 (+ Int64.max 1)))
  (output (: 1 Int64)))

(case "a conditional selects the false branch when the condition is false"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch.")
  (input  (if false 1 2))
  (output (: 2 Int64)))

; The single-level case above shields a top-level unselected branch. The guarantee holds at DEPTH too:
; a trapping expression inside a NESTED unselected branch must not be evaluated either — and, dually, a
; conditional's CONDITION may itself be a conditional (an ordinary Bool-valued expression). These pin
; #Conditionals Evaluate One Branch where the single-level case cannot: the shielding is recursive, and
; the condition position accepts a computed Bool, not only a literal or a direct comparison.

(case "a conditional shields a trap in a nested unselected branch"
  (doc    "`(if true (if true 5 (/ 1 0)) 9)`: the outer `if` selects its then-branch, which is another
           `if` selecting 5; the innermost else `(/ 1 0)` (a division-by-zero trap) is in a branch that
           is never selected at either level, so it is NOT evaluated and the result is 5. Pins that
           #Conditionals Evaluate One Branch shields a trap NESTED two levels deep, not only a
           top-level unselected branch (the `(+ Int64.max 1)` case above).")
  (input  (if true (if true 5 (/ 1 0)) 9))
  (output (: 5 Int64)))

(case "a conditional's condition may itself be a conditional"
  (doc    "`(if (if true false true) 1 2)`: the condition is an `if` that evaluates to `false`, so the
           outer conditional selects its else-branch, yielding 2. Pins that the condition position
           accepts an arbitrary Bool-valued expression — here a nested `if` — not only a literal or a
           direct comparison (core-semantics.md #Conditionals Evaluate One Branch: a conditional selects
           by its condition, whatever Bool expression computes it).")
  (input  (if (if true false true) 1 2))
  (output (: 2 Int64)))

(case "a conditional whose condition folds to a constant still drops the untaken trapping branch"
  (doc    "`(if (< 1 2) 7 (% 5 0))`: the condition is a COMPARISON that a constant-folding compiler
           reduces to true at compile time, after which the conditional selects its then-branch (7) and
           the untaken else-branch `(% 5 0)` — a modulo-by-zero that would trap — is never evaluated,
           so the result is 7. Pins that folding a conditional whose CONDITION became a constant is
           short-circuit-preserving: it becomes the taken branch and DROPS the other, exactly as a
           run-time conditional shields an unselected branch (core-semantics.md #Conditionals Evaluate
           One Branch). This is the dual of the divisor-folds-to-zero case (06-numeric-model.sexp): there
           a fold must not ERASE a trap the source denotes; here a fold must not MANUFACTURE a trap the
           source shields. Distinct from the literal-`true` shielding case above in that the shielding
           holds only AFTER the condition itself folds — a fold that evaluated both branches, or kept
           the trapping one, would wrongly trap.")
  (input  (module m (def (main) (if (< 1 2) 7 (% 5 0)))))
  (output (: 7 Int64)))

(case "a conditional selects a branch by a runtime value that is not known at compile time"
  (doc    "`(def (f x) (if (< x 10) x (* x 2)))`: the condition `(< x 10)` depends on the runtime
           parameter `x`, so it CANNOT fold — the conditional must emit a real runtime branch that
           selects `x` (then) or `(* x 2)` (else) by the value computed at run time. `f(21)`: 21 is not
           < 10, so the else-branch yields 42. Pins the runtime conditional — a condition that is a
           genuine runtime value, not a literal or a fold — which a compiler lowers to a structured
           branch (push the condition, then a then/else region each leaving one value of the branches'
           shared type on the stack). Distinct from every conditional case above, whose condition is
           known at compile time (a literal, a nested `if`, or a foldable comparison): here the selection
           happens at run time. The companion `f(3)` (3 < 10) takes the then-branch and yields 3.")
  (input  (module m
            (def (f x) (if (< x 10) x (* x 2)))
            (def (main) (f 21))))
  (output (: 42 Int64)))

(case "a runtime conditional selects its then-branch when the runtime condition holds"
  (doc    "The then-branch companion to the runtime-conditional case above: with `x` = 3, `(< x 10)` is
           true at run time, so `(if (< x 10) x (* x 2))` selects `x` and yields 3. Together the pair
           pins that a runtime conditional selects EITHER branch by the run-time condition value (42 when
           false, 3 when true), so the structured branch is a genuine two-way selection, not a folded
           constant.")
  (input  (module m
            (def (f x) (if (< x 10) x (* x 2)))
            (def (main) (f 3))))
  (output (: 3 Int64)))

(case "a conjunction guards a let over a runtime value inside a conditional"
  (doc    "An INTEGRATION case: several control constructs composed in one function over a runtime
           parameter, the way a real program (not an isolated feature test) uses the language.
           `classify x = (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0)` composes: a
           short-circuit `and` of two comparisons as the condition (each operand a runtime `>`/`<`), a
           `let` binding a RUNTIME value `(* x x)` in the then-branch (so it must emit a real local, not
           a compile-time alias), the outer conditional selecting Int64 branches, and the arithmetic —
           all driven by the runtime argument. `classify 4`: `0 < 4` and `4 < 10` both hold, so
           `(let ((y (* 4 4))) (- y 1))` = 16 - 1 = 15. Pins that these constructs COMPOSE in one
           function — the short-circuit `and` (which desugars to a nested conditional), a runtime `let`,
           and the enclosing `if` nest correctly and thread their values — not merely that each works in
           isolation. The out-of-range companion below takes the else-branch.")
  (input  (module m
            (def (classify x) (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0))
            (def (main) (classify 4))))
  (output (: 15 Int64)))

(case "the guarded-let conditional takes its else-branch when the conjunction is false"
  (doc    "The else companion of the integration case above: `classify 20` — `20 < 10` is false, so the
           short-circuit `and` is false and the outer conditional selects its else-branch 0, never
           evaluating the `let`. Together the pair pins that the composed `and`/`let`/`if` selects by the
           runtime value in both directions (15 in range, 0 out of range), and that the short-circuit
           `and` shields the `let`-bearing then-branch when the guard fails.")
  (input  (module m
            (def (classify x) (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0))
            (def (main) (classify 20))))
  (output (: 0 Int64)))

; --- A conditional's branches must have the same type ------------------------------------
; core-semantics.md #Conditionals Evaluate One Branch, 2nd sentence: "Every branch of a
; conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated
; branch cannot carry a deferred error." So a conditional whose branches have DIFFERENT types
; is ill-typed even when the condition is a compile-time constant that never evaluates the
; mismatched branch — the compiler MUST reject it (CDZ0201). The rejection is the recorded
; outcome; the program does not run, so it has no branch value. A generation that does not yet
; type-check the unevaluated branch declines rather than emitting a component
; (reject-don't-miscompile).

(case "a conditional with an integer then-branch and a boolean else-branch is a type error"
  (doc    "The then-branch is Int64, the else-branch is Bool — different types. Even with a constant
           condition selecting the Int64 branch, the compiler MUST type-check BOTH branches and reject
           the mismatch (CDZ0201) rather than run the program.")
  (input  (if true 1 false))
  (error  CDZ0201))

(case "a conditional type error is caught even when the mismatched branch is the one taken"
  (doc    "The companion with the condition false, selecting the Bool branch: the branches still
           disagree in type (Int64 vs Bool), so the compiler MUST reject (CDZ0201). Pins that the
           check is on the pair of branch types, not on which branch would run.")
  (input  (if false 1 false))
  (error  CDZ0201))

(case "a conditional with integer and floating-point branches is a type error"
  (doc    "Int64 and Float64 are distinct numeric types that do not silently unify (numeric-model.md
           #Numeric Types Do Not Silently Promote). A conditional with an Int64 branch and a Float64
           branch is therefore ill-typed and the compiler MUST reject it (CDZ0201).")
  (input  (if true 1 3.5))
  (error  CDZ0201))

; --- A conditional's condition must be a Bool --------------------------------------------
; core-semantics.md #Conditionals Evaluate One Branch: a conditional selects a branch by its
; condition, which is a Bool. A condition of any other type is ill-typed — the compiler MUST
; reject it (CDZ0201). A COMPOUND condition (a tuple/record/list) must be rejected as a not-a-Bool
; type error with the constructor `tuple`/`record`/`list` intact — it is a recognized form (it
; builds a value everywhere else), so a diagnostic of "unbound name: tuple" would be a misleading
; code (CDZ0101) for what is plainly a not-a-Bool type error, the same wrong-diagnostic class as an
; out-of-range integer literal reported as an unbound name (01-literals.sexp).

(case "an integer if condition is a type error, not a running conditional"
  (doc    "1 is Int64, not Bool. A conditional's condition selects a branch and MUST be a Bool; an
           Int64 condition is ill-typed (CDZ0201). A C-like language treats a nonzero int as true —
           Cadenza does not silently coerce (numeric-model.md #Numeric Types Do Not Silently
           Promote); there is no truthiness. A generation that does not yet wire the CDZ0201 code
           declines rather than running the program (reject-don't-miscompile).")
  (input  (if 1 10 20))
  (error  CDZ0201))

(case "a compound if condition is a type error, not an unbound name"
  (doc    "A tuple is not a Bool, so `(if (tuple 1 2) …)` is ill-typed (CDZ0201). The constructor
           `tuple` is a recognized form — `(tuple 1 2)` builds a value in every other position — so
           reporting `unbound name: tuple` (CDZ0101) would mistake a not-a-Bool type error for a name
           resolution failure. The condition's type is what is wrong, not the spelling of a name.
           Pins that a compound condition is rejected as a type error with the constructor intact,
           the same misleading-diagnostic class as an out-of-range literal reported as unbound.")
  (input  (if (tuple 1 2) 10 20))
  (error  CDZ0201))

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

; --- A literal pattern's type must match the scrutinee's type ----------------------------
; A literal pattern matches the scrutinee by equality (above), and equality is only defined between
; values of the SAME type (core-semantics.md #Equality Is Structural; a cross-type comparison is a
; type error). So a literal pattern whose type differs from the scrutinee's — a `true` (Bool) pattern
; against an Int64 scrutinee, an integer pattern against a Bool scrutinee — can never meaningfully
; match: it is a static type mismatch between the arm and the scrutinee, a type error (CDZ0201), the
; same class as a tuple pattern of the wrong arity or a `(Some x)` pattern against an Int64. The
; compiler rejects the ill-typed arm; a generation that does not yet check the pattern's type against
; the scrutinee's declines rather than running the program (reject-don't-miscompile).

(case "a boolean literal pattern against an integer scrutinee is a type error"
  (doc    "The scrutinee `5` is Int64; the pattern `true` is Bool. A literal pattern matches by
           equality, which is only defined within one type, so a Bool pattern can never match an Int64
           value — the arm is ill-typed and the compiler MUST reject the match (CDZ0201). Pins that a
           literal pattern's type is checked against the scrutinee's, not silently failed to match.")
  (input  (match 5 (true 1) (_ 0)))
  (error  CDZ0201))

(case "an integer literal pattern against a boolean scrutinee is a type error"
  (doc    "The mirror: scrutinee `true` is Bool, pattern `5` is Int64 — a type mismatch, so the arm is
           ill-typed (CDZ0201). Pins the check in both directions — the scrutinee and every literal
           pattern must share a type.")
  (input  (match true (5 1) (_ 0)))
  (error  CDZ0201))

(case "matching on string literals"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns
           match by equality. The compiler uses this heavily to dispatch on instruction tags like
           'i64.const', 'i64.add', etc. — replacing nested if/= chains with readable match.")
  (input  (match "hello"
            ("hello" 1)
            ("world" 2)
            (else    0)))
  (output (: 1 Int64)))

(case "matching on a string produced by an expression"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns match by
           equality against the scrutinee's VALUE, whether the scrutinee is written as a bare literal
           (the case above) or produced by an expression. `(String.concat \"a\" \"b\")` evaluates to
           \"ab\", which the \"ab\" arm matches, yielding 100 — not the wildcard. (That the two strings
           are equal is independently witnessed: `(= (String.concat \"a\" \"b\") \"ab\")` is true. A
           bare and a let-bound \"ab\" scrutinee already select the arm; a string-valued expression
           must behave identically — the common compiler idiom of dispatching on a computed
           instruction name.)")
  (input  (match (String.concat "a" "b")
            ("ab"  100)
            (else  200)))
  (output (: 100 Int64)))

(case "matching on a sliced string selects the literal arm"
  (doc    "Companion using another string-producing operation: `(String.slice \"hello\" 0 2)` yields Some
           \"he\"; `expect` unwraps the in-bounds slice to \"he\", which the \"he\" arm matches, yielding
           100. A slice result is fallible (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping), so the program names the in-bounds expectation before matching the substring.")
  (needs  fallible-access)
  (input  (match (Option.expect (String.slice "hello" 0 2) "slice is in bounds")
            ("he"  100)
            (else  200)))
  (output (: 100 Int64)))

(case "matching falls through to else when no literal matches"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: when no literal pattern
           matches, the else (wildcard) catches it. Without else, a non-exhaustive match traps.")
  (input  (match 99
            (0 "zero")
            (1 "one")
            (else "other")))
  (output (: "other" String)))

; --- A match must cover every value of the scrutinee's type ------------------------------
; core-semantics.md #Matching Is Exhaustive Or Rejected: "A match whose patterns do not cover
; every value of the scrutinee's type MUST be a compile-time error." A Bool has exactly two
; values, true and false, so a match on a Bool that arms only ONE of them (and has no wildcard)
; is non-exhaustive and the compiler MUST reject it (CDZ0210) — even though the missing case would
; only be reached for one of the two inputs. The rejection is the recorded outcome; the program
; does not run. A generation that does not yet check runtime-bool exhaustiveness declines rather
; than emitting a component (reject-don't-miscompile).

(case "a bool match missing the false arm is non-exhaustive"
  (doc    "The scrutinee `b` is a Bool — its type has exactly two values. A match arming only `true`
           leaves `false` uncovered and has no wildcard, so it is non-exhaustive and the compiler MUST
           reject it (CDZ0210, coded-span-record.md). The rejection is the recorded outcome; the
           program does not run. Pins runtime-bool exhaustiveness against a match whose scrutinee is a
           function parameter, not a compile-time constant.")
  (input  (module m
            (def (f b) (match b (true 1)))
            (def (main) (f false))))
  (error  CDZ0210))

(case "a bool match missing the true arm is non-exhaustive"
  (doc    "The mirror of the case above: a match on a Bool arming only `false` leaves `true`
           uncovered and the compiler MUST reject it as non-exhaustive (CDZ0210). Pins that
           exhaustiveness is checked for BOTH bool values, not only the one the sole arm happens to
           name.")
  (input  (module m
            (def (f b) (match b (false 0)))
            (def (main) (f true))))
  (error  CDZ0210))

; A sum type's value set is its variant set, so exhaustiveness for a sum match is checked against
; ALL its variants — not just the scrutinee's runtime value. `Option` has variants Some and None;
; a match arming only `Some` leaves `None` uncovered, so it is non-exhaustive and the compiler MUST
; reject it (CDZ0210) EVEN when the scrutinee happens to be a `Some`. Exhaustiveness is a
; compile-time property of the arm set against the sum's variant set, not of which variant the
; scrutinee holds. The bool cases above are the two-value instance of the same rule; these are the
; general sum instance.

(case "a sum match missing a variant is non-exhaustive even when the scrutinee is the covered one"
  (doc    "`Option` has variants Some and None. `(match (Some 5) ((Some x) x))` arms only Some, leaving
           None uncovered and having no wildcard — non-exhaustive, so the compiler MUST reject it
           (CDZ0210), independent of the scrutinee being a Some. Exhaustiveness is a compile-time
           property of the arm set against the sum's variant set, not of which variant the scrutinee
           holds.")
  (input  (match (Some 5) ((Some x) x)))
  (error  CDZ0210))

(case "a Sign match missing two of three variants is non-exhaustive"
  (doc    "Sign has three variants (Neg | Zero | Pos). `(match (Sign.Pos unit) ((Sign.Pos _) 1))`
           arms only Pos, leaving Neg and Zero uncovered — non-exhaustive, so the compiler MUST reject
           it (CDZ0210). Pins that a sum's exhaustiveness covers every declared variant, not only the
           one the constant scrutinee names — a three-variant sum with a single arm is rejected just
           as a two-variant one is.")
  (input  (match (Sign.Pos unit) ((Sign.Pos _) 1)))
  (error  CDZ0210))

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

(case "a literal inside a constructor pattern matches a runtime payload"
  (doc    "core-semantics.md #Pattern Matching + #Matching Is Exhaustive Or Rejected: a literal nested
           inside a constructor pattern must be tested against the payload's RUNTIME value, exactly as
           a top-level literal pattern is. Here the payload `n` is a function parameter (not known at
           compile time); `(Some n)` with n=0 must match `(Some 0)` and yield 100, not fall through to
           the binding arm `(Some k)`. Companion to \"nested patterns with literals\" above, whose
           scrutinee `(Some 0)` is a compile-time constant — this one pins the same refinement when the
           payload is only known at run time.")
  (input  (module m
            (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k)))
            (def (main) (f 0))))
  (output (: 100 Int64)))

(case "a non-matching literal inside a constructor pattern binds the runtime payload"
  (doc    "The companion of the case above: with n=7 the literal arm `(Some 0)` does not match, so the
           binding arm `(Some k)` binds k=7 and yields 7. Confirms the nested literal is a genuine
           runtime test (matching for 0, falling through otherwise) rather than always-taken or
           always-skipped.")
  (input  (module m
            (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k)))
            (def (main) (f 7))))
  (output (: 7 Int64)))

(case "a literal inside a tuple pattern matches a runtime element"
  (doc    "core-semantics.md #Pattern Matching: the same refinement inside a tuple pattern. `(tuple n
           9)` with a runtime n; the arm `(tuple 0 y)` matches only when the first element is 0. With
           n=0 it matches and yields 100; the literal element is tested against the runtime value, not
           treated as a binder.")
  (input  (module m
            (def (f n) (match (tuple n 9) ((tuple 0 y) 100) ((tuple x y) x)))
            (def (main) (f 0))))
  (output (: 100 Int64)))

; --- A tuple pattern's arity must match the scrutinee's tuple arity ----------------------
; core-semantics.md #A Tuple Is Deconstructible By Pattern Matching (`(tuple a b)` binds the
; elements): a tuple pattern deconstructs a tuple of the SAME arity. A pattern `(tuple a b c)` has a
; three-element tuple shape, which can NEVER match a two-element tuple scrutinee — the pattern and
; scrutinee shapes are statically incompatible, a type error (CDZ0201), exactly as a `(Some x)`
; pattern against an Int64 scrutinee is. A wrong-arity tuple pattern is ill-typed, not a runtime
; non-match: the compiler rejects it, and a generation that does not yet check a tuple pattern's
; arity against the scrutinee's declines rather than running the program (reject-don't-miscompile).

(case "a tuple pattern of the wrong arity is a type error"
  (doc    "`(tuple a b c)` is a three-element tuple pattern; the scrutinee `(tuple 1 2)` is a
           two-tuple. A three-element pattern can never match a two-element tuple — their shapes are
           statically incompatible, so the arm is ill-typed and the compiler MUST reject the match
           (CDZ0201). Pins that a tuple pattern's arity is checked against the scrutinee's, not
           silently failed.")
  (input  (match (tuple 1 2) ((tuple a b c) a) (_ 0)))
  (error  CDZ0201))

(case "a one-element tuple pattern against a two-tuple is a type error"
  (doc    "The other direction: `(tuple a)` is a one-element tuple pattern, which cannot match the
           two-tuple `(tuple 1 2)` — a static shape mismatch, CDZ0201. Pins that BOTH too-many and
           too-few pattern elements are a type error, not a runtime non-match.")
  (input  (match (tuple 1 2) ((tuple a) a) (_ 0)))
  (error  CDZ0201))

; A pattern's KIND must also match the scrutinee's kind, not only a tuple's arity: a tuple pattern
; against a SUM scrutinee (or a sum/constructor pattern against a tuple) is a static shape mismatch.
; A `(tuple a b)` pattern deconstructs a tuple; a `Some`/`Ok`/`Sign.Pos` value is a sum, so the tuple
; pattern can never match it — CDZ0201, the same shape-mismatch class as a wrong-arity tuple pattern
; or a type-mismatched literal pattern above. (A literal pattern vs a sum/tuple scrutinee, and a
; constructor pattern vs a tuple/scalar scrutinee, are already rejected; this pins the tuple-pattern-
; vs-sum-scrutinee direction.)

(case "a tuple pattern against a sum scrutinee is a type error"
  (doc    "`(tuple a b)` is a tuple pattern; the scrutinee `(Some 5)` is a sum value. A tuple pattern
           deconstructs a tuple, so it can never match a sum — the arm's shape is statically
           incompatible with the scrutinee, a type error (CDZ0201). Pins the pattern-KIND check
           (tuple vs sum), the companion of the tuple-ARITY check above.")
  (input  (match (Some 5) ((tuple a b) a) (_ 0)))
  (error  CDZ0201))

(case "a tuple pattern against a Sign scrutinee is a type error"
  (doc    "The companion with a user-facing sum: `(Sign.Pos unit)` is a sum value, so a `(tuple a b)`
           pattern against it is a shape mismatch (CDZ0201). Pins that the tuple-pattern-vs-sum check
           holds for every sum, not only Option.")
  (input  (match (Sign.Pos unit) ((tuple a b) a) (_ 0)))
  (error  CDZ0201))

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

; --- Matching a RUNTIME scrutinee ---------------------------------------------------
; Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected for scrutinees whose
; value is NOT known at compile time — a function parameter or a computed expression. The
; matching arm must be selected from the scrutinee's RUNTIME value, exactly as when the
; scrutinee is an inline literal (cases above). These are core (functions + match are core):
; the compiler that dispatches instruction opcodes matches on runtime-computed byte values.

(case "an integer literal pattern matches a runtime scrutinee"
  (doc    "The scrutinee `n` is a function parameter — its value (0) is not known until run
           time. The first arm's literal pattern 0 must match the runtime value 0 and select
           its body, exactly as it would for an inline literal scrutinee. This is the base-case
           dispatch every recursive function over integers relies on.")
  (input  (module m
            (def (classify n) (match n (0 100) (1 200) (else 900)))
            (def (main) (classify 0))))
  (output (: 100 Int64)))

(case "a runtime scrutinee selects a non-first literal arm"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: arms are tried top-to-bottom
           and the first whose pattern matches the runtime value wins. Here the runtime value 2
           skips the 0 and 1 arms and selects the 2 arm — not the else.")
  (input  (module m
            (def (classify n) (match n (0 10) (1 20) (2 30) (else 99)))
            (def (main) (classify 2))))
  (output (: 30 Int64)))

(case "a negative integer literal pattern matches a runtime scrutinee"
  (doc    "A negative literal pattern matches by equality against the runtime value, like any
           other integer literal.")
  (input  (module m
            (def (classify n) (match n (-1 100) (else 200)))
            (def (main) (classify -1))))
  (output (: 100 Int64)))

(case "an earlier literal arm is chosen over a later name-binding arm for a runtime scrutinee"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected + #Bindings Introduced By A
           Pattern Are Scoped To Its Branch: a bare name pattern `k` matches anything and binds
           the whole scrutinee, but only if reached. With the runtime value 0, the earlier
           literal arm `0` matches first, so the name arm is never entered.")
  (input  (module m
            (def (f n) (match n (0 100) (k (+ k 1))))
            (def (main) (f 0))))
  (output (: 100 Int64)))

(case "a name pattern binds the runtime scrutinee when no literal arm matches"
  (doc    "The companion to the case above: with the runtime value 41 no literal arm matches,
           so the name arm `k` binds k=41 and its body computes 42. Confirms the name arm and
           the literal arm are selected consistently from the same runtime value.")
  (input  (module m
            (def (f n) (match n (0 100) (k (+ k 1))))
            (def (main) (f 41))))
  (output (: 42 Int64)))

(case "a match on a computed runtime value dispatches on the result"
  (doc    "The scrutinee is the expression `(% n 2)`, computed at run time. Its value (0 for an
           even n) selects the literal arm 0. Exercises a match whose scrutinee is neither a
           literal nor a variable but an arbitrary runtime expression — the parity dispatch a
           LEB128 encoder performs.")
  (input  (module m
            (def (parity n) (match (% n 2) (0 0) (_ 1)))
            (def (main) (parity 4))))
  (output (: 0 Int64)))

(case "a match on a record-field-access scrutinee dispatches on the field value"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected + #Member Access Projects A Record
           Field: the match scrutinee is `(. r n)`, a member access whose value is 5. The literal arm
           5 must match that value and yield 100 — the scrutinee's value is what is matched, whether it
           is written as a literal, a variable, an arithmetic expression, or a field projection.
           (Binding the field to a name first and matching that already works; matching the projection
           directly must behave identically.)")
  (input  (let ((r (record (n 5))))
            (match (. r n)
              (5 100)
              (_ 200))))
  (output (: 100 Int64)))

(case "a match on a tuple-element-access scrutinee dispatches on the element value"
  (doc    "The tuple companion of the case above: the scrutinee `(tuple.0 t)` projects element 0 (value
           5), which the literal arm 5 must match, yielding 100. A positional access is a scrutinee
           value like any other.")
  (input  (let ((t (tuple 5 9)))
            (match (tuple.0 t)
              (5 100)
              (_ 200))))
  (output (: 100 Int64)))

(case "a match on a record field selects a later literal arm"
  (doc    "Confirms the field-access scrutinee is matched against EACH literal arm, not just skipped to
           the wildcard: with r.n = 6, the 5 arm is passed over and the 6 arm selected, yielding 300.")
  (input  (let ((r (record (n 6))))
            (match (. r n)
              (5 100)
              (6 300)
              (_ 200))))
  (output (: 300 Int64)))

(case "a nested match on a runtime scrutinee"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: a match body may itself be a
           match on the same runtime scrutinee. Both selections are driven by the runtime value
           0, so the inner match's 0 arm is chosen and the result is 7.")
  (input  (module m
            (def (f n) (match n (0 (match n (0 7) (_ 8))) (_ 9)))
            (def (main) (f 0))))
  (output (: 7 Int64)))

; The case above nests a match in a match ARM (both on the same scrutinee). A match may also take
; another match's RESULT as its SCRUTINEE — `(match (match …) …)` — the outer match dispatching on the
; value the inner match produced. This is the compiler idiom of dispatching on a sub-dispatch's result
; (classify, then act on the classification). The inner match's selected value crosses into the outer as
; an ordinary scrutinee value; core-semantics.md #Matching Is Exhaustive Or Rejected applies at each
; level. Distinct from the same-scrutinee nesting above: here the inner match is EVALUATED and its value
; consumed, not a body reached after the outer already matched.

(case "a match takes another match's result as its scrutinee"
  (doc    "The scrutinee of the outer match is itself a match: `(match 1 (1 (Some 7)) (_ (None unit)))`
           evaluates to `(Some 7)`, which the outer match deconstructs, binding x=7. Pins that a match's
           scrutinee may be a match RESULT — the sub-dispatch is evaluated and its value consumed as an
           ordinary scrutinee, the compiler idiom of dispatching on a classification.")
  (input  (match (match 1 (1 (Some 7)) (_ (None unit)))
            ((Some x) x)
            ((None _) 0)))
  (output (: 7 Int64)))

(case "a wildcard in a nested pattern position ignores that element"
  (doc    "core-semantics.md #Pattern Matching: a `_` wildcard may appear at a NESTED position, matching
           anything there without binding. `(Some (tuple _ b))` matches a Some whose payload is a pair,
           ignoring the first element and binding `b` to the second — here 2. Pins that the wildcard is
           positional inside a compound pattern, not only a top-level catch-all arm.")
  (input  (match (Some (tuple 1 2))
            ((Some (tuple _ b)) b)
            ((None _)           0)))
  (output (: 2 Int64)))

(case "a runtime scrutinee matching no arm traps"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: a match on an Int64 arming only 1
           and 2, with no wildcard/else, cannot be proven to cover every Int64 value, so it is
           non-exhaustive and the compiler MUST reject it at compile time (CDZ0210) rather than emit a
           component that could trap at run time. The rejection is the recorded outcome; the program
           does not run.")
  (input  (module m
            (def (f n) (match n (1 10) (2 20)))
            (def (main) (f 3))))
  (error  CDZ0210))

(case "a boolean literal pattern matches a runtime scrutinee"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected over the two Bool values, with
           the scrutinee a runtime function parameter. `not` is a total match on true/false —
           exhaustive, so no else is needed and no generation rejects it.")
  (input  (module m
            (def (negate b) (match b (true false) (false true)))
            (def (main) (negate true))))
  (output (: false Bool)))

(case "a match on a runtime integer scrutinee producing a boolean"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: the scrutinee is a runtime integer
           but the arm bodies are Bool — a match is an expression of whatever type its arms yield,
           not restricted to the scrutinee's type. `is-zero` maps 0 → true, else → false; is-zero(0)
           = true. The Bool result must cross the run boundary as the program's value (compare the
           Bool-returning function cases in 09-functions.sexp — same result-kind requirement, reached
           through a match rather than a call).")
  (input  (module m
            (def (is-zero n) (match n (0 true) (_ false)))
            (def (main) (is-zero 0))))
  (output (: true Bool)))

; --- Boolean connectives (short-circuit) -------------------------------------------------
; core-semantics.md #Boolean Connectives Short-Circuit: the language offers conjunction, disjunction,
; and negation over Bool. Conjunction evaluates its right operand ONLY when the left is true;
; disjunction ONLY when the left is false — so a connective shields a trapping or effectful right
; operand exactly as an unselected conditional branch does (#Conditionals Evaluate One Branch). Each
; operand is type-checked as a Bool whether or not it is evaluated. Tagged (needs boolean-connectives):
; the seed does not yet realize `and`/`or`/`not`, so it SKIPS these until a generation adds them; they
; desugar to short-circuit conditionals (`(and a b)` = `(if a b false)`, `(or a b)` = `(if a true b)`,
; `(not a)` = `(if a false true)`), which the seed already lowers.

(case "conjunction is true exactly when both operands are true"
  (doc    "The `and` value table over the four Bool pairs, folded to one witness: only true∧true is
           true (core-semantics.md #Boolean Connectives Short-Circuit).")
  (needs  boolean-connectives)
  (input  (module m
            (def (row a b) (if (and a b) 1 0))
            (def (main) (+ (+ (row true true) (row true false)) (+ (row false true) (row false false))))))
  (output (: 1 Int64)))

(case "disjunction is false exactly when both operands are false"
  (doc    "The `or` value table: only false∨false is false, so three of the four pairs are true
           (core-semantics.md #Boolean Connectives Short-Circuit).")
  (needs  boolean-connectives)
  (input  (module m
            (def (row a b) (if (or a b) 1 0))
            (def (main) (+ (+ (row true true) (row true false)) (+ (row false true) (row false false))))))
  (output (: 3 Int64)))

(case "negation inverts a boolean"
  (doc    "`(not true)` is false and `(not false)` is true (core-semantics.md #Boolean Connectives
           Short-Circuit).")
  (needs  boolean-connectives)
  (input  (module m (def (main) (if (not false) (not true) true))))
  (output (: false Bool)))

(case "conjunction shields a trapping right operand when the left is false"
  (doc    "`(and false (< (/ 1 0) 2))`: `and` evaluates its right operand ONLY when the left is true,
           so with the left false the division-by-zero trap in the right operand is NOT evaluated and
           the result is false — the connective shields the trap exactly as an unselected conditional
           branch does (core-semantics.md #Boolean Connectives Short-Circuit). Without short-circuit
           this would trap.")
  (needs  boolean-connectives)
  (input  (and false (< (/ 1 0) 2)))
  (output (: false Bool)))

(case "disjunction shields a trapping right operand when the left is true"
  (doc    "`(or true (< (/ 1 0) 2))`: `or` evaluates its right operand ONLY when the left is false, so
           with the left true the trap in the right operand is NOT evaluated and the result is true.
           The dual of the `and` shielding case (core-semantics.md #Boolean Connectives Short-Circuit).")
  (needs  boolean-connectives)
  (input  (or true (< (/ 1 0) 2)))
  (output (: true Bool)))

(case "a boolean connective with a non-boolean operand is a type error"
  (doc    "`(and true 1)` gives an Int64 where a Bool operand is required. core-semantics.md #Boolean
           Connectives Short-Circuit: each operand is type-checked as a Bool whether or not it is
           evaluated, so the compiler MUST reject the non-Bool operand (CDZ0201) rather than run — the
           same discipline as a conditional's branch type-check, applied to a connective's operand.")
  (needs  boolean-connectives)
  (input  (and true 1))
  (error  CDZ0201))
