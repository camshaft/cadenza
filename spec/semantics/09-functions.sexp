; Functions and closures — witnesses core-semantics.md §Functions. Functions are
; first-class values (fn), applied by (fn-expr arg), capturing their enclosing
; scope. Functions are SINGLE-ARITY: each function takes exactly one argument.
; Multi-parameter syntax (fn (x y) body) is sugar for currying: (fn x (fn y body)).
; Application (f a b) is sugar for ((f a) b). These are CORE cases (no (needs …)):
; the seed realizes them, because a compiler authored in Cadenza is built from
; functions and closures. Results are (: <value> <Type>).

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
  (input  (do
            (def (ap g v) (g v))
            (def (main) (ap (fn (x) (* x 2)) 7)) (export main)))
  (output (: 14 Int64)))

(case "a function is returned as a result"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value: adder returns a closure over
           its parameter n; the returned function is then applied.")
  (input  (let ((adder (fn (n) (fn (x) (+ x n)))))
            ((adder 10) 5)))
  (output (: 15 Int64)))

(case "a multi-parameter closure keeps its captured environment distinct from its arguments"
  (doc    "A closure that BOTH captures multiple variables AND takes multiple parameters must keep the two
           sets of slots distinct — the captured environment (`a`, `b`) and the applied arguments (`x`, `y`)
           must not be confused by the closure calling convention. `(mk a b)` returns `(fn (x y) (+ (* a x)
           (* b y)))`; with distinguishable powers-of-ten weights any env/arg swap changes the result:
           `((mk 1 1000) 7 3)` = 1·7 + 1000·3 = 3007. A convention that read an argument where a capture
           belongs (or vice versa) would give a different number (7·1 + 3·1000, or 1·1 + 1000·1). Pins that
           a multi-param closure's environment cells and argument slots are separately addressed — captures
           first, then the full-arity arguments.")
  (input  (do
            (def (mk (: a Int64) (: b Int64)) (fn (x y) (+ (* a x) (* b y))))
            (def (main) ((mk 1 1000) 7 3))
            (export main)))
  (output (: 3007 Int64)))

; A function SELECTED BY A RUNTIME CONDITION and then applied — `((if b f g) x)`. `core-semantics.md`
; §A Function Is A First-Class Value: a function is a value an `if` may return, so applying the `if`'s
; result must run whichever function the runtime condition chose. The condition here is a RUNTIME
; parameter (`b`), so the choice is not known at compile time — the application is pushed into each
; branch (a case-of-case / commuting conversion `((if b f g) x)` → `(if b (f x) (g x))`), where each
; branch's function applies. Both branches must yield the same type (Int64), which is the application's
; type. A generation that cannot select a runtime function value declines rather than running.

(case "a function chosen by a runtime condition is applied (true branch)"
  (doc    "`choose` returns one of two functions by its Bool argument; `((choose b) 5)` applies the
           chosen one. With b=true the chosen function is `(fn (x) (+ x 1))`, so the result is 6. The
           condition is a runtime parameter, so the function is selected at run time, not folded.")
  (input  (do
            (def (choose (: b Bool)) (if b (fn (x) (+ x 1)) (fn (x) (+ x 10))))
            (def (main (: b Bool)) ((choose b) 5)) (export main)))
  (call   main (: true Bool))
  (output (: 6 Int64)))

(case "a function chosen by a runtime condition is applied (false branch)"
  (doc    "The false branch of the case above: with b=false the chosen function is `(fn (x) (+ x 10))`,
           so `((choose false) 5)` = 15. The SAME program, run with the other runtime input, takes the
           other branch — pinning that the selection is genuinely by the runtime condition.")
  (input  (do
            (def (choose (: b Bool)) (if b (fn (x) (+ x 1)) (fn (x) (+ x 10))))
            (def (main (: b Bool)) ((choose b) 5)) (export main)))
  (call   main (: false Bool))
  (output (: 15 Int64)))

(case "a runtime-selected function chosen directly at the application head is applied"
  (doc    "The commuting conversion at the application head directly: `((if b (fn (x) (+ x 1)) (fn (x)
           (- x 1))) 10)`. No intervening def — the `if` sits in head position and the application is
           pushed into its branches. With b=true the result is 11.")
  (input  (do
            (def (main (: b Bool)) ((if b (fn (x) (+ x 1)) (fn (x) (- x 1))) 10)) (export main)))
  (call   main (: true Bool))
  (output (: 11 Int64)))

; A function argument passed to a RECURSIVE higher-order function, applied inside the recursion. This
; is the case a function value MUST exist at run time: the recursive `apply-sum` cannot be inlined
; away (it recurses), so its function parameter `g` is a genuine runtime CLOSURE VALUE — the lambda is
; lambda-lifted to a standalone function and applied through an indirect call, not folded. The whole
; point of first-class functions for a compiler (`core-semantics.md` §A Function Is A First-Class
; Value): a pass maps a function over a recursive structure. `apply-sum g n = g(n)+g(n-1)+…+g(1)`.

(case "a function argument is applied through a recursive higher-order function"
  (doc    "`apply-sum` sums `g` applied to each of n, n-1, …, 1 — a recursive HOF. Its `g` parameter is
           a runtime function value (the recursion prevents inlining `g` away), applied via an indirect
           call. With `g = (fn (x) (* x 2))` and n=3: g(3)+g(2)+g(1) = 6+4+2 = 12. The lambda is lifted
           to a standalone function; a generation with no runtime function representation declines.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main (: n Int64)) (apply-sum (fn ((: x Int64)) (* x 2)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 12 Int64)))

(case "a different function argument through the same recursive higher-order function"
  (doc    "The companion pinning that the closure carries the RIGHT code — a DIFFERENT lambda `(fn (x)
           (+ x 100))` through the same `apply-sum`, so the indirect call must dispatch to THIS
           function, not a fixed one. n=3: (3+100)+(2+100)+(1+100) = 306.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main (: n Int64)) (apply-sum (fn ((: x Int64)) (+ x 100)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 306 Int64)))

; A CAPTURING closure through the recursive HOF — the lambda closes over a free variable `k` from its
; creation scope. `core-semantics.md` §A Function Value Captures The Bindings In Scope Where It Is
; Created: `k` is captured BY VALUE into the closure, so each `g(i)` observes the captured `k`. The
; closure is a heap cell (the code pointer + the captured `k`); applying it reads `k` back from the
; cell. `apply-sum (fn (x) (+ x k)) 3 = (3+k)+(2+k)+(1+k) = 6 + 3k`.

(case "a capturing closure is applied through a recursive higher-order function"
  (doc    "The lambda `(fn (x) (+ x k))` CAPTURES `k` from `main`'s scope — a genuine runtime closure
           with an environment, not just a code pointer. Passed to the recursive `apply-sum` and applied
           at each step, every application observes the captured `k`. With k=10: (3+10)+(2+10)+(1+10) =
           36. A generation that cannot store a captured value in the closure declines.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main (: k Int64)) (apply-sum (fn ((: x Int64)) (+ x k)) 3))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 36 Int64)))

; An UNANNOTATED closure parameter — `(fn (x) …)` with no `(: x T)` — is grounded from its USES in the
; body, exactly as a recursive def's unannotated parameter is (`type-system.md`: a parameter's type is
; solved from how it is used). `(fn (x) (* x 2))` uses `x` as an integer operand, so `x : Int64` falls
; out; the closure lifts with that machine type, needing no annotation. Same runtime path as the
; annotated case above, only the parameter's type is inferred rather than declared.

(case "an unannotated closure parameter is grounded from its body and applied at runtime"
  (doc    "`(fn (x) (* x 2))` has no annotation on `x`; its type is solved from the body's `(* x 2)`
           (an integer operand → `x : Int64`). Passed to the recursive `apply-sum` and applied via the
           indirect call, `apply-sum (fn (x) (* x 2)) 3 = 6+4+2 = 12`. Pins that a bare-parameter lambda
           lifts to a runtime closure without requiring an explicit parameter type.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main (: n Int64)) (apply-sum (fn (x) (* x 2)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 12 Int64)))

; A MULTI-PARAMETER runtime closure, applied at FULL arity. `core-semantics.md` §Functions Are
; Single-Arity says a multi-param `(fn (a b) …)` is curried sugar; when the whole function is applied to
; all its arguments at once through a recursive HOF, it lifts to one `(env, a, b) → result` function and
; applies via a single indirect call (no intermediate closure). `ap2 (fn (a b) (+ a b)) n` sums
; `(g i i)` for i = n…1, i.e. `2·(n + … + 1) = n·(n+1)`. (A PARTIAL application of a runtime multi-param
; closure — runtime currying — still declines: it would need to build the intermediate closure.)

(case "a two-parameter closure is applied at full arity through a recursive HOF"
  (doc    "`ap2` applies its two-argument function `g` to `(g i i)` at each recursion level and sums the
           results. `g = (fn (a b) (+ a b))` lifts to a two-parameter closure `(env, a, b) → result`
           applied at full arity; with n=3 the sum is (3+3)+(2+2)+(1+1) = 12. Pins that a multi-parameter
           lambda VALUE runs at run time when applied to all its arguments at once.")
  (input  (do
            (def (ap2 (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
              (if (= n 0) 0 (+ (g n n) (ap2 g (- n 1)))))
            (def (main (: n Int64)) (ap2 (fn ((: a Int64) (: b Int64)) (+ a b)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 12 Int64)))

; core-semantics.md §A Function Is A First-Class Value: a function can be "stored in a data structure."
; A tuple and a list are data structures exactly as a record is, so a function stored in a tuple
; element (or list element) must be extractable and callable, exactly as one stored in a record field
; is. The compiler resolves a function through record member access `.` (the control below runs); the
; same projection-to-lambda resolution must extend to the positional/indexed accessors `(. x N)` and
; `List.at`. A generation that does not yet resolve a stored lambda through those accessors declines
; rather than running the program (reject-don't-miscompile).

(case "a function stored in a tuple element is called after extraction"
  (doc    "A function is a first-class value storable in any data structure. `(tuple (fn (x) (+ x 1))
           9)` stores a function as element 0; `(. … 0)` extracts it and applying it to 5 yields 6.
           This must behave exactly as the record-field companion below — a tuple is a data structure
           like a record. A generation that does not yet resolve the stored lambda through `(. x N)`
           the way it does through `.` declines rather than running the program.")
  (input  ((. (tuple (fn (x) (+ x 1)) 9) 0) 5))
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
  (input   (. ((fn (x) (tuple x 9)) 7) 0))
  (output  (: 7 Int64)))

(case "a field is projected from a record returned by a let-bound function"
  (doc    "The same record-builder reached through a named binding: `mk` is a lambda returning a
           record; `(mk 7)` builds {v: 7} and `(. (mk 7) v)` projects 7. Binding the builder to a name
           does not change that its result is an accessible record.")
  (needs   collections)
  (input   (let ((mk (fn (x) (record (v x)))))
             (. (mk 7) v)))
  (output  (: 7 Int64)))

; A NULLARY function returning a SCALAR is callable: `(def (g) <scalar>)` defines a zero-argument
; function, and `(g)` — a zero-argument application — invokes it, yielding the scalar. Applying a value
; to no arguments is the identity, so a nullary call reduces to the function's body (a bare reference
; `g`, with no call, denotes that same body value). These pin the scalar case the compound-projection
; cases below build on — a nullary call must be recognized as a CALL, not misread as applying the
; body value to zero arguments.

(case "a nullary function returning a scalar is callable"
  (doc    "`(def (mk) 42)` is a nullary function; `(mk)` calls it and yields the scalar 42. A
           zero-argument application invokes the function — it is not an attempt to apply the body
           value 42 to no arguments. A bare reference `mk` (no call) denotes the same value, so `(mk)`
           and `mk` agree; the parenthesized form is the call.")
  (input  (do (def (mk) 42) (def (main) (mk)) (export main)))
  (output (: 42 Int64)))

(case "a nullary helper called and used in arithmetic"
  (doc    "`(def (g) 7)` and `(def (main) (+ (g) 5))`: the nullary `g` is called and its result 7
           added to 5, yielding 12. A nullary call composes in an ordinary expression like any other
           call — its result is a plain value the enclosing operation consumes.")
  (input  (do (def (g) 7) (def (main) (+ (g) 5)) (export main)))
  (output (: 12 Int64)))

(case "a nullary function called from another function's body"
  (doc    "`(def (g) 7)`, `(def (f x) (+ x (g)))`, `(def (main) (f 5))`: `f` calls the nullary `g` in
           its body; `(f 5)` = 5 + 7 = 12. A nullary call works inside a non-entry function body, not
           only at the top level — the callee is reached and reduced wherever the call appears.")
  (input  (do (def (g) 7) (def (f x) (+ x (g))) (def (main) (f 5)) (export main)))
  (output (: 12 Int64)))

; A NULLARY function that returns a compound value must be projectable exactly as a unary one is.
; The cases above return a structure from a function of one parameter; a nullary function `(def (mk)
; <compound>)` called as `(mk)` returns the same kind of value, and projecting a field/element from
; it must yield the value, not trap. The seed projects a UNARY function's structure result correctly
; (above) but TRAPS on a NULLARY function's structure result — a nullary call `(mk)` is not reduced
; to its body for projection the way a unary call `(mk arg)` is, so the access finds no compile-time
; structure and traps at run time. (A nullary function returning a SCALAR works — `(mk)` → 42; only
; a projected compound result traps.)

(case "an element is projected from a tuple returned by a nullary function"
  (doc    "`mk` is a nullary function returning the pair (7, 9); `(mk)` calls it and `(. (mk) 1)`
           projects element 1, yielding 9. A positional access on a nullary function's tuple result
           must project it, exactly as it does for a unary function's result (above) — not trap. The
           seed traps: it does not reduce the nullary call `(mk)` to its tuple body for the access.")
  (input   (do
             (def (mk) (tuple 7 9))
             (def (main) (. (mk) 1)) (export main)))
  (output  (: 9 Int64)))

(case "a field is projected from a record returned by a nullary function"
  (doc    "The record companion: `mk` is a nullary function returning {a: 5}; `(. (mk) a)` projects
           the field, yielding 5. Projecting a field of a nullary function's record result must behave
           like projecting a unary function's record result (above), not trap. The seed traps on the
           nullary case.")
  (needs   collections)
  (input   (do
             (def (mk) (record (a 5)))
             (def (main) (. (mk) a)) (export main)))
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
           error (CDZ0203), the same as `(5 3)` above. The compiler MUST reject it rather than drop
           the `2` and yield `(Some 1)`, which would silently accept the ill-formed application.")
  (input  (Some 1 2))
  (error  CDZ0203))

(case "over-applying a constructor by several arguments is a type error"
  (doc    "The same shape with more extra arguments: `(Some 1 2 3)` desugars to `(((Some 1) 2) 3)`,
           applying the Sum value `(Some 1)` to `2` (already a non-function application). The compiler
           MUST reject it (CDZ0203). Pins that the arity check is on the constructor's single-argument
           application, not forgiving of any number of trailing arguments.")
  (input  (Some 1 2 3))
  (error  CDZ0203))

; The arity check has a lower end too: a UNARY variant applied to ZERO arguments is under-applied. A
; sum type constructor is a single-arity function that produces the tagged variant "when applied to
; EXACTLY ONE argument" (core-semantics.md #A Sum Type Constructor Is A Single-Arity Function). `Some`
; is unary (Option's non-nullary variant, argument type the payload T), so `(Some)` supplies no
; argument — the mirror of the over-application above. A compiler that fabricates a Unit payload for a
; missing argument produces `(Some unit)` — a value of type `Option Unit` the program never wrote,
; observable by matching `(Some x)` binding x=unit, and one that slips past the payload-annotation check
; (`(: (Some) (Option Int64))` yields `(Some unit)` where `(: (Some unit) (Option Int64))` is correctly
; rejected — a Unit payload under an `Int64` annotation). The Unit filler is right only for a NULLARY
; variant, whose argument type IS Unit; a unary variant applied to zero arguments MUST be rejected
; (CDZ0203), exactly as over-application is. A generation that does not yet check the low end declines
; rather than fabricating the payload (reject-don't-miscompile).

(case "under-applying a unary constructor is a type error, not a fabricated unit payload"
  (doc    "`(Some)` applies the unary constructor `Some` to zero arguments — under-application, the
           mirror of `(Some 1 2)` over-application. `Some` produces its Sum value only when applied to
           exactly one argument (core-semantics.md #A Sum Type Constructor Is A Single-Arity Function),
           so `(Some)` MUST be rejected (CDZ0201). A compiler that fabricates a Unit payload yields
           `(Some unit)` — a value of type `Option Unit` the program never wrote, observable by matching
           `(Some x)` and slipping past the payload-annotation check. The Unit filler is correct only for
           a NULLARY variant (argument type Unit); a unary variant demands its one argument. A generation
           that does not yet check the low arity end declines rather than fabricating the payload.")
  (input  (Some))
  (error  CDZ0201))

(case "a recursive def computes over its argument"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments:
           sum-to counts down to 0 through direct self-recursion. sum-to(3) = 3 + 2 + 1 + 0 = 6.")
  (input  (do
            (def (sum-to n)
              (if (= n 0) 0 (+ n (sum-to (+ n -1)))))
            (def (main) (sum-to 3)) (export main)))
  (output (: 6 Int64)))

; ROBUSTNESS: a compiler must DECLINE (or complete), never ABORT, on any well-formed input
; (self-hosting-and-bootstrap.md §An Unsupported Construct Is Declined, Not Miscompiled). Two shapes
; that a naive recursive-descent compiler crashes on — an unproductive compile-time recursion, and a
; deeply nested expression — must instead stop at a recursion/resource bound and decline. A generation
; that cannot reduce such input declines; it does not overflow its own stack.

(case "an unproductive self-recursion is declined, not a compiler crash"
  (doc    "`(def (f) (f))` — a nullary self-call with no base case — cannot be reduced to a value: the
           compile-time evaluator would inline it without end. The compiler must DECLINE it (a
           recursive function it cannot specialize), exactly as an unproductive PARAMETERIZED recursion
           declines, and MUST NOT abort with a native stack overflow. A generation that does not realize
           runtime specialization of such a function declines; the point of the case is 'never crash'.")
  (input  (do (def (f) (f)) (def (main) (f)) (export main)))
  (error  CDZ0999))

(case "a deeply nested constant expression compiles or declines without crashing"
  (doc    "A 64-deep nest of `(+ 1 …)` folds to 65 — well within any reasonable bound. The point is the
           companion the gate cannot record: the SAME shape thousands deep must DECLINE (a
           recursion/resource-limit rejection) rather than overflow the compiler's stack and abort. This
           anchors the shallow end; the compiler bounds its own recursive descent and declines when the
           bound is reached, so a pathological depth is a decline, never a process crash.")
  (input  (do (def (main) (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))) (export main)))
  (output (: 65 Int64)))

; --- A nested CALL chain compiles in roughly LINEAR time, never exponential ----------------------
; The deeply-nested-CONSTANT case above declines cleanly at a pathological depth (the descent-depth
; guard). A nested CALL chain `(f (f (f … 0)))` is a DIFFERENT cost: each level β-inlines the callee
; body, and both `infer` and `lower` reduce every call, recursing into the reduced (fault + type) walk.
; A generation that did not MEMOIZE the reduction and the fault walk re-analyzed each cached-but-shared
; reduced term per enclosing level — EXPONENTIAL in the depth (×2 per level; far worse — 2^depth — when
; the callee DUPLICATES its parameter, so the substituted term doubles each level). A ~20-deep chain
; took seconds, ~50 never finished: a compiler HANG on a trivial, well-formed program. Memoizing the
; β-reduction (a call site reduces once) and the fault collection (a node's faults are collected once)
; makes the chain LINEAR, so it folds to its constant. These pin the folded value at a depth that would
; have taken exponential time unmemoized; the pathology was the GROWTH RATE, so a linear-time compile is
; the property. (A chain nested deeper than the inliner reduces is a resource-limit DECLINE, not a hang.)

(case "a nested chain of function calls compiles in linear time and folds to a constant"
  (doc    "`(f (f (f … (f 0))))` — a depth-18 chain of `(def (f n) (+ n 1))`. Each level inlines the
           callee; the emitted program is a single constant (18). Unmemoized this took time EXPONENTIAL
           in the depth (167ms@16, 652ms@18, 10s@22, never finishing by depth 50) — a hang on a trivial
           program. With the reduction and the fault walk memoized it compiles in milliseconds and folds
           to 18 (0, then +1 eighteen times). Pins that a nested call chain is compiled in roughly linear
           time, never exponentially; the value triangulates the fold is correct, and the depth is chosen
           to be far past where the unmemoized compile was already seconds.")
  (input  (do
            (def (f n) (+ n 1))
            (def (main)
              (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f 0)))))))))))))))))))
            (export main)))
  (call   main)
  (output (: 18 Int64)))

(case "a nested call chain whose callee duplicates its parameter folds without exponential blowup"
  (doc    "The worse shape: `(def (g n) (+ n n))` DUPLICATES its parameter, so each inline DOUBLES the
           substituted term — a depth-d chain is 2^d nodes if re-analyzed naively. Unmemoized, depth 15
           already took ~17s and depth 18 never finished. `(g (g … (g 1)))` at depth 12 computes
           1·2^12 = 4096. Pins that parameter DUPLICATION under nesting does not make the compile
           exponential — the classic β-reduction size explosion a real compiler bounds by memoizing the
           reduction and the per-node analyses. Folds to 4096.")
  (input  (do
            (def (g n) (+ n n))
            (def (main)
              (g (g (g (g (g (g (g (g (g (g (g (g 1)))))))))))))
            (export main)))
  (call   main)
  (output (: 4096 Int64)))

; --- A recursive Bool-returning function used as a condition, in BOTH branch orders --------------
; A recursive predicate — "all elements from i satisfy P" — is a byte/element loop whose recursive
; self-call sits in one branch of an inner `if` and a Bool literal in the other: `(if guard (recurse …)
; false)` (all-so-far, else fail) or its mirror `(if guard false (recurse …))`. Both denote a Bool and
; must type as a Bool CONDITION regardless of which branch holds the self-call — the recursive
; function's return kind is inferred from its body, and a still-unsolved self-call must NOT let branch
; ORDER decide the kind (a Bool-literal branch pins the result to Bool). This is the return-kind
; companion of the recursion cases above, and the exact shape of a reader's byte-by-byte name matcher.

(case "a recursive predicate with the self-call in the then branch is a Bool condition"
  (doc    "`all-lt` tests that every element from i is < the bound: `(if (< i n) (if (< i bound)
           (all-lt (+ i 1) n bound) false) true)` — the recursive self-call is the THEN branch, the
           `false` is the ELSE. Used as an `if` condition, `all-lt` MUST type as Bool; with n=3 and a
           bound of 5 over indices 0,1,2 (all < 5) it is true, so the outer `if` yields 1. Pins that a
           recursive Bool function whose self-call is the then-branch infers a Bool return regardless of
           branch order — the shape a reader's name matcher takes ('all bytes equal so far, else fail').")
  (input  (do
            (def (all-lt i n bound)
              (if (< i n) (if (< i bound) (all-lt (+ i 1) n bound) false) true))
            (def (main) (if (all-lt 0 3 5) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "a recursive predicate with the self-call in the else branch is a Bool condition"
  (doc    "The mirror of the case above: the self-call is the ELSE branch and `false` the THEN —
           `(if (< i n) (if (< i bound) false (all-ge (+ i 1) n bound)) true)`, testing every element
           from i is NOT < the bound. With n=3, bound=0 over indices 0,1,2 (none < 0) it is true → 1.
           Pins that BOTH branch orders of a recursive Bool predicate type identically as a Bool
           condition (the return-kind inference is order-independent).")
  (input  (do
            (def (all-ge i n bound)
              (if (< i n) (if (< i bound) false (all-ge (+ i 1) n bound)) true))
            (def (main) (if (all-ge 0 3 0) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "a recursive def with a match base case computes over its argument"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments,
           with the base case expressed
           as a `match` on the argument rather than an `if`. This is the canonical functional idiom:
           sum-to(n) matches 0 → 0, else n + sum-to(n-1). The base-case arm must be selected from
           the RUNTIME value of n; sum-to(3) = 3 + 2 + 1 + 0 = 6. Companion to the if-based
           `sum-to` above — both must agree.")
  (input  (do
            (def (sum-to n)
              (match n
                (0 0)
                (_ (+ n (sum-to (- n 1))))))
            (def (main) (sum-to 3)) (export main)))
  (output (: 6 Int64)))

(case "recursive factorial with a match base case"
  (doc    "core-semantics.md §Recursion: factorial via a match on the argument. The 0 arm is the
           base case, reached only from the runtime value hitting 0 after counting down; without
           selecting the 0 arm at run time the recursion would never terminate. fact(5) = 120.")
  (input  (do
            (def (fact n)
              (match n
                (0 1)
                (_ (* n (fact (- n 1))))))
            (def (main) (fact 5)) (export main)))
  (output (: 120 Int64)))

(case "recursive fibonacci with literal match base cases"
  (doc    "core-semantics.md §Recursion: two literal base-case arms (0 and 1) matched against the
           runtime argument, and a recursive arm summing the two predecessors. fib(10) = 55.
           Exercises multiple literal arms dispatching on a runtime scrutinee within a recursion.")
  (input  (do
            (def (fib n)
              (match n
                (0 0)
                (1 1)
                (_ (+ (fib (- n 1)) (fib (- n 2))))))
            (def (main) (fib 10)) (export main)))
  (output (: 55 Int64)))

; --- Overflow checking holds THROUGH a recursive call chain, not only at the top level ----
; numeric-model.md #Overflow Is Defined: an integer operation that overflows traps under the checked
; default. The `(+ Int64.max 1)` and `(* Int64.max 2)` cases (06-numeric-model) pin this for a top-level
; operation on constant operands; here the overflowing `*` is buried inside a RECURSION, reached only
; after the call chain unwinds. `fact(20)` = 2432902008176640000 is the largest factorial that fits
; Int64; `fact(21)` = 21·fact(20) ≈ 5.1e19 overflows, and the checked `*` MUST trap when the recursion
; multiplies up to it — not wrap to a garbage value. A generation that emits a checked `*` at the top
; level but an unchecked one inside a recursive helper would compute a wrong `fact(21)` and pass every
; small-input recursion case; this pins the boundary.

(case "the largest factorial that fits the integer type computes exactly"
  (doc    "fact(20) = 2432902008176640000, the largest factorial within Int64 (fact(21) overflows). The
           recursion multiplies 20·19·…·1 with the checked `*`, and every intermediate product stays in
           range, so it computes the exact value — the passing companion of the overflow case below.")
  (input  (do
            (def (fact n)
              (match n
                (0 1)
                (_ (* n (fact (- n 1))))))
            (def (main) (fact 20)) (export main)))
  (output (: 2432902008176640000 Int64)))

(case "a factorial that overflows the integer type traps through the recursion"
  (doc    "fact(21) = 21·fact(20) ≈ 5.1e19, which overflows Int64. The overflowing `*` sits INSIDE the
           recursion, reached as the call chain unwinds; the checked-Int64 default MUST trap there
           (numeric-model.md #Overflow Is Defined), not wrap to a wrong value. Pins that overflow
           checking is emitted on the recursive arithmetic path, not only for a top-level constant
           operation — the recursion companion of `(* Int64.max 2)`.")
  (input  (do
            (def (fact n)
              (match n
                (0 1)
                (_ (* n (fact (- n 1))))))
            (def (main) (fact 21)) (export main)))
  (trap   "integer overflow"))

; --- Two functions may recurse through EACH OTHER (mutual recursion) ----------------------
; core-semantics.md §Recursion + §A Function Is A First-Class Value: recursion need not be
; self-recursion — two top-level defs may call each other, each in scope in the other's body (the same
; lexical resolution that makes a single recursive def work, extended to a pair). `even`/`odd` count
; down through one another; the base case is reached only after the mutual chain unwinds. The existing
; recursion cases are all SELF-recursive; this pins that a mutually-recursive pair resolves and
; terminates too, and returns the Bool result faithfully.

(case "two functions defined by mutual recursion compute a result"
  (doc    "`even` and `odd` are mutually recursive: each calls the other with n-1 until n reaches 0.
           even(10) counts 10→9→…→0 alternating between the two defs and returns true (10 is even). Pins
           that mutual recursion resolves (each def is in scope in the other's body) and terminates via
           the shared base case, carrying the Bool result across the run boundary.")
  (input  (do
            (def (even n) (if (= n 0) true  (odd  (- n 1))))
            (def (odd  n) (if (= n 0) false (even (- n 1))))
            (def (main) (even 10)) (export main)))
  (output (: true Bool)))

(case "the other parity of a mutually-recursive pair"
  (doc    "The companion on the other outcome: even(7) alternates even→odd→…→base and returns false (7
           is odd). Confirms the mutual recursion follows the runtime count to the correct base-case
           result for both parities, not a fixed answer.")
  (input  (do
            (def (even n) (if (= n 0) true  (odd  (- n 1))))
            (def (odd  n) (if (= n 0) false (even (- n 1))))
            (def (main) (even 7)) (export main)))
  (output (: false Bool)))

; --- A TAIL call runs in constant stack ---------------------------------------------------------
; A recursive call in TAIL position (the function's result is exactly that call) must reuse the
; caller's stack frame rather than pushing a new one — otherwise a tail-recursive loop over a RUNTIME
; count grows the wasm call stack one frame per iteration and TRAPS (stack exhausted) on a valid,
; finite input, which the emitted component must be able to complete. The cases above recurse over
; CONSTANT arguments (folded away at compile time, so no runtime frame is ever emitted); these run the
; SAME shapes over a `(call …)` runtime argument, where the self-call is a real emitted call. A
; tail-recursive accumulator counting a million down, and a mutually-tail-recursive even/odd at 100000,
; both complete in O(1) stack — the self-recursive and the cross-function (mutual) tail-call shapes.

(case "a tail-recursive accumulator over a large runtime count iterates in constant stack"
  (doc    "`(def (f n acc) (if (= n 0) acc (f (- n 1) (+ acc 1))))` counted down from a runtime `n` =
           1000000, accumulating +1 each step. The self-call is in TAIL position (it is the `if`'s
           result), so it reuses the frame and the loop runs in constant stack, yielding 1000000. A
           frame-per-iteration recursive call would trap by stack exhaustion well before a million —
           the recorded outcome is the value, not a trap.")
  (input  (do
            (def (f n acc) (if (= n 0) acc (f (- n 1) (+ acc 1))))
            (def (main (: n Int64)) (f n 0))
            (export main)))
  (call   main (: 1000000 Int64))
  (output (: 1000000 Int64)))

; A recursive function with TWO OR MORE NARROW-WIDTH parameters (UInt8/Int8/UInt16/…) threading a narrow
; accumulator through the recursive call. A narrow value lives in an i32 machine slot (a wide Int64 is
; i64); a bare-literal argument (`(f n 0)` — the `0` for a UInt8 `acc`) defaults to Int64, so passing it
; unnormalized pushed an i64 into the i32 parameter slot and rcdzc emitted a STRUCTURALLY INVALID wasm
; module ("expected i32, found i64"). Every call argument must be grounded to its PARAMETER's machine
; width — the same narrow-normalization the operator/if-branch sites already apply, at the call boundary.
; A single narrow parameter and an Int64 two-parameter recursion both worked; the gap was a narrow value
; threaded as the 2nd+ recursive argument. A well-typed narrow-accumulator recursion must never emit
; invalid wasm.

(case "a narrow-width two-parameter recursion compiles to valid wasm and computes"
  (doc    "`(def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) (+ acc 1))))` — a UInt8
           accumulator counting n down while adding 1 to acc. `f(10, 0)` = 10. The narrow `acc`'s
           bare-literal seed `0` (and each recursive `(+ acc 1)`) must be emitted at the parameter's i32
           width, not the default i64, or the call pushes a mismatched slot and the module fails wasm
           validation. The Int64 control above compiles at i64 slots; this pins the narrow width threads
           a recursive argument correctly. Expected: 10.")
  (input  (do
            (def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) (+ acc 1))))
            (def (go (: n UInt8)) (f n 0)) (export go)))
  (call   go (: 10 UInt8))
  (output (: 10 UInt8)))

(case "a narrow-width accumulator that never changes threads through the recursion"
  (doc    "The minimal narrow-threading shape: the accumulator is passed UNCHANGED — `(f (- n 1) acc)` —
           so the only narrow argument at the recursive call is the parameter `acc` itself (no `(+ acc
           1)` to widen it). `f(10, 0)` = 0 (acc starts 0, never incremented). Pins that even a bare
           narrow PARAMETER reference threaded as a recursive argument is emitted at its i32 slot, not
           widened to i64. Expected: 0.")
  (input  (do
            (def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) acc)))
            (def (go (: n UInt8)) (f n 0)) (export go)))
  (call   go (: 10 UInt8))
  (output (: 0 UInt8)))

(case "a mutually tail-recursive even/odd over a large runtime count iterates in constant stack"
  (doc    "The cross-function shape: `even` and `odd` each end in a tail call to the OTHER. At a runtime
           depth of 100000 the alternating tail calls run in constant stack and yield 1 (100000 is
           even). A self-tail-call→loop optimization would not cover this — the tail calls cross between
           two functions — so this pins that a genuine cross-function tail call reuses the frame, not
           only direct self-recursion.")
  (input  (do
            (def (even n) (if (= n 0) 1 (odd (- n 1))))
            (def (odd n)  (if (= n 0) 0 (even (- n 1))))
            (def (main (: n Int64)) (even n))
            (export main)))
  (call   main (: 100000 Int64))
  (output (: 1 Int64)))

(case "a self-recursive Bool-returning function whose recursive call is the then-branch"
  (doc    "A self-recursive function that returns Bool, whose `if` body puts the recursive SELF-CALL in
           the THEN branch and a Bool literal in the ELSE — the `all …` / `every-so-far` shape a reader's
           name matcher takes (`(if (< i n) (if guard (recurse (+ i 1)) false) true)` = \"all positions
           satisfy the guard\"). `(go 0 3)` recurses to the base case and returns true. Pins that a
           recursive function's RETURN KIND settles to Bool regardless of whether the self-call (whose
           kind is a placeholder until the function's kind is known) is the then-branch or the else-branch:
           a Bool-literal sibling must pin the `if`'s result kind to Bool, so the result does not depend on
           branch ORDER. The mutually-recursive `even`/`odd` above already returns Bool, but there each
           branch is a Bool literal or the OTHER function's call; here the branch is the function's OWN
           call, which is the order-dependent kind-inference case (the Bool analogue of the recursive
           heap-accumulator kind race). The mirror shape — self-call in the ELSE, literal in the THEN —
           and an Int-returning self-recursive function both settle correctly; this pins the Bool + then
           combination that does not yet.")
  (input  (do
            (def (go i n) (if (< i n) (go (+ i 1) n) true))
            (def (main) (if (go 0 3) 1 0)) (export main)))
  (output (: 1 Int64)))

; A self-tail call (or any call) evaluates ALL its arguments onto the operand stack simultaneously — a
; parallel move into the parameter slots (the self-tail-loop back-edge) or the call's argument sequence.
; Each argument's scratch is live until the store, so sibling arguments must occupy DISJOINT scratch
; slots. A HEAP-scrutinee `match` argument (Option/List/sum) evaluates its non-reusable scrutinee into an
; i32 handle slot; an arithmetic argument's overflow guard uses an i64 slot. When both shared the same
; scratch `base`, one wasm local was `local.set` at two widths (i64 then i32) and rcdzc emitted a
; STRUCTURALLY INVALID module ("expected i32, found i64"). A match as an OPERAND worked (its i32 slot
; nested above the arith i64 slots), as did a scalar-scrutinee match (it reuses the param, claims no
; slot); the gap was a heap-match sitting DIRECTLY in a call/tail-call argument. Each argument's scratch
; must float above the running high-water — the same disjoint-slot discipline the checked-arith operands
; and the sum-match arms already apply — so a well-typed tail-recursive accumulate-a-matched-value never
; emits invalid wasm. Sibling of the narrow-two-parameter invalid-wasm regression above (both are a
; call-boundary machine-slot mismatch).

(case "a self-tail call passing a heap-match argument compiles to valid wasm"
  (doc    "`(def (f n acc) (if (= n 0) acc (f (- n 1) (match (if (> n 0) (Some n) (None)) ((Some x) (+ acc
           x)) ((None) acc)))))` — a tail-recursive accumulator whose self-call's second argument is a
           `match` over a heap Option. `f(5, 0)` sums 5+4+3+2+1 = 15. rcdzc emitted a STRUCTURALLY INVALID
           wasm module: the self-tail-loop back-edge slot received the heap-match value at a width
           (i32 handle) colliding with the first argument's i64 arith-guard slot. The same shape with the
           match as an OPERAND `(+ acc (match …))` works (15) and a SCALAR-scrutinee match in the same
           argument works, so the machinery is right; the gap is a heap-scrutinee match in a self-tail-call
           argument. Expected: 15.")
  (input  (do
            (def (f (: n Int64) (: acc Int64))
              (if (= n 0) acc (f (- n 1) (match (if (> n 0) (Some n) (None)) ((Some x) (+ acc x)) ((None) acc)))))
            (def (main) (f 5 0)) (export main)))
  (call   main)
  (output (: 15 Int64)))

(case "a non-tail call passing a heap-match argument compiles to valid wasm"
  (doc    "The same scratch-slot collision on the ORDINARY (non-tail) call path: `g(a, m) = a + m`, called
           `(g (- 6 1) (match (Some 10) ((Some x) x) ((None) 0)))`. Argument 0 `(- 6 1)` claims an i64
           arith-guard slot; argument 1's heap-match scrutinee claims an i32 handle slot — they must be
           disjoint. 5 + 10 = 15. Companion of the self-tail-call case; pins that the disjoint-slot fix
           covers a plain call's argument sequence, not only the self-tail-loop back-edge.")
  (input  (do
            (def (g (: a Int64) (: m Int64)) (+ a m))
            (def (main) (g (- 6 1) (match (Some 10) ((Some x) x) ((None) 0)))) (export main)))
  (call   main)
  (output (: 15 Int64)))

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
  (input  (do
            (def (add x y) (+ x y))
            (def (main) ((add 3) 4)) (export main)))
  (output (: 7 Int64)))

; Partial application to a VARIABLE reference (a runtime parameter, a let-bound value) must CAPTURE it in
; the residual (partially-applied) lambda — the primary use of currying: fixing a function's first
; argument to a runtime value. `((sub n) 3)` curries to a residual `(fn (b) (- n b))` whose body
; references `n`; `n`'s binding (the caller's parameter/`let`) must be carried into the residual's scope
; (closed over), exactly as the non-partial `(sub n 3)` has `n` in scope. A currying copy that substitutes
; the name occurrence WITHOUT capturing its binding leaves `n` unbound (CDZ0101). A CONSTANT capture (`(add
; 3)` above) has no free variable to capture and already worked; these pin the variable-reference case.

(case "partial application captures a runtime parameter in the residual lambda"
  (doc    "`(sub a b)` = a − b. Partially applying it to a runtime PARAMETER — `((sub n) 3)` with `n` a
           parameter — curries to a residual lambda that CAPTURES `n`, then subtracts: `n` = 10 gives
           `(sub 10 3)` = 7. The residual body references `n`, so `n`'s binding is carried into the
           residual's scope (closed over), exactly as the non-partial `(sub n 3)`. Was CDZ0101 'unbound
           name n' — the currying copy substituted the name occurrence without capturing its binding.")
  (input  (do
            (def (sub a b) (- a b))
            (def (main (: n Int64)) ((sub n) 3))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 7 Int64)))

(case "partial application captures a let-bound value in the residual lambda"
  (doc    "The let-binding companion: `(let ((m 10)) ((sub m) 3))` partially applies `sub` to the
           let-bound `m`, currying to a residual lambda that captures `m` = 10, so `(sub 10 3)` = 7. Pins
           that the captured argument may be any in-scope binding (a `let` name, not only a parameter or a
           constant) — the residual closes over it.")
  (input  (do
            (def (sub a b) (- a b))
            (def (main) (let ((m 10)) ((sub m) 3)))
            (export main)))
  (output (: 7 Int64)))

(case "a named multi-argument function applies to all its arguments at once"
  (doc    "The DIRECT multi-argument application `(add a b)` — not the explicit curried `((add a) b)` of
           the case above — of a named two-parameter def, at a module entrypoint. `(add2 20 22)` = 42.
           By §Functions Are Single-Arity these are the same program (`(f a b)` desugars to `((f a) b)`),
           but the direct form is the surface shape a program (and a self-hosted compiler reading a call
           node with an argument list) actually writes, and it exercises the N-ary-call lowering — the
           arguments read into an argument list, then pushed left-to-right before the `call` (wasm's
           calling convention) — rather than the nested single-application form. The three-argument
           companion `(add3 10 20 12) = 42` pins that an arbitrary arity, not just two, applies at once.")
  (input  (do
            (def (add2 a b)   (+ a b))
            (def (add3 a b c) (+ a (+ b c)))
            (def (main)       (+ (add2 20 22) (- (add3 10 20 12) 42))) (export main)))
  (output (: 42 Int64)))

(case "the module entrypoint is the def named main regardless of its position"
  (doc    "The module entrypoint is the def NAMED `main` — its position among the defs does not matter.
           Here `main` is the FIRST def and calls a helper `f` DEFINED AFTER it: `(def (main) (f 41))`
           then `(def (f x) (+ x 1))`, so f(41) = 42. This pins two things at once: a forward reference
           (a call to a def that appears later in source order resolves) and, more pointedly, that entry
           selection is by NAME, not by position — the companion cases in this file all place `main`
           last, so nothing else pins that a main-first module has the same entry. A compiler that
           instead took the FIRST def as the nullary entry would lift the parameter-taking `f` as the
           entry and miscompile (or must decline); selecting `main` by name reorders it to the entry
           slot no matter where it sits. The call itself is the ordinary N-ary call lowering — the
           argument `41` pushed before the `call` — exercised across the forward edge.")
  (input  (do
            (def (main) (f 41))
            (def (f x)  (+ x 1)) (export main)))
  (output (: 42 Int64)))

(case "a named function is partially applied, bound, and used"
  (doc    "core-semantics.md §Functions Are Single-Arity: partial application is natural for a named
           def too — `(add 3)` returns a closure awaiting the second argument, bound to `inc` and then
           applied to 4, yielding 7. The lambda form of this already holds; a named def must behave
           identically since multi-param defs desugar to curried single-arity functions.")
  (input  (do
            (def (add x y) (+ x y))
            (def (main) (let ((inc (add 3))) (inc 4))) (export main)))
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
  (input  (do
            (def (is-zero n) (= n 0))
            (def (main) (is-zero 0)) (export main)))
  (output (: true Bool)))

(case "a boolean-returning function called with a false result"
  (doc    "The companion to the case above: is-zero(5) = false. Confirms the Bool result is carried
           faithfully across the run boundary for both truth values, not coerced or truncated.")
  (input  (do
            (def (is-zero n) (= n 0))
            (def (main) (is-zero 5)) (export main)))
  (output (: false Bool)))

(case "a comparison-predicate function returns its boolean result"
  (doc    "core-semantics.md §Ordering Where Offered Is Total, as a function result: `lt5` returns
           `(< n 5)`. lt5(3) = true. A comparison predicate is the most common Bool-returning
           helper a compiler writes (bounds checks, dispatch guards).")
  (input  (do
            (def (lt5 n) (< n 5))
            (def (main) (lt5 3)) (export main)))
  (output (: true Bool)))

(case "a boolean result threaded through a second function"
  (doc    "core-semantics.md §A Function Is A First-Class Value: `b` forwards `a`'s Bool result, and
           `main` returns `b`'s. The Bool return type propagates through the call chain; b(1) = false.")
  (input  (do
            (def (a n) (= n 0))
            (def (b n) (a n))
            (def (main) (b 1)) (export main)))
  (output (: false Bool)))

(case "a boolean result propagates through a three-deep chain of forwarding functions"
  (doc    "core-semantics.md §A Function Is A First-Class Value, one level deeper than the two-function
           case above: `a` forwards `b`'s result, `b` forwards `c`'s, and `c` is the only function with
           a directly Bool body (`(= n 0)`). So a's and b's return types are Bool only TRANSITIVELY —
           neither has a Bool-shaped body; each just returns a call whose callee's return type must
           already be known. Determining every function's result type is therefore a FIXPOINT over the
           call graph, not a single pass: the first pass learns `c` returns Bool, the second propagates
           that to `b`, the third to `a` and `main`. A single-pass return-type computation (enough for
           the two-function case, where one propagation step suffices) leaves `a`/`b` unresolved — and a
           compiler that defaults an unresolved function result to the integer type would give `a` and
           `b` mismatched result kinds versus the `i32`/Bool value they actually forward. a(0) = true.
           This pins that result-type resolution iterates to convergence across an arbitrary-depth chain,
           the companion of the two-deep case and of the recursive Bool cases earlier in this file.")
  (input  (do
            (def (main) (a 0))
            (def (a n)  (b n))
            (def (b n)  (c n))
            (def (c n)  (= n 0)) (export main)))
  (output (: true Bool)))

(case "a boolean function result bound by let is still a boolean"
  (doc    "core-semantics.md §Binding Is Lexical: binding a predicate's result to a name and
           returning that name does not change its type. The program's result is the Bool true.")
  (input  (do
            (def (is-zero n) (= n 0))
            (def (main) (let ((r (is-zero 0))) r)) (export main)))
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
  (input  (do
            (def (f b) (if b 10 20))
            (def (main) (f true)) (export main)))
  (output (: 10 Int64)))

(case "a boolean-parameter function applied to false"
  (doc    "The companion of the case above: f(false) = 20. Confirms both Bool argument values are
           bound and dispatched correctly through a call.")
  (input  (do
            (def (f b) (if b 10 20))
            (def (main) (f false)) (export main)))
  (output (: 20 Int64)))

(case "a boolean parameter forwarded to a conditional result"
  (doc    "core-semantics.md §A Function Is A First-Class Value: `both` takes two Bools and returns
           `b` when `a` is true, else false — a logical AND. both(true, true) = true. Exercises two
           Bool parameters in one signature, curried.")
  (input  (do
            (def (both a b) (if a b false))
            (def (main) (both true true)) (export main)))
  (output (: true Bool)))

; --- A parameter whose type the body does not constrain is polymorphic -------------------
; The cases above pin a parameter's type via a use in the body (`(if b …)` forces Bool). The
; identity function `(def (id x) x)` uses `x` only by returning it, so nothing in the body
; constrains its type: `id` is polymorphic (∀a. a → a) and applies to a value of ANY type,
; returning it unchanged (core-semantics.md §Applying A Function Binds Its Parameters To Its
; Arguments — the parameter is bound to whatever argument it is applied to; type-system.md
; §Inference — an unconstrained parameter generalizes to a type variable rather than defaulting
; to Int64). Inference realizes this: an unconstrained parameter generalizes to a type variable,
; so `id : ∀a. a → a` accepts `(id 42)` AND `(id true)`, each application instantiating `a` at
; its argument's type. These pin the polymorphic case; the Int64 companion is the control.

(case "the identity function applied to an integer returns the integer"
  (doc    "The control: `(def (id x) x)` applied to an Int64 returns it. id(42) = 42. The body does
           not constrain `x`'s type; applying to an integer determines it here.")
  (input  (do
            (def (id x) x)
            (def (main) (id 42)) (export main)))
  (output (: 42 Int64)))

(case "the identity function applied to a boolean returns the boolean"
  (doc    "The polymorphic case: the same `(def (id x) x)` applied to a Bool returns the Bool.
           id(true) = true. Nothing in `id`'s body restricts `x` to Int64 — it is returned
           unchanged — so `id` is polymorphic and accepts a Bool argument exactly as it accepts an
           Int64. Inference generalizes the unconstrained parameter to a type variable (`id : ∀a. a →
           a`), so both `(id 42)` and `(id true)` type-check, each application instantiating `a` at its
           argument's type.")
  (input  (do
            (def (id x) x)
            (def (main) (id true)) (export main)))
  (output (: true Bool)))

; --- A bare parameter PROJECTED in the body is constrained only at the call site ------------------
; A companion of the polymorphic-parameter cases above, for a STRUCTURAL use: a bare (unannotated)
; parameter that the body PROJECTS — `(. r field)` / `(. t N)` — is unconstrained in the standalone
; body (its type is `Any` until the def inlines), exactly as an arithmetic use `(+ r 1)` leaves it
; `Any`. A non-recursive def inlines at its call site, so the projection's real check runs THERE,
; where the argument's compound type flows in — the same way the identity function's parameter type is
; determined by the argument. Earlier the seed rejected the body standalone with a self-contradictory
; CDZ0201 "requires a record/tuple, found Any" (an `Any` operand is unconstrained, not a proven
; non-compound), spuriously failing a well-typed helper; arithmetic on an `Any` parameter never
; faulted, so projection was the outlier. A genuinely non-compound argument (an Int64) is still
; rejected at the call site (the reduced body projects a non-record) — the check is deferred, not
; dropped.

(case "a helper projects a record parameter constrained by its argument"
  (doc    "`(def (get-x r) (. r x))` reads field `x` of its bare parameter `r`. `r` is unconstrained in
           the body (typed `Any` — nothing pins it until `get-x` inlines), so the field read is NOT a
           fault there; the argument `(mk v)` is a runtime `(record (x v) (y 2))`, so at the call site
           `r` is that record and `(. r x)` is `v`. With v=41 the result is 41. Pins that a bare
           parameter projected in the body types like an arithmetic use of it — constrained at the call
           site, not spuriously rejected standalone.")
  (input  (do
            (def (get-x r) (. r x))
            (def (mk n)    (record (x n) (y 2)))
            (def (main (: v Int64)) (get-x (mk v)))
            (export main)))
  (call   main (: 41 Int64))
  (output (: 41 Int64)))

(case "a helper projects a tuple parameter constrained by its argument"
  (doc    "The tuple companion: `(def (fst t) (. t 0))` projects element 0 of its bare parameter. `t` is
           unconstrained in the body; the argument `(mk v)` is a runtime `(tuple v 2)`, so `(. t 0)` is
           `v`. With v=9 the result is 9. The positional analogue of the record helper — a bare
           parameter projected by position is likewise constrained at the call site.")
  (input  (do
            (def (fst t) (. t 0))
            (def (mk n)  (tuple n 2))
            (def (main (: v Int64)) (fst (mk v)))
            (export main)))
  (call   main (: 9 Int64))
  (output (: 9 Int64)))

(case "a helper sums two fields of a record parameter"
  (doc    "The body uses the parameter's fields in ARITHMETIC: `(+ (. r x) (. r y))`. Both field reads
           are on the unconstrained `r`, and both feed `+`; at the call site `r` is `(record (x v) (y
           2))`, so the sum is v+2. With v=7 the result is 9. Pins that MULTIPLE projections of one bare
           compound parameter all resolve at the call site and compose with arithmetic on the results.")
  (input  (do
            (def (sum-xy r) (+ (. r x) (. r y)))
            (def (mk n)     (record (x n) (y 2)))
            (def (main (: v Int64)) (sum-xy (mk v)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 9 Int64)))

(case "projecting a field of a non-compound argument is rejected at the call site"
  (doc    "The deferral is not a drop: `(def (get-x r) (. r x))` is well-formed standalone (its `r` is
           unconstrained), but applying it to an Int64 — `(get-x v)` with `v : Int64` — makes the
           reduced body project a field of an integer, which has no defined result. type-system.md
           §Member Access Projects A Record Field: the seed rejects CDZ0201 at the call site (the
           argument's Int64 type flows into `r`), so a bad structural use is still caught — just where
           the concrete type is known, not in the polymorphic body.")
  (input  (do
            (def (get-x r) (. r x))
            (def (main (: v Int64)) (get-x v))
            (export main)))
  (error  CDZ0201))

; --- A recursive parameter used ONLY as a call argument infers from the callee -----------------------
; A RECURSIVE def's parameter that no primitive operator ever touches — it is only PASSED AS AN ARGUMENT
; to another def, threaded unchanged through the recursion — is still determined: its type is the
; callee's parameter type at that position. `(def (f a n) (… (twice a) … (f a (- n 1))))` uses `a` only
; in `(twice a)`, so `a`'s type is `twice`'s parameter type (Int64, pinned by `twice`'s own `(+ a a)`).
; The recursive-parameter solver reads that argument-position constraint; without it `a` stayed
; unconstrained and the def declined "a recursive function with an unannotated parameter is not yet
; inferred", refusing a well-typed program (annotating `a` compiled the same program — inference, not
; codegen, was the gap). The constraint is precise: a parameter passed to a POLYMORPHIC callee (whose
; parameter is itself unconstrained) is NOT pinned, so a generic position stays generic. This is the last
; inference piece the byte-walking reader family (a `Bytes` param threaded through a recursive walk via a
; helper) needs — see the CBOR-reader cases in 10-bytes.sexp.

(case "a recursive parameter used only as a call argument infers from the callee's parameter type"
  (doc    "`f` is recursive; its parameter `a` is threaded unchanged through the recursion and used ONLY
           as the argument of `(twice a)` — no primitive operator touches `a` directly. Its type is
           `twice`'s parameter type: `twice`'s body `(+ a a)` pins that parameter to Int64, so `a` infers
           Int64 without an annotation. Was declined ('a recursive function with an unannotated parameter
           is not yet inferred') because the solver derived a constraint only from an operator applied to
           the parameter or the self-call, never from an argument position. `f(5, 3)` sums `twice(5)` =
           10 three times → 30. Inference, not codegen, was the only gap.")
  (input  (do
            (def (twice a) (+ a a))
            (def (f a n) (if (< n 1) 0 (+ (twice a) (f a (- n 1)))))
            (def (main) (f 5 3)) (export main)))
  (call   main)
  (output (: 30 Int64)))

(case "a recursive byte walk threading a Bytes parameter through a helper infers without annotation"
  (doc    "The motivating instance (the CBOR-reader family): `be` is recursive; its `Bytes` parameter `b`
           is threaded unchanged and used only as the first argument of `(byte-at b i)`. `byte-at`'s body
           `(match (Bytes.at b i) …)` pins its first parameter to `Bytes`, so `b` infers `Bytes` from that
           argument position — no annotation needed. The non-recursive helper `byte-at` itself needs no
           annotation. The bytes 1, 2, 3 are read and summed over three steps → 6. Was declined for want
           of the argument-position constraint.")
  (input  (do
            (def (byte-at b i)
              (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (be b i n)
              (if (< n 1) 0 (+ (byte-at b i) (be b (+ i 1) (- n 1)))))
            (def (main) (be (Bytes.of (list 1 2 3)) 0 3))
            (export main)))
  (call   main)
  (output (: 6 Int64)))

; A function's name is an ordinary lexical binding, and #Binding Is Lexical resolves a reference to the
; NEAREST enclosing binding of that name — regardless of the name's capitalization. So a `def` whose name
; happens to start with an uppercase letter binds that name exactly as a lowercase one does, and a call to
; it MUST invoke the defined function, not be reinterpreted as a constructor of some tagged variant. A
; compiler that treats any capitalized name in call position as an ad-hoc constructor — synthesizing
; `(Foo 10)` for `(Foo 10)` — silently IGNORES the user's `(def (Foo x) …)` binding and returns a
; constructor value instead of the function's result: a wrong value that contradicts #Binding Is Lexical
; (the nearest binding of `Foo` is the `def`, not a prelude constructor, and there is no `Foo` variant
; declared) and #A Module Binds Its Name In Its Enclosing Scope. The lowercase companion `(def (bar) …)`
; is called correctly; the uppercase one must be too — capitalization is not a binding-precedence rule.

(case "a function whose name is capitalized is called, not treated as a constructor"
  (doc    "`(def (Foo x) (+ x 1))` binds the name `Foo` to a function in the module's scope; `(Foo 10)`
           MUST resolve to that binding (core-semantics.md #Binding Is Lexical: a name resolves to the
           nearest enclosing binding) and invoke it, yielding 11. `Foo` is not a variant of any declared
           sum type, and even if it were, the user's `def` is the nearest binding. A compiler that treats
           a capitalized name in call position as an ad-hoc constructor synthesizes the value `(Foo 10)`
           and IGNORES the `def` — a wrong value (the function computing x+1 is bypassed). Capitalization
           is not a binding-precedence rule: the lowercase `(def (bar) …)` companion is called correctly,
           and the uppercase one must be too. A generation that does not resolve a capitalized name to its
           user binding declines rather than answering `(Foo 10)` (reject-don't-miscompile).")
  (input  (do
            (def (Foo x) (+ x 1))
            (def (main) (Foo 10)) (export main)))
  (output (: 11 Int64)))

(case "a parameter carries a type annotation in the signature"
  (doc    "A `def` parameter may be written `(: name Type)` in the signature — an annotation in BINDER
           position. `(: a Int64)` binds `a` (the annotation names the binder, not an opaque form) and
           constrains its type to Int64, per type-system.md §Annotations Constrain, Never Contradict:
           the annotation is an additional unification constraint on the parameter, not an override. The
           body references `a` exactly as an unannotated parameter — the annotation is transparent to the
           value. `(annotated 20 22)` = 42. Pins that a signature reads through a `(: name Type)` binder
           to the name it binds, so an author can pin a parameter's type where inference would otherwise
           leave it open — the disambiguation an ambiguous runtime parameter requires.")
  (input  (do
            (def (annotated (: a Int64) b) (+ a b))
            (def (main)                    (annotated 20 22)) (export main)))
  (output (: 42 Int64)))

(case "a parameter annotation contradicting its use is rejected"
  (doc    "An annotation constrains and MUST NOT contradict (type-system.md §Annotations Constrain,
           Never Contradict): a parameter annotated `(: a Bool)` but used where an Int64 is required —
           `(+ a 1)` unifies `a` with the integer operand of `+` — cannot be reconciled, so the program
           is rejected (CDZ0203) rather than having the annotation silently replace the inferred type or
           the use silently reinterpret the annotation. The contradiction is between the WRITTEN Bool and
           the INFERRED Int64 at the same binding, exactly the conflicting-annotation shape.")
  (input  (do
            (def (bad (: a Bool)) (+ a 1))
            (def (main)           (bad true)) (export main)))
  (error  CDZ0203))

; The case above contradicts the annotation via the BODY (`(: a Bool)` then `(+ a 1)`). The dual is a
; contradiction via the ARGUMENT: a parameter whose annotation and body AGREE, called with an argument
; of a conflicting type. An argument's type MUST be checked against its parameter's type at the call
; (type-system.md §Annotations Constrain, Never Contradict; core-semantics.md — a well-typed program
; does not go wrong). A compiler that reduces a call by substituting the argument into the body erases
; the parameter↔argument relationship, so this check must be made at the call site, not left to the
; reduced body — else a mistyped argument is silently accepted (and, once the mis-accepted value is
; USED at its claimed type, miscompiled). These pin the argument side, the complement of the body side.

(case "an Int argument to a Bool-annotated parameter is rejected"
  (doc    "`(def (f (: x Bool)) x)` annotates `x` as Bool and returns it (body agrees with the
           annotation). `(f 5)` passes an Int64 where a Bool is required — a type error (CDZ0203). The
           argument's type is checked against the parameter's ANNOTATION at the call, not silently
           accepted; the degenerate identity body would otherwise let the mis-accepted 5 flow back out
           as a returned value. Distinct from the body-contradiction case above: here the annotation and
           body agree and it is the ARGUMENT that disagrees.")
  (input  (do (def (f (: x Bool)) x) (def (main) (f 5)) (export main)))
  (error  CDZ0203))

(case "an Int argument to a parameter used as a Bool condition is rejected"
  (doc    "`(def (f x) (if x 1 2))` uses the unannotated `x` as a Bool condition, so `x : Bool` is
           inferred from its use. `(f 5)` passes an Int64 — a type error (CDZ0203). Reducing the call
           substitutes 5 into `(if x 1 2)`, giving `(if 5 1 2)` whose condition is a non-Bool — the
           reduced body's fault is reported, so the program is rejected rather than miscompiled to an
           invalid component. The correctly-typed `(f true)` yields 1.")
  (input  (do (def (f x) (if x 1 2)) (def (main) (f 5)) (export main)))
  (error  CDZ0203))

(case "a Bool argument to a parameter used in integer addition is rejected"
  (doc    "The mirror direction: `(def (f x) (+ x x))` infers `x : Int64` from the addition; `(f true)`
           passes a Bool — a type error (CDZ0203). The reduced body `(+ true true)` faults on the
           non-integer operand, so the call is rejected, not miscompiled. The correctly-typed `(f 5)`
           yields 10. Pins that an argument is checked against a body-INFERRED parameter type, not only
           an explicit annotation.")
  (input  (do (def (f x) (+ x x)) (def (main) (f true)) (export main)))
  (error  CDZ0203))

; --- A FUNCTION-TYPED parameter annotation is checked against the passed function, RESULT included -
; The higher-order analogue of the scalar arg-vs-param checks above. A parameter annotated with a
; function type `(-> A B)` constrains the ARGUMENT to a function of that type — parameter AND result
; (type-system.md §Annotations Constrain, Never Contradict). Passing an `A -> B'` function whose result
; `B'` disagrees with the annotated `B` is a type error, and the check must descend through NESTED
; arrows (a curried `(-> A (-> C D))` checks the inner result too). A passed lambda is typed as its own
; arrow type — a bare parameter contributes `Any` (so it unifies with any expected parameter type, no
; over-rejection), only a definite RESULT disagreement faults. The scalar-vs-function mismatch (`(f 5)`
; to a function parameter) is already caught; this closes the function-vs-function deep-result hole.

