; Functions and closures — witnesses core-semantics.md §Functions. Functions are
; first-class values (fn), applied by (fn-expr arg), capturing their enclosing
; scope. Functions are SINGLE-ARITY: each function takes exactly one argument.
; Multi-parameter syntax (fn (x y) body) is sugar for currying: (fn x (fn y body)).
; Application (f a b) is sugar for ((f a) b). These are CORE cases (no (needs …)):
; the seed realizes them, because a compiler authored in Cadenza is built from
; functions and closures. Results are (: <value> <Type>); unbounded recursion halts
; as (exhausted).

(case "a function applied to an argument"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value and §Applying A Function
           Binds Its Parameters To Its Arguments: an inline fn is applied to 5, binding x to 5.")
  (input  ((fn (x) (+ x 1)) 5))
  (output (: 6 Int64)))

(case "a function bound to a name and then applied"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value: a fn is an ordinary value
           bindable by let, then applied by naming it in head position.")
  (input  (let ((inc (fn (x) (+ x 1))))
            (inc 10)))
  (output (: 11 Int64)))

(case "a closure captures the binding in scope where it was created"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value (2nd sentence): the fn
           captures y=3 from its creation scope; applying it later observes the captured y even though
           the application site has its own y=100.")
  (input  (let ((add-y (let ((y 3)) (fn (x) (+ x y)))))
            (let ((y 100))
              (add-y 4))))
  (output (: 7 Int64)))

(case "a function is passed as an argument (higher-order)"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value: apply-twice takes a function
           f and a value v and applies f to the result of applying f to v.")
  (input  (let ((apply-twice (fn (f v) (f (f v)))))
            (apply-twice (fn (x) (+ x 3)) 1)))
  (output (: 7 Int64)))

; The higher-order function above is LET-BOUND (a lambda); a NAMED-def higher-order function must be
; able to receive a function argument just the same — core-semantics.md §A Function Is A First-Class
; Value places no restriction on how the receiving function is bound. The seed resolves a lambda
; argument to a let-bound HOF (compile-time beta reduction, above) but NOT to a NAMED-def HOF: `(def
; (ap g v) (g v))` applied to a lambda declines "bare lambda in scalar position". The same inlining
; that handles the let-bound HOF must apply when the HOF is a top-level def.