(case "a function-typed parameter annotation's result type is checked against the argument"
  (doc    "`(def (f (: g (-> Int64 Bool))) (g 41))` declares `g` as `Int64 -> Bool`, but `(f (fn (x) (+
           x 1)))` passes an `Int64 -> Int64` function — the RESULT types disagree (Bool vs Int64), a
           type error (CDZ0203). The annotation must not be silently dropped: the passed lambda is typed
           as its arrow type `Int64 -> Int64` and unified against the declared `Int64 -> Bool`, so the
           result mismatch faults. The higher-order analogue of the scalar `(f 5)`-to-a-Bool-parameter
           rejection above.")
  (input  (do (def (f (: g (-> Int64 Bool))) (g 41)) (def (main) (f (fn (x) (+ x 1)))) (export main)))
  (error  CDZ0203))

(case "a function-typed parameter annotation is not silently discarded in the body"
  (doc    "The witness that the annotation CONSTRAINS the body, not merely the call: `(def (f (: g (->
           Int64 Bool))) (+ (g 41) 1))` — if `g`'s result were the annotated Bool, `(+ (g 41) 1)` would
           be `(+ Bool 1)` and reject. It must reject (CDZ0203): `g`'s result is fixed to Bool by the
           annotation, so using it as an integer operand contradicts. A generation that dropped the
           annotation typed `(g 41)` as the actual Int64 and computed 43 — the annotation having no
           effect. Pins that the fn-type annotation governs `g`'s result type throughout the body.")
  (input  (do (def (f (: g (-> Int64 Bool))) (+ (g 41) 1)) (def (main) (f (fn (x) (+ x 1)))) (export main)))
  (error  CDZ0203))

(case "a curried function-type annotation checks its inner result type against the argument"
  (doc    "The annotation check descends through NESTED arrows: `(def (f (: g (-> Int64 (-> Int64
           Bool)))) ((g 1) 2))` annotates `g` as `Int64 -> Int64 -> Bool`, but `(fn (a) (fn (b) (+ a
           b)))` is `Int64 -> Int64 -> Int64` — the INNER result types disagree (Bool vs Int64). Must
           reject (CDZ0203). The function-type unification is structural, so a mismatch at any arrow
           depth is caught, not only the outermost result.")
  (input  (do (def (f (: g (-> Int64 (-> Int64 Bool)))) ((g 1) 2)) (def (main) (f (fn (a) (fn (b) (+ a b))))) (export main)))
  (error  CDZ0203))

(case "a correctly-annotated function parameter is accepted"
  (doc    "The passing boundary: `(def (f (: g (-> Int64 Int64))) (g 41))` with the matching `Int64 ->
           Int64` function `(fn (x) (+ x 1))` yields 42. Pins that a CORRECT function-type annotation is
           accepted — the fix REJECTS a mismatched annotation without over-rejecting a matching one. A
           bare-param lambda's parameter type is `Any`, so it unifies with the declared `Int64`
           parameter freely; only a result disagreement faults, and here there is none.")
  (input  (do (def (f (: g (-> Int64 Int64))) (g 41)) (def (main) (f (fn (x) (+ x 1)))) (export main)))
  (output (: 42 Int64)))

(case "a function-typed parameter with a matching Bool-returning argument is accepted"
  (doc    "A matching non-Int result: `(def (f (: g (-> Int64 Bool))) (g 41))` applied to `(fn (x) (< x
           5))` — an `Int64 -> Bool` function that agrees with the annotation — yields `(< 41 5)` =
           false. Complements the rejection cases: when the passed function's result type MATCHES the
           annotated one, the program is accepted and runs, confirming the check is a genuine agreement
           test, not a blanket rejection of function-typed parameters.")
  (input  (do (def (f (: g (-> Int64 Bool))) (g 41)) (def (main) (f (fn (x) (< x 5)))) (export main)))
  (output (: false Bool)))

; --- Runtime arguments to the entrypoint: (call <export> <arg>…) --------------------------------
; Every case above calls a parameterized function with CONSTANT arguments, so the compiler folds the
; whole program to a value at compile time — a real strength (a compile-provable trap fails the build),
; but it means the emitted component's runtime machinery (parameter slots, `local.get`, a genuine
; runtime `+`/`*`/comparison, a branch on a runtime value) is never exercised. A value that arrives at
; RUN TIME — an argument supplied to the exported entry from outside the component — cannot be folded:
; the entry becomes `input -> output` and its parameter crosses the boundary as a lifted value
; (contracts/component-abi.md §The Entry Is A Plain Function; §The Exported Interface Is The Declared
; Signature — the interface is read from the export's declared PARAMETER and result types). These cases
; use the `(call <export> <arg>…)` clause to run the exported entry with runtime arguments, so the
; operation over the parameter is emitted as real instructions rather than constant-folded. Each `<arg>`
; is a `(: <value> <Type>)` value-form; the runner coerces it to the export's declared parameter type.
; The parameter MUST be annotated (`(: x Int64)`) — an entry's boundary representation follows its
; declared signature, and an unannotated parameter has no boundary width, so the compiler declines it.
; CORE cases (no `(needs …)`): the seed realizes a parameterized export, because a compiler authored in
; Cadenza is itself a component whose entry takes its input as a runtime argument.

(case "the entrypoint returns its runtime argument unchanged"
  (doc    "The identity entry: `(def (main (: x Int64)) x)` exported and called with the runtime
           argument 42. The argument arrives from OUTSIDE the component (not a compile-time constant), so
           it cannot be folded — the body is a bare parameter reference lowered to a `local.get` of the
           entry's one parameter slot, lifted back across the boundary. Pins that a parameterized entry
           receives a runtime value and returns it, the minimal exercise of the boundary parameter path
           the folded nullary cases never reach (contracts/component-abi.md §The Entry Is A Plain
           Function — an entry is `input -> output`, its parameter type carrying a boundary form).")
  (input  (do (def (main (: x Int64)) x) (export main)))
  (call   main (: 42 Int64))
  (output (: 42 Int64)))

(case "the entrypoint adds one to its runtime argument"
  (doc    "`(def (main (: x Int64)) (+ x 1))` exported and called with 41. One operand of `+` is the
           runtime parameter `x`, so the addition CANNOT fold to a constant — it is emitted as a genuine
           runtime `i64.add` over the parameter's local slot and the literal 1. (Contrast the folded
           `(+ 2 3)` in 06-numeric-model, which the compiler reduces to 5 at build time.) This is the
           smallest case that exercises the runtime arithmetic path a program's machinery actually runs;
           41 + 1 = 42.")
  (input  (do (def (main (: x Int64)) (+ x 1)) (export main)))
  (call   main (: 41 Int64))
  (output (: 42 Int64)))

(case "the entrypoint multiplies its runtime argument"
  (doc    "`(def (main (: x Int64)) (* x 3))` called with 7 — a runtime `i64.mul` over the parameter and
           the literal 3, yielding 21. Companion to the runtime `+` case, pinning that multiplication too
           is emitted as a real instruction (not folded) when an operand is a runtime argument.")
  (input  (do (def (main (: x Int64)) (* x 3)) (export main)))
  (call   main (: 7 Int64))
  (output (: 21 Int64)))

(case "the entrypoint sums its two runtime arguments"
  (doc    "A two-parameter entry `(def (main (: a Int64) (: b Int64)) (+ a b))` called with 20 and 22.
           BOTH operands are runtime arguments, so the `+` is a runtime `i64.add` over two parameter
           slots — nothing is constant. Pins that an entry takes MORE than one boundary argument, each in
           its own local slot in signature order, and the arguments are supplied in order; 20 + 22 = 42.")
  (input  (do (def (main (: a Int64) (: b Int64)) (+ a b)) (export main)))
  (call   main (: 20 Int64) (: 22 Int64))
  (output (: 42 Int64)))

(case "the entrypoint returns its runtime boolean argument"
  (doc    "`(def (main (: b Bool)) b)` called with the runtime boolean `true`. Pins that a Bool crosses
           the entry boundary as a runtime argument (not only an integer) and lifts back unchanged — the
           boolean boundary representation on the parameter side, mirroring the Bool result cases.")
  (input  (do (def (main (: b Bool)) b) (export main)))
  (call   main (: true Bool))
  (output (: true Bool)))

(case "the entrypoint compares its runtime argument to a bound"
  (doc    "`(def (main (: x Int64)) (< x 10))` called with 5 — a runtime `<` comparison between the
           parameter and the literal 10, producing a Bool. The comparison cannot fold (one operand is a
           runtime value), so it is emitted as a real runtime comparison. 5 < 10 is true. Pins that a
           relational operator over a runtime argument runs as an instruction and yields a boundary Bool.")
  (input  (do (def (main (: x Int64)) (< x 10)) (export main)))
  (call   main (: 5 Int64))
  (output (: true Bool)))

(case "the entrypoint branches on its runtime argument"
  (doc    "`(def (main (: x Int64)) (if (< x 0) 0 x))` — clamp-to-zero — called with -3. The `if`
           condition is a runtime comparison on the parameter, so the branch is a genuine runtime
           structured `if` (not a compile-time choice of arm): with x = -3 the condition holds and the
           entry yields 0. Pins that control flow driven by a runtime argument is emitted as a real
           branch, the last piece of the runtime machinery the folded nullary cases skip. (A negative
           argument also exercises the runner taking a leading-`-` value as the argument, not a flag.)")
  (input  (do (def (main (: x Int64)) (if (< x 0) 0 x)) (export main)))
  (call   main (: -3 Int64))
  (output (: 0 Int64)))

; --- Narrow-width runtime arguments cross as their FAITHFUL component primitive -------------------
; The eight aliased widths (Int8/16/32, UInt8/16/32/64 and their `(Int N)` expansions) each have a
; component boundary representation: they cross as `s8`/`u8`/`s16`/`u16`/`s32`/`u32`/`s64`/`u64`, NOT as
; a wider machine slot. So a `(: n UInt8)` entry parameter takes a `u8` at the edge — the host cannot
; pass 300 for it (wasmtime rejects an out-of-range u8), which is exactly the safety a narrow width buys.
; These `(call …)` cases run a narrow-width entry over a runtime argument, exercising the faithful
; boundary lift on the parameter side and the emitted narrow (i32-slot, range-checked) operation. CORE
; cases (no `(needs …)`): the seed realizes the aliased widths' boundary forms.

(case "an unsigned-byte entrypoint takes and returns a u8 at the boundary"
  (doc    "`(def (main (: n UInt8)) n)` exported and called with 200. The parameter crosses as the
           component `u8` (its faithful width, not a machine s32/u32), lifts to the i32 slot the body
           reads, and lowers back to `u8` — 200. Pins that an aliased narrow width has a boundary form
           and that a UInt8 argument round-trips through the component edge unchanged.")
  (input  (do (def (main (: n UInt8)) n) (export main)))
  (call   main (: 200 UInt8))
  (output (: 200 UInt8)))

(case "a runtime unsigned-byte addition traps on overflow of its width"
  (doc    "`(def (main (: a UInt8) (: b UInt8)) (+ a b))` called with (200, 55) = 255, which fits UInt8
           (max 255). The `+` is emitted (both operands runtime) as the width-generic checked op: it
           computes in the i32 slot and range-checks the result back to 0..=255. 200+55 fits, so it
           returns 255 — the companion overflow (200+56=256) is the trap case pinned in 06-numeric-model.
           Pins that a NARROW runtime arithmetic op runs over faithful-u8 boundary arguments.")
  (input  (do (def (main (: a UInt8) (: b UInt8)) (+ a b)) (export main)))
  (call   main (: 200 UInt8) (: 55 UInt8))
  (output (: 255 UInt8)))

; The addition above has two NARROW runtime operands; the far more common shape is a narrow parameter
; and a BARE INTEGER LITERAL — incrementing a byte, comparing a narrow counter to a bound. A bare
; literal is width-polymorphic (it defaults to Int64 on its own), so it MUST take the width of the
; operand it is combined with — `(+ x 1)` with `x : UInt8` treats `1` as a UInt8. The operands of a
; binary op share one machine representation; a literal left at its Int64 default beside a narrow
; (i32-slot) parameter is a width clash the emitted op cannot express. These pin that a narrow-param-
; plus-literal op computes (the literal grounded to the operand's width), for `+`, `*`, and comparison.

(case "a narrow-width parameter plus a bare literal computes at the parameter width"
  (doc    "`(def (main (: x UInt8)) (+ x 1))` called with 100 = 101. The bare literal `1` takes `x`'s
           UInt8 width (a literal is width-polymorphic until an operand constrains it), so the addition
           is a homogeneous narrow op — not a UInt8-plus-Int64 clash. The annotated form `(+ x (: 1
           UInt8))` and two-narrow-param `(+ a b)` (above) already compute; this pins the bare-literal
           operand, the common increment-a-byte shape.")
  (input  (do (def (main (: x UInt8)) (+ x 1)) (export main)))
  (call   main (: 100 UInt8))
  (output (: 101 UInt8)))

(case "a signed narrow-width parameter plus a bare literal computes at the parameter width"
  (doc    "The signed sibling: `(def (main (: x Int8)) (+ x 1))` called with 50 = 51. The literal takes
           the Int8 width, so the op is a homogeneous narrow (i32-slot) addition. Pins that the
           literal-width unification is not UInt8-specific.")
  (input  (do (def (main (: x Int8)) (+ x 1)) (export main)))
  (call   main (: 50 Int8))
  (output (: 51 Int8)))

(case "a narrow-width parameter compared to a bare literal computes at the parameter width"
  (doc    "The comparison face: `(def (main (: x UInt8)) (> x 50))` called with 100 = true. The literal
           `50` takes `x`'s UInt8 width, so the comparison's operands share one machine slot. Pins that
           the bare-literal width unification applies to every binary op over a narrow parameter, not
           only `+`.")
  (input  (do (def (main (: x UInt8)) (> x 50)) (export main)))
  (call   main (: 100 UInt8))
  (output (: true Bool)))

(case "a narrow-width parameter plus a bare literal computes inside a helper function"
  (doc    "`(def (bump (: x UInt8)) (+ x 1))` called via `(bump y)` where `y : UInt8` = 101. The
           narrow-param-plus-literal op computes in a non-entry function body exactly as in the entry —
           the literal takes the parameter's width wherever the operation appears.")
  (input  (do (def (bump (: x UInt8)) (+ x 1)) (def (main (: y UInt8)) (bump y)) (export main)))
  (call   main (: 100 UInt8))
  (output (: 101 UInt8)))

; A `match` over a NARROW-width scrutinee whose arms include both a bare-literal arm and a binder (or a
; narrow value) arm must reconcile the arm widths: every arm produces the match's RESULT type, so a
; bare-literal arm (which defaults to Int64 on its own) takes the result's narrow width — otherwise a
; default-Int64 arm beside a narrow arm pushes a mismatched machine slot and wasm rejects the block.
; This is the match-arm analogue of the bare-literal-operand width reconciliation above. The corpus
; gates match binders only over Int64; these pin the narrow-scrutinee binder path.

(case "a match binder over a narrow scrutinee returns the bound value"
  (doc    "`(match x (0 100) (n n))` with `x : UInt8`, called with 5, binds the non-zero scrutinee to
           `n` and returns it = 5. The literal arm `100` takes the match's UInt8 result width (so both
           arms share the i32 slot); the binder arm returns the scrutinee at its UInt8 width. A binder
           over an Int64 scrutinee already works; this pins the narrow scrutinee's binder.")
  (input  (do (def (main (: x UInt8)) (match x (0 100) (n n))) (export main)))
  (call   main (: 5 UInt8))
  (output (: 5 UInt8)))

(case "a signed narrow match binder returns the bound value"
  (doc    "The signed sibling: `(match x (0 100) (n n))` with `x : Int8`, called with 5 = 5. Confirms
           the narrow-arm-width reconciliation spans every aliased narrow width, not just UInt8.")
  (input  (do (def (main (: x Int8)) (match x (0 100) (n n))) (export main)))
  (call   main (: 5 Int8))
  (output (: 5 Int8)))

(case "a narrow match binder used in arithmetic with the scrutinee"
  (doc    "`(match x (0 0) (n (+ n x)))` with `x : UInt8`, called with 50 = 100. The binder `n` is the
           narrow scrutinee, and the arithmetic arm combines it with `x` at UInt8; the zero-arm literal
           `0` takes the UInt8 result width. Pins that the bound value is usable in a downstream op, not
           only returned directly.")
  (input  (do (def (main (: x UInt8)) (match x (0 0) (n (+ n x)))) (export main)))
  (call   main (: 50 UInt8))
  (output (: 100 UInt8)))

(case "matching against zero probes the normalized narrow value, not the raw wide slot"
  (doc    "A match probe against the literal 0 may be emitted as wasm `eqz` (a single zero test rather than
           `const 0 ; eq`). It MUST test the NORMALIZED narrow value, not the raw machine slot that carries
           it: `(match (UInt8.wrap n) (0 100) (_ 200))` with `n = 2^32` truncates to the UInt8 0 — its low
           8 bits are zero — so the `0` arm fires and the result is 100, EVEN THOUGH the wide i64 slot
           holding 2^32 is non-zero. An `eqz` applied to the un-masked wide slot would see 2^32 ≠ 0 and
           wrongly take the `_` arm (200). Pins that the zero-probe operates on the value at its width (the
           `UInt8.wrap` result masked to 8 bits), the match-probe companion of the narrow-operand
           normalization the arithmetic cases require.")
  (input  (do (def (main (: n Int64)) (match (UInt8.wrap n) (0 100) (_ 200))) (export main)))
  (call   main (: 4294967296 Int64))
  (output (: 100 Int64)))

; An `if` whose branches MIX a narrow-width value and a bare integer literal must reconcile the branch
; widths: both branches produce the `if`'s RESULT type, so a bare-literal branch (which defaults to
; Int64 on its own) takes the result's narrow width — otherwise a default-Int64 branch beside a narrow
; branch pushes a mismatched machine slot into the block. This is the `if`-branch analogue of the
; bare-literal-operand (`(+ x 1)`) and bare-literal-match-arm reconciliations above. The corpus gates
; `if` over narrow conditions but never an `if` whose branches mix a narrow value and a bare literal.

(case "an if with a narrow branch and a bare-literal branch computes at the narrow width"
  (doc    "`(if c x 0)` with `x : UInt8` and `c : Bool`: the then-branch is the UInt8 param, the
           else-branch a bare literal `0`. With c = true the result is x = 200. The literal branch takes
           the `if`'s UInt8 result width so both branches share the i32 slot — not a UInt8-vs-Int64
           machine-type clash. The annotated form `(if c x (: 0 UInt8))` and the both-same-param form
           already compute; this pins the bare-literal branch.")
  (input  (do (def (main (: x UInt8) (: c Bool)) (if c x 0)) (export main)))
  (call   main (: 200 UInt8) (: true Bool))
  (output (: 200 UInt8)))

(case "a signed narrow if-branch opposite a bare literal computes at the narrow width"
  (doc    "The signed sibling: `(if c x 0)` with `x : Int8`, c = true → 50. Confirms the `if`-branch
           width reconciliation spans every aliased narrow width, not just UInt8.")
  (input  (do (def (main (: x Int8) (: c Bool)) (if c x 0)) (export main)))
  (call   main (: 50 Int8) (: true Bool))
  (output (: 50 Int8)))

(case "a narrow value in the else branch opposite a bare literal computes at the narrow width"
  (doc    "Branch-position independence: `(if c 0 x)` puts the bare literal in the THEN branch and the
           narrow `x` in the ELSE; with c = false the result is x = 200. The reconciliation grounds
           whichever branch is the bare literal, so both orders compute identically.")
  (input  (do (def (main (: x UInt8) (: c Bool)) (if c 0 x)) (export main)))
  (call   main (: 200 UInt8) (: false Bool))
  (output (: 200 UInt8)))

(case "a signed-byte entrypoint returns its runtime argument"
  (doc    "`(def (main (: n Int8)) n)` called with -128 (Int8.min). The parameter crosses as the
           component `s8`, so the sign is preserved at the boundary (an s8 -128, not a widened s32). Pins
           the signed narrow-width boundary form and that Int8.min round-trips.")
  (input  (do (def (main (: n Int8)) n) (export main)))
  (call   main (: -128 Int8))
  (output (: -128 Int8)))

; --- Truncating conversion `T.wrap` over a runtime operand: the emitted mask-and-reinterpret ------
; `T.wrap` truncates any integer to width T, keeping the low bits of its two's-complement value — the
; principled, TYPE-directed form of a byte-truncation (the width comes from the type `UInt8`, not a magic
; op name). It is TOTAL — it never traps, whatever the input (the checked companion `T.of`, which reports
; an out-of-range value rather than truncating, returns an Option and arrives with sum types). On a
; runtime operand it cannot fold, so the conversion is EMITTED (a slot move + a mask, + a sign-extend for
; a signed target). These `(call …)` cases run `wrap` over a runtime Int64 argument, pinning that the
; emitted path agrees with the constant fold across the slot-crossing (i64 source → narrow target) the
; folded cases never reach. CORE (no `(needs …)`): the seed realizes `wrap` for the aliased widths.

(case "a runtime truncation to an unsigned byte keeps the low bits, total on negatives"
  (doc    "`(def (main (: n Int64)) (UInt8.wrap n))` — a runtime truncating conversion (a self-hosted
           encoder truncating a computed value to a byte), emitted as an `i32.wrap_i64` of the parameter
           then a mask. `wrap` keeps the low 8 bits and is TOTAL (never traps, unlike the checked
           `T.of`). Exercised at two operands: n = 300 = 0x12C keeps 0x2C = 44 : UInt8; n = -1 keeps the
           low 8 bits of -1's two's-complement (all ones) = 255 : UInt8, WITHOUT trapping on the negative
           value — the emitted conversion reinterprets the low bits exactly as the constant fold does.")
  (input  (do (def (main (: n Int64)) (UInt8.wrap n)) (export main)))
  (call   main (: 300 Int64))
  (output (: 44 UInt8))
  (call   main (: -1 Int64))
  (output (: 255 UInt8)))

(case "a runtime truncation into a signed byte sign-extends"
  (doc    "`(def (main (: n Int64)) (Int8.wrap n))` called with 200. The low 8 bits (0xC8) have bit 7
           set, so as a SIGNED Int8 the value is -56 (sign-extended) — crossing the boundary as s8. Pins
           that a signed target's `wrap` sign-extends from the target's high bit, distinct from the
           unsigned truncation above.")
  (input  (do (def (main (: n Int64)) (Int8.wrap n)) (export main)))
  (call   main (: 200 Int64))
  (output (: -56 Int8)))

; ── An argument bound to an unused parameter is UNOBSERVED, so its trap is not raised ────────────────
; core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed: an argument whose value the
; function body never uses is unobserved — its value reaches neither the result nor a host call — so an
; implementation MAY decline to evaluate it, eliding the trap it would have raised. The dual anchor pins
; that the moment the body USES the parameter, the argument is observed and its trap fires. This is the
; call-boundary companion of the un-projected tuple element in 05-compound-types.sexp. (An argument that
; PROVABLY traps and is elided also earns a non-error diagnostic — CDZ0305 — asserted by a compiler unit
; test; the gate observes the run, and the build succeeds.)

(case "an argument bound to an unused parameter is not evaluated, so its trap does not occur"
  (doc    "`(def (f x y) x)` ignores its second parameter `y`. Calling `(f 7 (/ 1 d))` with d = 0 passes
           a division by zero as the unused argument. `y`'s value is never observed in the body, so the
           argument need not be evaluated and its trap does not occur — the program yields 7. Uses a
           runtime (parameter-driven) div0 so this is a genuine emitted-code question, not a constant
           fold. The anchor below pins that a USED argument's trap DOES fire.")
  (input  (do (def (f x y) x) (def (main (: d Int64)) (f 7 (/ 1 d))) (export main)))
  (call   main (: 0 Int64))
  (output (: 7 Int64)))

(case "an argument bound to a used parameter IS observed, so its trap occurs (the anchor)"
  (doc    "The control: `(def (f x y) y)` returns its SECOND parameter, so `(f 7 (/ 1 d))` with d = 0
           observes the trapping argument — its value flows out as the result — and must trap. Pins that
           the elision above is specifically about an argument whose parameter is UNUSED; the trap fires
           the moment the argument is observed. The call-boundary dual of the projected-tuple-element
           anchor in 05-compound-types.sexp.")
  (input  (do (def (f x y) y) (def (main (: d Int64)) (f 7 (/ 1 d))) (export main)))
  (call   main (: 0 Int64))
  (trap   "division by zero"))