(case "a named higher-order function receives a lambda argument"
  (doc    "`ap` is a named-def higher-order function taking a function `g` and a value `v`, applying g
           to v; `(ap (fn (x) (* x 2)) 7)` = 14. A named HOF must accept a function argument exactly as
           the let-bound `apply-twice` above does — the difference is only whether the HOF is named or
           let-bound. The seed declines the named case (\"bare lambda in scalar position\"): it inlines
           a lambda argument into a let-bound HOF but not into a named-def HOF.")
  (input  (module m
            (def (ap g v) (g v))
            (def (main) (ap (fn (x) (* x 2)) 7))))
  (output (: 14 Int64)))

(case "a function is returned as a result"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value: adder returns a closure over
           its parameter n; the returned function is then applied.")
  (input  (let ((adder (fn (n) (fn (x) (+ x n)))))
            ((adder 10) 5)))
  (output (: 15 Int64)))

; core-semantics.md §A Function Is A First-Class Value: a function can be "stored in a data structure."
; A tuple and a list are data structures exactly as a record is, so a function stored in a tuple
; element (or list element) must be extractable and callable, exactly as one stored in a record field
; is. The compiler resolves a function through record member access `.` (the control below runs); the
; same projection-to-lambda resolution must extend to the positional/indexed accessors `tuple.N` and
; `List.at`. A generation that does not yet resolve a stored lambda through those accessors declines
; rather than running the program (reject-don't-miscompile).

(case "a function stored in a tuple element is called after extraction"
  (doc    "A function is a first-class value storable in any data structure. `(tuple (fn (x) (+ x 1))
           9)` stores a function as element 0; `(tuple.0 …)` extracts it and applying it to 5 yields 6.
           This must behave exactly as the record-field companion below — a tuple is a data structure
           like a record. A generation that does not yet resolve the stored lambda through `tuple.N`
           the way it does through `.` declines rather than running the program.")
  (input  ((tuple.0 (tuple (fn (x) (+ x 1)) 9)) 5))
  (output (: 6 Int64)))

(case "a function stored in a record field is called after extraction"
  (doc    "The control the case above must match: `(record (f (fn (x) (+ x 1))))` stores a function in
           field `f`; `(. … f)` extracts it and applying it to 5 yields 6. The seed runs this — a
           function stored in a record is resolved and called. The tuple case must behave identically.")
  (needs   collections)
  (input   ((. (record (f (fn (x) (+ x 1)))) f) 5))
  (output  (: 6 Int64)))

(case "a field is projected from a record returned by a function"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value + #Member Access Projects A
           Record Field: a function may return a record, and its caller projects a field from the
           result. `((fn (x) (record (v x))) 7)` builds the record {v: 7}; projecting `v` yields 7.
           Accessing a field inside the lambda body already works, and accessing a directly-written or
           let-bound record works — projecting the record a lambda RETURNS must behave the same, not
           trap. This is the record-builder idiom a compiler uses constantly.")
  (needs   collections)
  (input   (. ((fn (x) (record (v x))) 7) v))
  (output  (: 7 Int64)))

(case "an element is projected from a tuple returned by a function"
  (doc    "The tuple companion: `((fn (x) (tuple x 9)) 7)` returns the pair (7, 9); projecting element 0
           yields 7. A positional access on a function's tuple result must project it, not trap.")
  (input   (tuple.0 ((fn (x) (tuple x 9)) 7)))
  (output  (: 7 Int64)))

(case "a field is projected from a record returned by a let-bound function"
  (doc    "The same record-builder reached through a named binding: `mk` is a lambda returning a
           record; `(mk 7)` builds {v: 7} and `(. (mk 7) v)` projects 7. Binding the builder to a name
           does not change that its result is an accessible record.")
  (needs   collections)
  (input   (let ((mk (fn (x) (record (v x)))))
             (. (mk 7) v)))
  (output  (: 7 Int64)))

; A NULLARY function that returns a compound value must be projectable exactly as a unary one is.
; The cases above return a structure from a function of one parameter; a nullary function `(def (mk)
; <compound>)` called as `(mk)` returns the same kind of value, and projecting a field/element from
; it must yield the value, not trap. The seed projects a UNARY function's structure result correctly
; (above) but TRAPS on a NULLARY function's structure result — a nullary call `(mk)` is not reduced
; to its body for projection the way a unary call `(mk arg)` is, so the access finds no compile-time
; structure and traps at run time. (A nullary function returning a SCALAR works — `(mk)` → 42; only
; a projected compound result traps.)

(case "an element is projected from a tuple returned by a nullary function"
  (doc    "`mk` is a nullary function returning the pair (7, 9); `(mk)` calls it and `(tuple.1 (mk))`
           projects element 1, yielding 9. A positional access on a nullary function's tuple result
           must project it, exactly as it does for a unary function's result (above) — not trap. The
           seed traps: it does not reduce the nullary call `(mk)` to its tuple body for the access.")
  (input   (module m
             (def (mk) (tuple 7 9))
             (def (main) (tuple.1 (mk)))))
  (output  (: 9 Int64)))

(case "a field is projected from a record returned by a nullary function"
  (doc    "The record companion: `mk` is a nullary function returning {a: 5}; `(. (mk) a)` projects
           the field, yielding 5. Projecting a field of a nullary function's record result must behave
           like projecting a unary function's record result (above), not trap. The seed traps on the
           nullary case.")
  (needs   collections)
  (input   (module m
             (def (mk) (record (a 5)))
             (def (main) (. (mk) a))))
  (output  (: 5 Int64)))

(case "applying a non-function is a type error"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameter To Its Argument:
           applying a value that is not a function has no defined result. The callee's type is not a
           function type, so the compiler MUST reject it at compile time (CDZ0201) rather than emit a
           component. With curried functions, partial application is natural (returns a closure), so
           the error case is applying a non-function like an integer.")
  (input  (5 3))
  (error  CDZ0201))

(case "applying a boolean is a type error"
  (doc    "Companion of the case above for another non-function scalar: a Bool is not a function, so
           applying it (`(true 1)`) is a type error the compiler MUST reject (CDZ0201).")
  (input  (true 1))
  (error  CDZ0201))

(case "applying a float is a type error"
  (doc    "Companion for a Float callee: `(3.5 1)` applies a non-function, a type error the compiler
           MUST reject (CDZ0201).")
  (input  (3.5 1))
  (error  CDZ0201))

; --- Over-applying a single-arity constructor is applying a non-function -----------------
; core-semantics.md #A Sum Type Constructor Is A Single-Arity Function (applied to EXACTLY ONE
; argument) together with #Functions Are Single-Arity (`(f a b)` desugars to `((f a) b)`): a
; constructor takes one argument, so `(Some 1 2)` desugars to `((Some 1) 2)` — applying the Sum
; value `(Some 1)`, which is NOT a function, to `2`. That is the apply-a-non-function error above,
; so the compiler MUST reject it (CDZ0201), exactly as `((Some 1) 2)` written explicitly is rejected.
; An over-applied constructor is arity-checked the same way an over-applied user function is (`(f 5
; 99)` on a unary `f`), so the ill-formed application never slips through with a wrong (truncated)
; value; a generation that does not yet check it declines rather than running the program.

(case "over-applying a constructor is a type error, not a silent argument drop"
  (doc    "`(Some 1 2)` desugars to `((Some 1) 2)`: the constructor `Some` is single-arity, so
           `(Some 1)` is a complete Sum value, and applying it to `2` applies a non-function — a type
           error (CDZ0201), the same as `(5 3)` above. The compiler MUST reject it rather than drop
           the `2` and yield `(Some 1)`, which would silently accept the ill-formed application.")
  (input  (Some 1 2))
  (error  CDZ0201))

(case "over-applying a constructor by several arguments is a type error"
  (doc    "The same shape with more extra arguments: `(Some 1 2 3)` desugars to `(((Some 1) 2) 3)`,
           applying the Sum value `(Some 1)` to `2` (already a non-function application). The compiler
           MUST reject it (CDZ0201). Pins that the arity check is on the constructor's single-argument
           application, not forgiving of any number of trailing arguments.")
  (input  (Some 1 2 3))
  (error  CDZ0201))

(case "a recursive def computes over its argument"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments and
           §Recursion Is Accountable Against The Resource Measure: sum-to counts down to 0, bounded by
           the resource measure. sum-to(3) = 3 + 2 + 1 + 0 = 6.")
  (input  (module m
            (def (sum-to n)
              (if (= n 0) 0 (+ n (sum-to (+ n -1)))))
            (def (main) (sum-to 3))))
  (output (: 6 Int64)))

(case "a recursive def with a match base case computes over its argument"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments
           and §Recursion Is Accountable Against The Resource Measure, with the base case expressed
           as a `match` on the argument rather than an `if`. This is the canonical functional idiom:
           sum-to(n) matches 0 → 0, else n + sum-to(n-1). The base-case arm must be selected from
           the RUNTIME value of n; sum-to(3) = 3 + 2 + 1 + 0 = 6. Companion to the if-based
           `sum-to` above — both must agree.")
  (input  (module m
            (def (sum-to n)
              (match n
                (0 0)
                (_ (+ n (sum-to (- n 1))))))
            (def (main) (sum-to 3))))
  (output (: 6 Int64)))

(case "recursive factorial with a match base case"
  (doc    "core-semantics.md §Recursion: factorial via a match on the argument. The 0 arm is the
           base case, reached only from the runtime value hitting 0 after counting down; without
           selecting the 0 arm at run time the recursion would never terminate. fact(5) = 120.")
  (input  (module m
            (def (fact n)
              (match n
                (0 1)
                (_ (* n (fact (- n 1))))))
            (def (main) (fact 5))))
  (output (: 120 Int64)))

(case "recursive fibonacci with literal match base cases"
  (doc    "core-semantics.md §Recursion: two literal base-case arms (0 and 1) matched against the
           runtime argument, and a recursive arm summing the two predecessors. fib(10) = 55.
           Exercises multiple literal arms dispatching on a runtime scrutinee within a recursion.")
  (input  (module m
            (def (fib n)
              (match n
                (0 0)
                (1 1)
                (_ (+ (fib (- n 1)) (fib (- n 2))))))
            (def (main) (fib 10))))
  (output (: 55 Int64)))

(case "unbounded recursion halts by exhausting the resource measure"
  (doc    "Witnesses core-semantics.md §Recursion Is Accountable Against The Resource Measure: a
           function that applies itself with no base case consumes the deterministic resource measure
           and halts at a defined point rather than running forever.")
  (input  (module m
            (def (spin n) (spin (+ n 1)))
            (def (main) (spin 0))))
  (exhausted))

(case "functions are single-arity and curried"
  (doc    "Witnesses core-semantics.md §Functions Are Single-Arity: a function takes exactly one
           argument. Multi-parameter syntax (fn (x y) body) desugars to (fn x (fn y body)). Partial
           application is natural: applying a two-param function to one argument returns a closure.")
  (input  (let ((add (fn (x y) (+ x y))))
            (let ((add3 (add 3)))
              (add3 7))))
  (output (: 10 Int64)))

(case "multi-argument application is curried application"
  (doc    "Witnesses core-semantics.md §Functions Are Single-Arity: application (f a b) desugars
           to ((f a) b). Each application passes one argument; the result of (f a) is a closure
           that accepts b.")
  (input  ((fn (x y) (+ x y)) 2 3))
  (output (: 5 Int64)))

(case "a curried function can be partially applied"
  (doc    "Witnesses core-semantics.md §Functions Are Single-Arity: since functions are single-arity
           and multi-param is sugar for currying, partial application works naturally. map-inc applies
           inc to each element — inc is (add 1), a partial application of add.")
  (input  (let ((add (fn (x y) (+ x y))))
            (let ((inc (add 1)))
              (inc 41))))
  (output (: 42 Int64)))

(case "a named multi-argument function applies in explicit curried form"
  (doc    "Witnesses core-semantics.md §Functions Are Single-Arity (\"Multi-argument application (f a b)
           MUST desugar to curried application ((f a) b)\"): `(add 3 4)` and `((add 3) 4)` are the SAME
           program by that desugaring, so both must yield 7. This pins the rule for a NAMED def (the
           cases above use lambda values); a def is single-arity and curried just like a lambda, so
           `(add 3)` is a closure `((add 3) 4)` then applies.")
  (input  (module m
            (def (add x y) (+ x y))
            (def (main) ((add 3) 4))))
  (output (: 7 Int64)))

(case "a named function is partially applied, bound, and used"
  (doc    "core-semantics.md §Functions Are Single-Arity: partial application is natural for a named
           def too — `(add 3)` returns a closure awaiting the second argument, bound to `inc` and then
           applied to 4, yielding 7. The lambda form of this already holds; a named def must behave
           identically since multi-param defs desugar to curried single-arity functions.")
  (input  (module m
            (def (add x y) (+ x y))
            (def (main) (let ((inc (add 3))) (inc 4)))))
  (output (: 7 Int64)))

; --- A function's result type is not restricted to Int64 --------------------------------
; core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments: a function's
; result is whatever value its body evaluates to. A predicate returns Bool; nothing in the
; semantics restricts a `def`'s return type to integers. These call a non-Int64-returning
; function and observe its result AS the program's result — the value must cross the run
; boundary faithfully (a Bool run returns a Bool). The point is well-formed programs: each
; must produce its recorded value, never an unrunnable artifact. (Contrast the cases above
; where a Bool result is consumed internally by `if`/`=`; here it is the program's result.)

(case "a function returning a boolean predicate result"
  (doc    "`is-zero` is an ordinary predicate: it returns the Bool `(= n 0)`. Calling it from
           `main` yields that Bool as the program's result. is-zero(0) = true.")
  (input  (module m
            (def (is-zero n) (= n 0))
            (def (main) (is-zero 0))))
  (output (: true Bool)))

(case "a boolean-returning function called with a false result"
  (doc    "The companion to the case above: is-zero(5) = false. Confirms the Bool result is carried
           faithfully across the run boundary for both truth values, not coerced or truncated.")
  (input  (module m
            (def (is-zero n) (= n 0))
            (def (main) (is-zero 5))))
  (output (: false Bool)))

(case "a comparison-predicate function returns its boolean result"
  (doc    "core-semantics.md §Ordering Where Offered Is Total, as a function result: `lt5` returns
           `(< n 5)`. lt5(3) = true. A comparison predicate is the most common Bool-returning
           helper a compiler writes (bounds checks, dispatch guards).")
  (input  (module m
            (def (lt5 n) (< n 5))
            (def (main) (lt5 3))))
  (output (: true Bool)))

(case "a boolean result threaded through a second function"
  (doc    "core-semantics.md §A Function Is A First-Class Value: `b` forwards `a`'s Bool result, and
           `main` returns `b`'s. The Bool return type propagates through the call chain; b(1) = false.")
  (input  (module m
            (def (a n) (= n 0))
            (def (b n) (a n))
            (def (main) (b 1))))
  (output (: false Bool)))

(case "a boolean function result bound by let is still a boolean"
  (doc    "core-semantics.md §Binding Is Lexical: binding a predicate's result to a name and
           returning that name does not change its type. The program's result is the Bool true.")
  (input  (module m
            (def (is-zero n) (= n 0))
            (def (main) (let ((r (is-zero 0))) r))))
  (output (: true Bool)))

; --- A function's PARAMETER type is not restricted to Int64 -----------------------------
; core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments: a parameter is
; bound to whatever argument value it is applied to — a Bool or a Float just as well as an
; Int64. Nothing in the semantics restricts a `def`'s parameter to integers. These pass a
; non-Int64 argument to a user function and observe the ordinary result. (Companion to the
; result-type cases above; together they say a function is polymorphic in neither direction
; artificially — the seed must handle a Bool/Float on both sides of a call.)

(case "a function takes a boolean parameter and branches on it"
  (doc    "`f` takes a Bool `b` and returns 10 or 20 via `if`. Applying it to `true` binds b=true,
           selecting the then-branch: f(true) = 10. The parameter is a Bool, not an Int64.")
  (input  (module m
            (def (f b) (if b 10 20))
            (def (main) (f true))))
  (output (: 10 Int64)))

(case "a boolean-parameter function applied to false"
  (doc    "The companion of the case above: f(false) = 20. Confirms both Bool argument values are
           bound and dispatched correctly through a call.")
  (input  (module m
            (def (f b) (if b 10 20))
            (def (main) (f false))))
  (output (: 20 Int64)))

(case "a boolean parameter forwarded to a conditional result"
  (doc    "core-semantics.md §A Function Is A First-Class Value: `both` takes two Bools and returns
           `b` when `a` is true, else false — a logical AND. both(true, true) = true. Exercises two
           Bool parameters in one signature, curried.")
  (input  (module m
            (def (both a b) (if a b false))
            (def (main) (both true true))))
  (output (: true Bool)))

; --- A parameter whose type the body does not constrain is polymorphic -------------------
; The cases above pin a parameter's type via a use in the body (`(if b …)` forces Bool). The
; identity function `(def (id x) x)` uses `x` only by returning it, so nothing in the body
; constrains its type: `id` is polymorphic (∀a. a → a) and applies to a value of ANY type,
; returning it unchanged (core-semantics.md §Applying A Function Binds Its Parameters To Its
; Arguments — the parameter is bound to whatever argument it is applied to; type-system.md
; §Inference — an unconstrained parameter generalizes to a type variable rather than defaulting
; to Int64). The seed monomorphizes such a parameter to Int64, so `(id true)` / `(id 3.5)`
; decline "argument kind mismatch" — only `(id 42)` is accepted. These pin the polymorphic case;
; the Int64 companion is the control that already passes.

(case "the identity function applied to an integer returns the integer"
  (doc    "The control: `(def (id x) x)` applied to an Int64 returns it. id(42) = 42. The body does
           not constrain `x`'s type; applying to an integer determines it here.")
  (input  (module m
            (def (id x) x)
            (def (main) (id 42))))
  (output (: 42 Int64)))

(case "the identity function applied to a boolean returns the boolean"
  (doc    "The polymorphic case: the same `(def (id x) x)` applied to a Bool returns the Bool.
           id(true) = true. Nothing in `id`'s body restricts `x` to Int64 — it is returned
           unchanged — so `id` is polymorphic and accepts a Bool argument exactly as it accepts an
           Int64. The seed defaults the unconstrained parameter to Int64 and declines a Bool argument
           (\"argument kind mismatch\"); a full inference generalizes `x` to a type variable so both
           applications type-check.")
  (input  (module m
            (def (id x) x)
            (def (main) (id true))))
  (output (: true Bool)))
