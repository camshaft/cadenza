; Functions and closures — witnesses core-semantics.md §Functions. Functions are
; first-class values (fn), applied by (fn-expr arg), capturing their enclosing
; scope. Functions are SINGLE-ARITY: each function takes exactly one argument.
; Multi-parameter syntax (fn (x y) body) is sugar for currying: (fn x (fn y body)).
; Application (f a b) is sugar for ((f a) b). The seed realizes these, because a compiler authored
; in Cadenza is built from functions and closures. Results are (: <value> <Type>).

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

; The case above captures a CONSTANT `y` and folds. These pin closure capture semantics at RUN TIME (a
; boundary parameter flows into the capture, so nothing folds) and with two closures alive at once — the
; cases the single-closure constant case cannot: capture is BY VALUE at creation (a later same-named
; binding does NOT rebind an existing closure's capture) and each closure holds its OWN captured
; environment (two closures from one factory do not share a capture slot). A representation that captured
; by reference / late-bound the name, or that shared one environment cell across closures, would give a
; different — and here numerically distinct — answer.

(case "a closure captures its environment by value at creation, unaffected by a later same-named binding"
  (doc    "`(let ((k n)) (let ((f (fn (x) (+ x k)))) (let ((k 1000)) (f 1))))` — `f` captures `k = n` at
           creation; the INNER `(let ((k 1000)) …)` introduces a NEW `k` in scope at the APPLICATION site,
           but `f` observes the `k` it captured, not the later one. So `(f 1)` = `1 + n`, NOT `1 + 1000`:
           n=5 → 6, n=40 → 41 (core-semantics.md §A Function Value Captures The Bindings In Scope Where It
           Is Created — capture is by value at creation). A compiler that late-bound the free `k` to the
           nearest binding at APPLICATION time would answer 1001. The runtime companion of the
           application-site-shadowing case above, with the shadowing binding sitting BETWEEN creation and
           application.")
  (input  (do (def (main (: n Int64))
                (let ((k n))
                  (let ((f (fn (x) (+ x k))))
                    (let ((k 1000))
                      (f 1))))) (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 40 Int64)) (output (: 41 Int64)))

(case "two closures from one factory capture distinct values"
  (doc    "`(adder k) = (fn (x) (+ x k))` built twice — `add3` captures 3, `add10` captures 10 — both alive
           at once. `(- (add10 n) (add3 n))` = `(n+10) - (n+3)` = 7 for EVERY `n` (n=5 and n=0 both → 7).
           Pins that each closure holds its OWN captured environment: a representation that shared one
           capture cell across the two closures (both ending at the last-built 10, or both at 3) would give
           0, not 7. The two captures are distinct and independent.")
  (input  (do (def (adder k) (fn (x) (+ x k)))
              (def (main (: n Int64)) (let ((add3 (adder 3)) (add10 (adder 10))) (- (add10 n) (add3 n)))) (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))

(case "a list of closures each keeps its own capture, selected by a runtime index"
  (doc    "Three closures `(mk 10)`, `(mk 20)`, `(mk 30)` — each `(mk k) = (fn (x) (+ x k))` capturing its
           own `k` — are stored in a LIST and one is selected by a runtime index, then applied. `apply-at
           fs i 1` = `(elem i)(1)` = `1 + (10|20|30)`: i=0 → 11, i=2 → 31, an out-of-bounds index → -1. Pins
           that closures carried in a collection each retain their distinct capture (the list does not
           collapse them to one environment), and that indexing selects the intended one at run time — the
           collection companion of the two-factory-closures case.")
  (input  (do (def (mk k) (fn (x) (+ x k)))
              (def (apply-at fs i x) (match (List.at fs i) ((Some f) (f x)) (None -1)))
              (def (main (: i Int64)) (apply-at (list (mk 10) (mk 20) (mk 30)) i 1)) (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: 2 Int64)) (output (: 31 Int64))
  (call   main (: 9 Int64)) (output (: -1 Int64)))

; A lambda that references an ENCLOSING binding and is applied INSIDE that binding's scope — the capture
; is a free variable bound further out, not inside the lambda's own body. core-semantics.md §A Function
; Value Captures The Bindings In Scope Where It Is Created: `(+ x k)` reads `k` from the enclosing `let`.
; Applying the lambda β-reduces `(+ 5 k)` and `k` must still resolve to that enclosing `k` — the free
; variable is PRESERVED across the reduction, not lost. (A generation that copied the free name into an
; orphan scope would report `k` unbound; this pins that a captured enclosing binding survives.)

(case "a lambda applied in the scope of the binding it captures observes that binding"
  (doc    "`(let ((k 10)) ((fn (x) (+ x k)) 5))` — the lambda captures `k` from the enclosing `let` and is
           applied to 5 inside that `let`. The application reduces to `(+ 5 k)` with `k = 10`, yielding
           15. The captured free variable `k` binds OUTSIDE the lambda body, so β-reducing the application
           must preserve its resolution to the enclosing `let`, not lose it.")
  (input  (let ((k 10)) ((fn ((: x Int64)) (+ x k)) 5)))
  (output (: 15 Int64)))

(case "a lambda captures an enclosing function parameter and is applied in its body"
  (doc    "The same capture over a def PARAMETER rather than a `let`: `(def (f k) ((fn (x) (+ x k)) 5))`
           — the lambda captures `f`'s parameter `k` and is applied inside `f`'s body. `f(10)` reduces
           `(+ 5 k)` with `k = 10` = 15. Pins that an enclosing PARAMETER is captured and preserved
           across the β-reduction exactly as an enclosing `let` binding is.")
  (input  (do
            (def (f (: k Int64)) ((fn ((: x Int64)) (+ x k)) 5))
            (def (main (: k Int64)) (f k)) (export main)))
  (call   main (: 10 Int64))
  (output (: 15 Int64)))

(case "an inner lambda captures an enclosing match-arm binder and is applied"
  (doc    "The same capture over a MATCH-ARM binder rather than a `let` or a parameter: the `A` arm binds
           `m` = 7, then an inner lambda `(fn (x) (+ x m))` captures `m` and is applied to 3, giving
           3 + 7 = 10. A match-arm binder must be visible to an inner lambda's capture exactly as a `let`
           binding or a parameter is (both above), and as `m` is when used directly `(+ m 3)`. The binder
           resolves to a `SumPayload` reading the arm's scrutinee; that resolution must be PINNED as a
           capture so β-reducing the applied inner lambda preserves it, rather than copying the reference
           into the reduced body where it re-resolves unbound (which rejected a valid program CDZ0101).")
  (input  (do (type C (A Int64) (B))
              (def (main) (match (A 7) ((A m) ((fn (x) (+ x m)) 3)) ((B) 0))) (export main)))
  (output (: 10 Int64)))

(case "an inner lambda captures an enclosing tuple-pattern binder and is applied"
  (doc    "The tuple-pattern companion: matching `(tuple 7 9)` binds `a` = 7 (an `Elem`-path binder), and an
           inner lambda `(fn (x) (+ x a))` captures `a` and is applied to 3 → 10. Pins that the capture of a
           pattern binder is general to a tuple-slot binder, not only a variant payload — both resolve to a
           `SumPayload` (bare `Elem` vs `Payload` path) and both must be pinned as a capture.")
  (input  (do (def (main) (match (tuple 7 9) ((tuple a b) ((fn (x) (+ x a)) 3)))) (export main)))
  (output (: 10 Int64)))

; A capturing lambda BOUND to a name (a `let` binding) and then applied — `(let ((g (fn (x) (+ x k))))
; (g 5))` where `g` closes over an enclosing `k`. Binding the closure to a name does not change that it
; folds when applied: `g` is copy-propagated (a lambda value is never kept as a runtime slot), so `(g 5)`
; β-reduces to `(+ 5 k)` and `k` resolves to its enclosing binding. Pins that a NAMED capturing closure
; applied directly folds exactly as the anonymous form does.

(case "a named capturing closure applied directly folds through its capture"
  (doc    "`(let ((k 10)) (let ((g (fn (x) (+ x k)))) (g 5)))` — `g` is a let-bound closure capturing the
           outer `k`; applying it yields (+ 5 10) = 15. A NAMED capturing closure applied directly must
           fold like the anonymous `((fn (x) (+ x k)) 5)` form — the name binding is transparent.")
  (input  (let ((k 10)) (let ((g (fn ((: x Int64)) (+ x k)))) (g 5))))
  (output (: 15 Int64)))

(case "a named capturing closure applied more than once folds at each use"
  (doc    "The same named closure `g` applied twice — `(+ (g 5) (g 6))` with `g = (fn (x) (+ x k))`,
           k = 10 — folds each application: (5+10) + (6+10) = 31. Two uses of a capturing closure each
           β-reduce independently; the closure value is not built at run time.")
  (input  (let ((k 10)) (let ((g (fn ((: x Int64)) (+ x k)))) (+ (g 5) (g 6)))))
  (output (: 31 Int64)))

; A closure factory — a function RETURNING a capturing closure — whose result is applied at the call
; site. `(mk k)` returns `(fn (x) (+ x k))` closing over `k`; `((mk 10) 5)` applies that returned
; closure. core-semantics.md §A Function Is A First-Class Value ("returned as a result") composed with
; capture: the returned closure carries `mk`'s parameter `k`. The whole chain folds — `mk` inlines,
; the returned lambda β-reduces — so no runtime closure survives.

(case "a closure factory's returned capturing closure is applied at the call site"
  (doc    "`(def (mk k) (fn (x) (+ x k)))` returns a closure over `k`; `((mk 10) 5)` = (+ 5 10) = 15. The
           returned closure captures the factory's parameter and applies correctly — a returned closure
           composed with a capture, both folded away.")
  (input  (do
            (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k)))
            (def (main) ((mk 10) 5)) (export main)))
  (output (: 15 Int64)))

; The SAME returned capturing closure, but BOUND with `let` before it is applied. `(let ((f (mk n))) (f 3))`
; must compute exactly as the inline `((mk n) 3)` above — the binding names the closure value but does not
; change its meaning. This was a MISCOMPILE (invalid wasm, silently written at exit 0): `should_keep_binding`
; short-circuits a syntactic `Resolved::Lambda` init to avoid a speculative lift that pollutes the capture
; set, but `(mk n)` is an `Apply` that REDUCES to a capturing lambda — it slipped past, was lifted, and
; recorded the captured `n`; the copy-propagated `((mk n) 3)` then β-reduced to `(+ n 3)` with the shared `n`
; lowered to a `Core::Captured` env-read in `main` (no env) → an i32/i64 slot mismatch. The fix propagates a
; binding whose value reduces to a lambda, so it folds inline like the un-bound form. Every relaxation of the
; trigger (inline application, a higher-order argument, a non-capturing closure) already worked; this pins the
; let-bound one.

(case "a returned capturing closure bound with let and applied folds like the inline form"
  (doc    "`(mk n)` returns `(fn (x) (+ n x))` capturing the parameter `n`; `(let ((f (mk n))) (f 3))` binds
           that closure to `f` and applies it, so with `n` = 10 the result is 10 + 3 = 13 — identical to the
           inline `((mk n) 3)`. A `let`-bound closure value round-trips through its binding at the value's
           own representation, not the default scalar width; a binding whose init reduces to a lambda is
           copy-propagated so its application folds inline rather than mis-lifting the closure into the local.")
  (input  (do
            (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x)))
            (def (main (: n Int64)) (let ((f (mk n))) (f 3)))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 13 Int64)))

(case "a let-bound returned closure applied twice folds each application independently"
  (doc    "The multi-use companion: `(let ((f (mk n))) (+ (f 3) (f 4)))` binds the returned capturing closure
           once and applies it twice; each application folds independently, so with `n` = 10 the result is
           (10 + 3) + (10 + 4) = 27. Pins that binding a closure value and applying it more than once keeps
           each use correct — the multi-reference case of the let-bound-closure fold.")
  (input  (do
            (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x)))
            (def (main (: n Int64)) (let ((f (mk n))) (+ (f 3) (f 4))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 27 Int64)))

(case "a capturing closure stored in a tuple is extracted and applied"
  (doc    "A capturing closure `(fn (x) (+ x k))` (over an enclosing `k = 7`) stored as a tuple element,
           projected out, and applied: `((. (tuple (fn (x) (+ x k)) 9) 0) 5)` = (+ 5 7) = 12. Storing a
           capturing closure in a data structure and reading it back preserves its capture — the whole
           thing folds (the tuple projection reaches the closure, which β-reduces).")
  (input  (let ((k 7))
            ((. (tuple (fn ((: x Int64)) (+ x k)) 9) 0) 5)))
  (output (: 12 Int64)))

(case "a closure carried in a sum payload is extracted by a match and applied"
  (doc    "core-semantics.md §A Function Is A First-Class Value: a function stored in a SUM variant's
           payload — the callback-in-a-variant shape — is extracted by a match binder and applied.
           `(Some (fn (n) (* n 2)))` carries a closure; `(match … ((Some f) (f 5)) …)` binds `f` to the
           payload and applies it, yielding 10. The closure is reached through the variant PAYLOAD (a
           `sum-payload` heap read), not a `let`/tuple projection the fold reduces through, so its
           application is a runtime `call_indirect` on the extracted closure cell — the payload-binder
           analogue of applying a function-typed PARAMETER. Pins that a closure survives being stored in
           and read back out of a sum variant, and that a match binder over a function-typed payload is a
           callable runtime function-value source (not merely a foldable projection).")
  (input  (match (Some (fn ((: n Int64)) (* n 2)))
            ((Some f) (f 5))
            ((None _) 0)))
  (output (: 10 Int64)))

(case "a CAPTURING closure carried in a sum payload keeps its capture through the match binder"
  (doc    "The capturing companion: the closure stored in the sum payload closes over a RUNTIME value, and
           that capture must survive being boxed into the variant and read back out. `(mk k)` returns
           `(Some (fn (x) (+ x k)))` capturing the parameter `k`; `(match (mk k) ((Some f) (f 5)) …)`
           extracts `f` and applies it, so with `k` = 100 the result is 5 + 100 = 105. The closure cell
           carried in the `Some` payload must retain its captured environment (not just the code pointer):
           a lowering that stored the function but dropped the capture would compute 5 (or read garbage).
           Pins that a closure's captured environment round-trips through a sum-variant payload, the
           capturing extension of the non-capturing payload-closure case above.")
  (input  (do
            (def (mk (: k Int64)) (Some (fn ((: x Int64)) (+ x k))))
            (def (main (: k Int64)) (match (mk k) ((Some f) (f 5)) ((None _) -1)))
            (export main)))
  (call   main (: 100 Int64))
  (output (: 105 Int64)))

(case "a closure carried in a USER-declared sum's payload is extracted and applied"
  (doc    "The USER-SUM companion of the built-in-payload closure case: `(type T (Mk (-> Int64 Int64)))`
           declares a variant carrying a FUNCTION, and `(T.Mk (fn (n) (* n 2)))` stores a closure in it.
           `(match … ((T.Mk f) (f 5)))` extracts and applies it → 10. Unlike a built-in `Some`/`Ok`
           (whose ctor scheme threads the payload type so the extracted closure's application types
           directly), a USER variant's payload is a declared arrow `(-> Int64 Int64)` reached through the
           payload binder; applying it must peel that arrow to type the result. Pins that a closure
           carried in a user-declared sum applies exactly as one in a built-in sum — the callback-in-a-
           variant idiom a user's own event/AST types rely on. A generation without sum-type
           declaration declines it.")
  (input  (do
            (type T (Mk (-> Int64 Int64)))
            (def (main) (match (T.Mk (fn ((: n Int64)) (* n 2))) ((T.Mk f) (f 5))))
            (export main)))
  (output (: 10 Int64)))

; --- An UNANNOTATED closure typed from its STORAGE CONTEXT's declared arrow -----------------------
; The payload-closure cases above ANNOTATE the lambda's parameter (`(fn ((: n Int64)) …)`). But when a
; closure is stored in a position whose type is DECLARED — a variant constructor's payload
; `(-> Int64 C)`, a built-in `Some`/`Ok` payload — the parameter type need not be repeated: it is the
; arrow's parameter, threaded from the storage site into the lambda. core-semantics.md §A Function Is A
; First-Class Value + §Applying A Function Binds Its Parameter To Its Argument: a closure typed against
; the function type its context requires. (`type_of` computes a lambda's type bottom-up, so a bare `(fn
; (n) …)` whose body does not otherwise pin `n` stayed `Any` and declined "a closure's parameter type has
; no machine representation" / "a tuple element of type Any"; the expected-arrow fallback closes that.)

(case "an unannotated closure in a user variant payload is typed from the declared arrow"
  (doc    "`(type T (Susp (-> Int64 C)))` declares a variant carrying a function `Int64 → C`. Storing
           `(T.Susp (fn (n) (C.A n)))` — the lambda's parameter UNANNOTATED — types `n : Int64` from the
           payload's declared arrow, not from a repeated annotation. Extracted by the match binder `f` and
           applied to 7, its `C.A` result matches the `(C.A m)` arm → 7. Pins that a closure stored in a
           declared-function-typed payload takes its parameter type from that declaration — the callback-
           in-a-variant idiom without redundant annotations.")
  (input  (do
            (type C (A Int64) B)
            (type T (Susp (-> Int64 C)))
            (def (main) (match (T.Susp (fn (n) (C.A n))) ((T.Susp f) (match (f 7) ((C.A m) m) ((C.B) 0)))))
            (export main)))
  (output (: 7 Int64)))

(case "an unannotated closure in a Some payload is typed from the Option's element arrow"
  (doc    "The built-in companion: `(Some (fn (n) (C.A n)))` carries an unannotated closure whose element
           type the `Some` payload fixes to the function `Int64 → C`, so `n : Int64` without annotation.
           Applied to 7 through the match binder → its `C.A` result yields 7. Pins the expected-arrow
           threading works for a built-in Option payload exactly as for a user variant.")
  (input  (do
            (type C (A Int64) B)
            (def (main) (match (Some (fn (n) (C.A n))) ((Some f) (match (f 7) ((C.A m) m) ((C.B) 0))) ((None) 0)))
            (export main)))
  (output (: 7 Int64)))

(case "an unannotated closure with an unused parameter in a payload takes the declared parameter type"
  (doc    "The lambda's parameter is not used by its body — `(fn (n) (C.B))` ignores `n` — so the body
           cannot constrain `n` at all; its type comes SOLELY from the payload's declared arrow `(-> Int64
           C)`. Without the expected-arrow fallback this declined 'a closure's parameter type has no
           machine representation' (nothing pinned `n`). Applied to 7, the body yields `C.B` → the `(C.B)`
           arm → 0. Pins that the declared arrow types even a body-unconstrained parameter.")
  (input  (do
            (type C (A Int64) B)
            (type T (Susp (-> Int64 C)))
            (def (main) (match (T.Susp (fn (n) (C.B))) ((T.Susp f) (match (f 7) ((C.A m) m) ((C.B) 0)))))
            (export main)))
  (output (: 0 Int64)))

(case "a capturing unannotated closure in a payload is typed from the declared arrow"
  (doc    "The capturing extension: `(mk k)` returns `(T.Susp (fn (n) (C.A (+ n k))))` — an unannotated
           closure that CAPTURES the runtime parameter `k` AND takes its own parameter type from the
           payload arrow `(-> Int64 C)`. Extracted and applied to 7 with k = 100 → `C.A (7 + 100)` → 107.
           Pins that the storage-context parameter typing composes with capture — the closure retains its
           environment through the variant payload and still types its parameter from the declaration.")
  (input  (do
            (type C (A Int64) B)
            (type T (Susp (-> Int64 C)))
            (def (mk (: k Int64)) (T.Susp (fn (n) (C.A (+ n k)))))
            (def (main (: k Int64)) (match (mk k) ((T.Susp f) (match (f 7) ((C.A m) m) ((C.B) 0)))))
            (export main)))
  (call   main (: 100 Int64))
  (output (: 107 Int64)))

(case "an unannotated closure typed Int8 from context overflows a constant like an explicit Int8 param"
  (doc    "The NARROW-WIDTH edge of context typing: `app : ((-> Int8 Int8)) -> Int8` applied `(app (fn (n)
           (+ n 1)))`, where `g` is applied to the constant 127. The unannotated `n` is typed Int8 from
           app's declared `(-> Int8 Int8)` arrow, so `(+ n 1)` with n=127 is `127 + 1 = 128`, which
           OVERFLOWS Int8 (max 127) — a constant OPERATION with no value → the SAME CDZ0304 (ConstTrap)
           the explicit `(fn ((: n Int8)) (+ n 1))` gives on the same constant. The recovered narrow width
           must reach the body's CONST-FOLD, not only the runtime path: without it the fold ran at the
           default Int64 and returned 128 (a wrong value where an overflow is due). A RUNTIME argument
           traps for both the annotated and unannotated forms; this pins that the compile-time const-fold
           carries the context width too.")
  (input  (do (def (app (: g (-> Int8 Int8))) (g 127))
              (def (main) (app (fn (n) (+ n 1)))) (export main)))
  (error  CDZ0304))

(case "an unannotated closure typed Int8 from context computes an in-range constant"
  (doc    "The value companion: the SAME `(app (fn (n) (+ n 1)))` but `g` applied to 5 — `5 + 1 = 6` fits
           Int8, so the context-Int8 closure computes 6 rather than over-rejecting. Together with the
           overflow case above this pins that the recovered narrow width is applied to the const-fold in
           BOTH directions — an out-of-range constant rejects, an in-range one computes — exactly as an
           explicit Int8 param does.")
  (input  (do (def (app (: g (-> Int8 Int8))) (g 5))
              (def (main) (app (fn (n) (+ n 1)))) (export main)))
  (output (: 6 Int8)))

; --- A lambda that MATCHES ITS OWN PARAMETER, passed through a higher-order function -------------
; core-semantics.md §A Function Is A First-Class Value + §Applying A Function Binds Its Parameter To Its
; Argument: a callback that DESTRUCTURES its argument (`(fn (c) (match c …))`) is an ordinary first-class
; value — passed to a HOF that applies it to the HOF's own argument. When the HOF is itself inlined, the
; callback is applied through a NESTED β-reduction: the callback's parameter IS the match scrutinee, and
; the reduction substitutes the argument for it. A pattern binder in the callback body reads the scrutinee
; via a `SumPayload` (resolve Case 6); the reduction must re-resolve that binder against the SUBSTITUTED
; scrutinee, not share its pre-substitution occurrence (which, lowered standalone, is a slot-less
; parameter — the "no local slot" decline this pins closed). Distinct from a callback that only RETURNS
; or PROJECTS its parameter (no scrutinee materialization); the destructuring match is the exercised path.

(case "a higher-order function applies a callback that matches its own sum argument"
  (doc    "`apply-to` takes a callback `f` and a `C` value `c`, applying `(f c)`. The callback `(fn (p)
           (match p ((C.A n) n) ((C.B) 0)))` destructures its OWN parameter. Because `apply-to` inlines,
           the callback is applied to `c` through a nested β-reduction where `p` — the match scrutinee — is
           substituted; the `n` binder must re-resolve against the substituted scrutinee. `apply-to`
           applied to `(C.A 9)` yields 9. Was 'parameter reference has no local slot' when the substituted
           scrutinee's pattern binder kept its pre-substitution occurrence.")
  (input  (do
            (type C (A Int64) B)
            (def (apply-to f (: c C)) (f c))
            (def (main) (apply-to (fn ((: p C)) (match p ((C.A n) n) ((C.B) 0))) (C.A 9)))
            (export main)))
  (output (: 9 Int64)))

(case "a HOF callback matching its sum argument reaches the nullary arm"
  (doc    "The companion selecting the OTHER variant: the same callback applied (through the inlined HOF)
           to `(C.B)` takes the nullary arm → 0. Pins that the through-a-HOF nested reduction dispatches
           correctly across variants, not just the payload one.")
  (input  (do
            (type C (A Int64) B)
            (def (apply-to f (: c C)) (f c))
            (def (main) (apply-to (fn ((: p C)) (match p ((C.A n) n) ((C.B) 0))) (C.B)))
            (export main)))
  (output (: 0 Int64)))

(case "a HOF callback matching a tuple argument computes through the nested reduction"
  (doc    "The same shape with a TUPLE-destructuring callback — `(fn (p) (match p ((tuple a b) (+ a b))))`
           — passed to a HOF. The tuple-pattern binders `a`/`b` read the substituted scrutinee just as a
           sum-variant binder does, so this pins the fix is over any compound-match scrutinee, not sums
           alone. `(tuple 3 4)` → 3 + 4 = 7.")
  (input  (do
            (def (apply-to f (: t (Tuple Int64 Int64))) (f t))
            (def (main) (apply-to (fn ((: p (Tuple Int64 Int64))) (match p ((tuple a b) (+ a b)))) (tuple 3 4)))
            (export main)))
  (output (: 7 Int64)))

; --- A RECURSIVE higher-order function with an UNANNOTATED function-typed parameter --------------
; core-semantics.md §A Function Is A First-Class Value: a recursive traversal takes a CALLBACK and applies
; it per element — `map`/`fold` over a recursive sum. The callback parameter `f` need not be annotated:
; its type is inferred from its USE as a call head (`(f h)` ⇒ `f : (-> typeof(h) result)`), the function
; analogue of inferring a data parameter's type from a pattern match. The recursive-parameter solve gives
; each fn-typed parameter its arrow shape before collecting constraints, so `(+ (f h) …)` flows the result
; type back to the arrow. Without it a recursive HOF's callback stayed unconstrained → the recursive-def
; guard declined "annotate its parameters"; annotating was the only recourse. These pin that the
; annotation is now optional — the recursion-over-a-sum-with-a-callback idiom compiles bare.

(case "a recursive fold over a sum list infers its unannotated callback parameter"
  (doc    "`sum-f` recurses over `(type L Nil (Cons Int64 L))`, applying an UNANNOTATED callback `f` to
           each head and summing: `(+ (f h) (sum-f f t))`. `f`'s type is inferred `(-> Int64 Int64)` from
           `(f h)` (h : Int64) and the `+` that consumes its result — no annotation on `f`. Applied with
           `(fn (x) (+ x 1))` over `[1, 2]` → (1+1) + (2+1) = 5. Was 'a recursive function with an
           unannotated parameter is not yet inferred' before fn-typed recursive params were solved.")
  (input  (do
            (type L Nil (Cons Int64 L))
            (def (sum-f f (: l L)) (match l ((L.Nil) 0) ((L.Cons h t) (+ (f h) (sum-f f t)))))
            (def (main) (sum-f (fn ((: x Int64)) (+ x 1)) (L.Cons 1 (L.Cons 2 L.Nil))))
            (export main)))
  (output (: 5 Int64)))

(case "a recursive map rebuilding a sum list infers its unannotated callback"
  (doc    "The map companion: `map-f` REBUILDS the list, applying an unannotated `f` to each element —
           `(L.Cons (f h) (map-f f t))`. `f` infers `(-> Int64 Int64)` from `(f h)` in a `Cons`-payload
           position. `(fn (x) (* x 2))` over `[3, 4]` yields `[6, 8]`; the caller reads the head → 6. Pins
           the inference works when the callback's result feeds a CONSTRUCTOR payload, not only an operator.")
  (input  (do
            (type L Nil (Cons Int64 L))
            (def (map-f f (: l L)) (match l ((L.Nil) L.Nil) ((L.Cons h t) (L.Cons (f h) (map-f f t)))))
            (def (main) (match (map-f (fn ((: x Int64)) (* x 2)) (L.Cons 3 (L.Cons 4 L.Nil))) ((L.Cons h t) h) ((L.Nil) 0)))
            (export main)))
  (output (: 6 Int64)))

(case "a recursive fold with an unannotated two-argument callback parameter"
  (doc    "The callback takes TWO arguments — `(fn (a b) (+ a b))` — and `fold` threads an accumulator:
           `(fold f (f acc h) t)`. `f` infers `(-> Int64 (-> Int64 Int64))` from the two-argument
           application `(f acc h)`, so a multi-argument callback param is inferred at its full arity, not
           just unary. `1 + 2 + 3` = 6. Pins the arrow-shaping is over the application's argument COUNT.")
  (input  (do
            (type L Nil (Cons Int64 L))
            (def (fold f (: acc Int64) (: l L)) (match l ((L.Nil) acc) ((L.Cons h t) (fold f (f acc h) t))))
            (def (main) (fold (fn ((: a Int64) (: b Int64)) (+ a b)) 0 (L.Cons 1 (L.Cons 2 (L.Cons 3 L.Nil)))))
            (export main)))
  (output (: 6 Int64)))

(case "a recursive HOF infers a callback whose RESULT is a sum matched in the body"
  (doc    "The callback's RESULT type is inferred too, not only its parameter: `find` applies an
           unannotated `f` and MATCHES its result — `(match (f h) ((C.A n) …) ((C.B) …))`. The `C.A`/`C.B`
           arm patterns pin `f`'s result to the sum `C`, so `f : (-> Int64 C)` with no annotation. `find`
           returns the first element for which `f` yields `C.A`: over `[0, 5]` with `f x = (if (> x 1) (C.A
           x) (C.B))`, element 5 gives `(C.A 5)` → 5. Pins that a fn-param's result is solved from a match
           on its application, the result-side companion of inferring the parameter from `(f h)`.")
  (input  (do
            (type L Nil (Cons Int64 L))
            (type C (A Int64) B)
            (def (find f (: l L))
              (match l
                ((L.Nil) (C.B))
                ((L.Cons h t) (match (f h) ((C.A n) (C.A n)) ((C.B) (find f t))))))
            (def (main) (match (find (fn ((: x Int64)) (if (> x 1) (C.A x) (C.B))) (L.Cons 0 (L.Cons 5 L.Nil)))
                          ((C.A n) n) ((C.B) 0)))
            (export main)))
  (output (: 5 Int64)))

(case "a branching recursive tree fold infers its unannotated callback across both arms"
  (doc    "A tree `(type T (Leaf Int64) (Node (Tuple T T)))` folded by an unannotated callback `f` with
           BRANCHING recursion — the `Node` arm makes TWO self-calls `(+ (fold-t f l) (fold-t f r))`. The
           `Leaf` arm returns `(f n)` DIRECTLY, so `f`'s result type is fixed only by the arms agreeing:
           the `Node` arm is Int64, so the `Leaf` arm — hence `f`'s result — is Int64. Pins that the
           arms-agree constraint reaches a fn-param's result var when an arm body is a bare callback
           application, the branching-recursion companion of the single-recursion fold. `(1 + 2) · 10`
           applied per leaf → 10 + 20 = 30.")
  (input  (do
            (type T (Leaf Int64) (Node (Tuple T T)))
            (def (fold-t f (: t T))
              (match t
                ((T.Leaf n) (f n))
                ((T.Node (tuple l r)) (+ (fold-t f l) (fold-t f r)))))
            (def (main) (fold-t (fn ((: x Int64)) (* x 10)) (T.Node (tuple (T.Leaf 1) (T.Leaf 2)))))
            (export main)))
  (output (: 30 Int64)))

(case "a recursive fold infers a callback applied to the RECURSIVE-CALL RESULT"
  (doc    "The callback is applied not to a payload but to the RESULT OF THE RECURSIVE CALL — `(f (foldn f
           z m))` over Peano `(type N Z (S N))`. `f`'s parameter is that recursive result and its result is
           the `S` arm's value; the arms agree (the `Z` arm returns the accumulator `z : Int64`), so `f`'s
           result — hence its whole arrow `(-> Int64 Int64)` — is inferred with no annotation on `f`. This
           is the general recursive-fold shape (fold right, applying the callback to the sub-fold), the
           companion of applying the callback to a payload element. `f = (+ x 1)` applied twice to z = 0
           → 2. (The accumulator `z` is annotated: a pure pass-through parameter has no INTERNAL constraint
           to infer from, so it is annotated, exactly as a non-callback accumulator is.)")
  (input  (do
            (type N Z (S N))
            (def (foldn f (: z Int64) (: n N))
              (match n ((N.Z) z) ((N.S m) (f (foldn f z m)))))
            (def (main) (foldn (fn ((: x Int64)) (+ x 1)) 0 (N.S (N.S (N.Z)))))
            (export main)))
  (output (: 2 Int64)))

(case "a closure capturing two enclosing bindings folds through nested arithmetic"
  (doc    "`(fn (x) (+ (* x a) b))` captures BOTH `a` and `b` from enclosing lets; applied to 5 with
           a = 2, b = 3 → (5·2)+3 = 13. Pins that MULTIPLE distinct captures from different enclosing
           `let`s are each preserved and folded through a nested arithmetic body.")
  (input  (let ((a 2) (b 3)) ((fn ((: x Int64)) (+ (* x a) b)) 5)))
  (output (: 13 Int64)))

; A closure that CAPTURES ANOTHER CLOSURE and applies it — a higher-order capture. `twice` closes over
; `inc` (itself a closure) and applies it twice; `(twice 5)` = inc(inc(5)) = 7. core-semantics.md §A
; Function Is A First-Class Value: a function value can be captured like any other. Both closures fold —
; the captured `inc` inlines at each application inside `twice`'s body.

(case "a closure captures another closure and applies it"
  (doc    "`inc = (fn (x) (+ x 1))`; `twice = (fn (y) (inc (inc y)))` captures `inc` and applies it twice;
           `(twice 5)` = inc(inc(5)) = 7. A closure captured by another closure is applied correctly —
           the captured function value folds at each use.")
  (input  (let ((inc (fn ((: x Int64)) (+ x 1))))
            (let ((twice (fn ((: y Int64)) (inc (inc y)))))
              (twice 5))))
  (output (: 7 Int64)))

(case "a closure captures another closure and applies it at RUNTIME"
  (doc    "The same higher-order capture but with a RUNTIME argument, so nothing folds: `(def (main (: n
           Int64)) …)` binds `inc = (fn (x) (+ x 1))` and `twice = (fn (y) (inc (inc y)))` (which CAPTURES
           `inc`), then `(twice n)`. With `n = 5` → inc(inc(5)) = 7, computed at run time — `twice`'s cell
           holds the captured `inc` handle, dispatched via `call_indirect` at each use. Complements the folded
           case above: the captured closure value survives on the heap and is applied without inlining.")
  (input  (do (def (main (: n Int64))
                (let ((inc (fn ((: x Int64)) (+ x 1))))
                  (let ((twice (fn ((: y Int64)) (inc (inc y)))))
                    (twice n))))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 7 Int64)))

(case "a factory RETURNS a closure that captures a let-bound inner closure"
  (doc    "`(def (mk (: k Int64)) (let ((g (fn (y) (+ y k)))) (fn (x) (g x))))` — `mk` binds an inner closure
           `g` (capturing `k`), then RETURNS an outer closure that captures `g`. `((mk 10) n)` with n = 5 →
           the returned closure applies `g` to 5 = 5 + 10 = 15, at runtime. Pins a returned closure capturing
           a LET-bound closure (a two-level capture: the outer holds `g`, `g` holds `k`).")
  (input  (do (def (mk (: k Int64)) (let ((g (fn ((: y Int64)) (+ y k)))) (fn ((: x Int64)) (g x))))
              (def (main (: n Int64)) ((mk 10) n))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 15 Int64)))

; A returned lambda capturing the def's SCALAR parameter (the C-HOST-2 make-forwarding shape at the def
; level): the scalar argument substitutes cleanly into the returned lambda's cell.

(case "a factory RETURNS a closure capturing the def's SCALAR parameter"
  (doc    "`(def (mk (: k Int64)) (fn (x) (+ x k)))` — the returned closure captures the def's SCALAR param
           `k`. Applied `((mk 10) n)` with n = 5 → 5 + 10 = 15. The scalar argument `10` substitutes cleanly
           into the returned lambda's cell.")
  (input  (do (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (main (: n Int64)) ((mk 10) n))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 15 Int64)))

; A nested lambda capturing a closure-typed DEF PARAMETER now works too, including when the def is applied to
; an INLINE lambda argument. The fix (`eval::apply_lambda`): a lambda ARGUMENT is pinned by its FREE variables
; only (`pin_free_vars`, excluding the arg lambda's own params) rather than by a blunt whole-subtree
; `resolve_subtree` — so its own-param body references stay unpinned and re-substitute when the arg lambda is
; later applied inside the returned lambda that lifts (previously they dangled as slot-less `Core::Param`, the
; "parameter reference has no local slot" decline). A def-ref or a let-bound lambda already worked; this
; brings the INLINE lambda argument to parity.

(case "a nested lambda captures+applies a closure-typed def PARAMETER (inline lambda argument)"
  (doc    "`(def (mk (: g (-> Int64 Int64))) (fn (x) (g x)))` returns a closure that captures the def's
           CLOSURE-typed parameter `g`, applied to an INLINE lambda `((mk (fn (y) (+ y 1))) n)`. The returned
           lambda captures `g` (= the arg lambda) and dispatches it; with n = 5 → `(fn y -> y+1)` applied to 5
           = 6. The arg lambda's own param `y` re-substitutes correctly inside the lifted returned body (the
           free-vars-only pinning fix). A higher-order (closure-arg) FACTORY at runtime.")
  (input  (do (def (mk (: g (-> Int64 Int64))) (fn ((: x Int64)) (g x)))
              (def (main (: n Int64)) ((mk (fn (y) (+ y 1))) n))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64)))

; The same factory with the closure argument supplied three OTHER ways — all equivalent now: a TOP-LEVEL def
; (a global ref), and (below) a LET-bound lambda. These already worked before the inline-arg fix; kept as
; coverage that the closure-arg factory is uniform across argument spellings.

(case "a returned lambda captures+applies a closure param bound to a TOP-LEVEL def"
  (doc    "The same `(def (mk (: g (-> Int64 Int64))) (fn (x) (g x)))` returning a closure that captures its
           closure param `g` — but here `g`'s argument is a TOP-LEVEL def `inc`, not an inline lambda.
           `((mk inc) n)` with n = 5 → the returned closure applies `inc` to 5 = 6. Works: a def reference is a
           global (re-resolves by name, no pinned own-param), so it captures + dispatches cleanly — isolating
           the decline above to the INLINE-lambda argument specifically.")
  (input  (do (def (inc (: y Int64)) (+ y 1))
              (def (mk (: g (-> Int64 Int64))) (fn ((: x Int64)) (g x)))
              (def (main (: n Int64)) ((mk inc) n))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64)))

; The third argument spelling: a LET-bound lambda. Equivalent to the inline and top-level-def forms above —
; all three now capture + dispatch the closure argument through the returned lambda cleanly.

(case "a let-bound lambda passed to a returned-closure factory"
  (doc    "The SAME `(def (mk (: g (-> Int64 Int64))) (fn (x) (g x)))` returned-closure factory, with the
           lambda argument LET-BOUND first: `(let ((f (fn (y) (+ y 1)))) ((mk f) n))`. `main(5)` → the returned
           closure applies `f` to 5 = 6. Equivalent to the inline and def-ref argument spellings above.")
  (input  (do (def (mk (: g (-> Int64 Int64))) (fn ((: x Int64)) (g x)))
              (def (main (: n Int64)) (let ((f (fn ((: y Int64)) (+ y 1)))) ((mk f) n)))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64)))

(case "a closure argument is another closure's result"
  (doc    "The argument to one closure is the result of applying another: `((fn (x) (+ x k)) ((fn (y)
           (* y 2)) 3))` with k = 10 → (fn x)(6) = 16. Composing two closure applications — the inner
           `(* 3 2) = 6` feeds the outer `(+ 6 10) = 16` — both fold.")
  (input  (let ((k 10))
            ((fn ((: x Int64)) (+ x k)) ((fn ((: y Int64)) (* y 2)) 3))))
  (output (: 16 Int64)))

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

; The COMMUTING CONVERSION also applies to a `match` head, not only an `if`: `((match c (p0 f0) (p1 f1)…)
; args…)` pushes the application into each ARM body → `(match c (p0 (f0 args…)) (p1 (f1 args…))…)` (a
; "case-of-match", the sum analogue of case-of-case). A match whose arms return CLOSURES — the dispatch-
; table idiom `(match c ((C.A n) (fn (x) …)) …)` — then folds each arm's lambda in place, INCLUDING one
; that CAPTURES the arm's payload binder (`(fn (x) (+ x n))`), because the arm's pattern is reused so `n`
; stays in scope for the rewritten body. Sound: only the taken arm runs, so applying in that arm is what
; the original did.

(case "applying the result of a match whose arms return payload-capturing closures"
  (doc    "A `match` selects a closure per variant and the result is applied: `((mk (C.A 10)) 5)` where
           `mk` returns `(fn (x) (+ x n))` from the `C.A n` arm — the closure CAPTURES the arm's payload
           `n`. The application pushes into each arm (case-of-match), and the `C.A` arm's lambda folds
           against `5` with `n` = 10 → 15. Was 'value is not applyable' (a match result was not recognized
           as an applyable head — only an `if` head commuted); now the match head commutes like an `if`.")
  (input  (do
            (type C (A Int64) B)
            (def (mk (: c C)) (match c ((C.A n) (fn ((: x Int64)) (+ x n))) ((C.B) (fn ((: x Int64)) x))))
            (def (main) ((mk (C.A 10)) 5))
            (export main)))
  (output (: 15 Int64)))

(case "a match-of-closures on a runtime-selected variant is applied per arm"
  (doc    "The runtime companion: the scrutinee is a runtime-selected variant `(if b (C.A 10) (C.B))`, so
           WHICH closure `mk` returns is decided at run time; applying `((mk …) 5)` dispatches to the taken
           arm's closure. b=true → the `C.A 10` arm → `(+ 5 10)` = 15; b=false → the `C.B` identity arm → 5.
           Pins the case-of-match commuting conversion over a runtime scrutinee, not only a constant one.")
  (input  (do
            (type C (A Int64) B)
            (def (mk (: c C)) (match c ((C.A n) (fn ((: x Int64)) (+ x n))) ((C.B) (fn ((: x Int64)) x))))
            (def (main (: b Bool)) ((mk (if b (C.A 10) (C.B))) 5))
            (export main)))
  (call   main (: true Bool))
  (output (: 15 Int64))
  (call   main (: false Bool))
  (output (: 5 Int64)))

(case "a match returning multi-argument closures applies at full arity"
  (doc    "The arms return TWO-argument closures — `(fn (x y) (+ (+ x y) n))` — and the result is applied
           to both args at once: `((mk (C.A 100)) 3 4)`. Case-of-match pushes the full multi-argument
           application into each arm, so the taken arm's lambda folds against `[3, 4]` with `n` = 100 →
           107. Pins the commuting conversion carries ALL arguments, not just one.")
  (input  (do
            (type C (A Int64) B)
            (def (mk (: c C)) (match c ((C.A n) (fn ((: x Int64) (: y Int64)) (+ (+ x y) n))) ((C.B) (fn ((: x Int64) (: y Int64)) (+ x y)))))
            (def (main) ((mk (C.A 100)) 3 4))
            (export main)))
  (output (: 107 Int64)))

; A function stored in a RECORD FIELD, where that record is a SUM's payload, and CALLED after a match
; binds the payload — `(match h ((H.M rec) ((. rec f) x)))`. The projected `(. rec f)` reads a fn value
; off a RUNTIME record (the payload survives the match as a heap value, so it does not fold to the
; lambda), so it must apply via `call_indirect` like any runtime closure. This was declined "value is
; not applyable" — a record-field projection was not recognized as a runtime function-value head the way
; a tuple-element projection (`Proj`) or a payload binder (`SumPayload`) already were. Pins that a fn
; reached through a record field of a sum payload is a first-class callable (the record-field analogue of
; the closure-in-a-sum-payload case above), while a DATA field read and a `(. Sum Variant)` constructor —
; both also member projections — keep their own paths.

(case "a function stored in a record field of a sum payload is called after a match"
  (doc    "`(type H (M (Record (f (-> Int64 Int64)) (n Int64))))` carries a record with a FUNCTION field
           `f` and a data field `n`. Matching binds the whole record to `rec`; `((. rec f) rec.n)` projects
           the fn field off the runtime payload record and applies it to the data field — `(fn (x) (+ x 1))`
           applied to 41 → 42. Pins that a fn projected from a record that is a sum payload dispatches via
           call_indirect (it cannot fold — the record is a runtime heap value behind the match), while the
           sibling `rec.n` data read folds as usual.")
  (input  (do
            (type H (M (Record (f (-> Int64 Int64)) (n Int64))))
            (def (run (: h H)) (match h ((H.M rec) ((. rec f) rec.n))))
            (def (main) (run (H.M (record (f (fn ((: x Int64)) (+ x 1))) (n 41)))))
            (export main)))
  (call   main)
  (output (: 42 Int64)))

; The runtime-condition selection above FOLDS because the chosen function is applied AT the selection
; site — `((if b f g) 5)` commutes the application into each branch, so no function value survives. But
; when the runtime-selected function is instead THREADED THROUGH A RECURSIVE HOF — chosen by `if`, then
; passed to `applyer` and applied inside the recursion — the `if` CANNOT commute into the recursive
; callee, so the selected closure must survive as a genuine runtime heap VALUE and dispatch via
; `call_indirect`. This is the runtime-selected companion to the recursive-HOF case below: the closure's
; identity is decided at run time, yet it is still applied indirectly at each recursion step.

(case "a runtime-selected closure survives as a value threaded through a recursive HOF (true branch)"
  (doc    "`(if b (fn (x) (+ x 10)) (fn (x) (* x 10)))` is selected by the runtime Bool `b`, then passed
           to the recursive `applyer` and applied at each step — the `if` cannot commute into the
           recursion, so the chosen closure is a real runtime value dispatched via call_indirect. With
           b=true the closure is `(+ x 10)`: applyer sums (3+10)+(2+10)+(1+10) = 36.")
  (input  (do
            (def (applyer (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (applyer g (- n 1)))))
            (def (main (: b Bool))
              (applyer (if b (fn ((: x Int64)) (+ x 10)) (fn ((: x Int64)) (* x 10))) 3))
            (export main)))
  (call   main (: true Bool))
  (output (: 36 Int64)))

(case "a runtime-selected closure survives as a value threaded through a recursive HOF (false branch)"
  (doc    "The false branch of the case above: with b=false the chosen closure is `(* x 10)`, so applyer
           sums (3·10)+(2·10)+(1·10) = 60. The SAME program with the other runtime input dispatches the
           other lifted closure through the same recursive indirect-call site — the table slot carried by
           the runtime-selected closure cell selects which code runs.")
  (input  (do
            (def (applyer (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (applyer g (- n 1)))))
            (def (main (: b Bool))
              (applyer (if b (fn ((: x Int64)) (+ x 10)) (fn ((: x Int64)) (* x 10))) 3))
            (export main)))
  (call   main (: false Bool))
  (output (: 60 Int64)))

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

; The same runtime closure, but capturing TWO enclosing bindings rather than one — a MULTI-SLOT
; environment. `(fn (x) (+ (+ x a) b))` closes over both `main`'s parameter `a` and the let-bound `b`,
; so the lifted closure cell must carry two captured slots, not one. Threaded through the recursive
; `apply-sum` and applied at each step, every indirect call observes both captured values. This pins
; that the closure environment generalizes past a single capture — the environment product holds an
; arbitrary number of captured slots, read back positionally in the lifted body.

(case "a closure capturing two enclosing bindings threads a multi-slot environment through a recursive HOF"
  (doc    "`(fn (x) (+ (+ x a) b))` captures BOTH `a` (main's parameter) and `b` (an enclosing `let`) —
           a two-slot closure environment, not the single capture of the case above. Passed to the
           recursive `apply-sum` and applied at each step, every application observes both captured
           values. With a=10, b=100: (3+10+100)+(2+10+100)+(1+10+100) = 336. Pins that a runtime
           closure's environment holds MORE THAN ONE captured slot, read back positionally.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main (: a Int64))
              (let ((b 100))
                (apply-sum (fn ((: x Int64)) (+ (+ x a) b)) 3)))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 336 Int64)))

; HIGHER-ORDER CAPTURE — a closure whose captured free variable is ITSELF A FUNCTION. `(fn (b) (g b))`
; closes over `g`, a fn-typed parameter of the enclosing recursive `rec`; the closure cell must store
; `g`'s closure HANDLE as a captured slot and, in the lifted body, read it back and apply it via
; `call_indirect`. `core-semantics.md` §A Function Is A First-Class Value composed with capture: a
; captured value may be any first-class value, a function included. Two subtleties this pins: because
; `rec` recurses, `g` threads through the recursive specialization as a synthesized parameter, so a
; capture whose target is that synthesized param must still be recognized (not mistaken for a global);
; and a `Ty::Fn` capture is a u32 cell handle stored/read AS-IS, like any compound handle, not boxed as
; a scalar. `rec` builds `(fn (b) (g b))`, hands it to the recursive `sumapply` (applied at 2 and 1),
; and sums over its own recursion — each level contributes g(2)+g(1).

(case "a closure captures a function value and applies it through a recursive HOF"
  (doc    "The captured free variable is a FUNCTION: `(fn (b) (g b))` closes over `g`, itself a runtime
           fn parameter, so the closure cell stores `g`'s handle and the lifted body applies it via an
           indirect call. `rec` passes that closure to the recursive `sumapply` (which applies it at 2
           and 1) and repeats over its own recursion. With `g = (fn (x) (+ x 1))`: each level is
           g(2)+g(1) = (2+1)+(1+1) = 5, and over n=3 levels the total is 15. Pins that a closure can
           capture and apply another closure — higher-order capture through a call_indirect.")
  (input  (do
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
            (def (rec (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g b)) 2) (rec g (- n 1)))))
            (def (main (: n Int64)) (rec (fn ((: x Int64)) (+ x 1)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 15 Int64)))

; NESTED CAPTURING CLOSURES — a closure captures another closure that ITSELF captures. `g = (fn (x) (f
; (+ x 1)))` captures `f`, and `f = (fn (y) (+ y k))` captures `k`. Inside `g`'s lifted body, `f` is a
; runtime closure HANDLE read from `g`'s env cell — NOT the compile-time lambda it was defined from — so
; `(f …)` must apply via `call_indirect` (which threads `f`'s OWN env, carrying `k`), not β-reduce to the
; original definition. This pins that a captured value that happens to be a function is applied as a
; runtime closure (its own environment preserved), rather than followed back to its definition and folded.

(case "a closure captures a capturing closure and calls it through a recursive HOF"
  (doc    "`g = (fn (x) (f (+ x 1)))` captures `f`, itself the capturing closure `(fn (y) (+ y k))` over
           `k`. Inside `g`'s lifted body `f` is a runtime handle applied via an indirect call that threads
           `f`'s own env (carrying `k`), not the original lambda. `ap g 2` = g(2)+g(1) = (2+1+k)+(1+1+k);
           with k=100 that is 103+102 = 205. Pins nested capturing closures — a captured function is
           called as a runtime closure with its own environment intact.")
  (input  (do
            (def (ap (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
            (def (main (: k Int64))
              (let ((f (fn ((: y Int64)) (+ y k))))
                (ap (fn ((: x Int64)) (f (+ x 1))) 2)))
            (export main)))
  (call   main (: 100 Int64))
  (output (: 205 Int64)))

; A NESTED LAMBDA inside a lifted closure body. `g = (fn (x) ((fn (y) (+ y k)) x))` is a runtime closure
; (passed to the recursive `ap`) whose body applies an inner lambda `(fn (y) (+ y k))` in place. The inner
; application must β-REDUCE during lowering — `((fn (y) (+ y k)) x)` → `(+ x k)` — so the lifted body is a
; simple capturing closure over `k`, NOT a body carrying an un-lowered nested lambda. (Analyzing the outer
; body must descend a nested lambda with its OWN params excluded — the inner `y` is bound locally, neither
; a capture of the outer nor a self-reference — so the nested lambda does not spuriously decline the lift.)
; `ap g 2` with k=10 = (2+10)+(1+10) = 23.

(case "a closure whose body applies a nested lambda in place runs through a recursive HOF"
  (doc    "`(fn (x) ((fn (y) (+ y k)) x))` is a runtime closure over `k` whose body applies an inner
           lambda to `x`; the inner application β-reduces to `(+ x k)` during lowering, so the lifted body
           is a plain capturing closure. `ap g 2` with k=10 = (2+10)+(1+10) = 23. Pins that a nested lambda
           inside a lifted closure body reduces rather than declining the lift.")
  (input  (do
            (def (ap (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
            (def (main (: k Int64))
              (ap (fn ((: x Int64)) ((fn ((: y Int64)) (+ y k)) x)) 2))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 23 Int64)))

; A runtime closure whose body CALLS A RECURSIVE TOP-LEVEL FUNCTION. `(fn (x) (fact x))` is lifted (it is
; passed to the recursive `ap`, so it cannot fold), and its body invokes the recursive `fact` — a
; `Core::Call` to a standalone wasm function nested inside a `call_indirect`ed closure body. This is the
; canonical "map a recursive function over a structure" shape a real compiler needs: the closure survives
; as a runtime value AND its body drives an ordinary recursive call. `ap (fn (x) (fact x)) 3` sums
; fact(3)+fact(2)+fact(1) = 6+2+1 = 9.

(case "a runtime closure whose body calls a recursive top-level function"
  (doc    "`(fn (x) (fact x))` is a runtime closure (passed to the recursive `ap`) whose body calls the
           recursive `fact`. The lifted closure body holds a `Core::Call` to `fact` — a recursive wasm
           function invoked from inside a call_indirect'd closure. `ap g 3` = fact(3)+fact(2)+fact(1) =
           6+2+1 = 9. Pins that a lifted closure's body can drive an ordinary recursive call.")
  (input  (do
            (def (fact (: m Int64)) (if (= m 0) 1 (* m (fact (- m 1)))))
            (def (ap (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
            (def (main (: n Int64)) (ap (fn ((: x Int64)) (fact x)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 9 Int64)))

; A runtime closure that COMPARES its argument to a CAPTURED value in an `if`. `(fn (x) (if (= x k) 1 0))`
; captures `k` and branches on `x == k` — the captured `k` feeds a comparison whose boolean drives an `if`
; inside the lifted body. `ap g 3` with k=2 counts how many of 3,2,1 equal 2, weighted 1 each = 1.

(case "a runtime closure compares its argument to a captured value in a branch"
  (doc    "`(fn (x) (if (= x k) 1 0))` captures `k` and compares its parameter against it, branching on
           the result. Through the recursive `ap` with k=2 over 3,2,1: only x=2 matches, so the sum is 1.
           Pins that a captured value drives a comparison + branch inside a lifted closure body.")
  (input  (do
            (def (ap (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
            (def (main (: k Int64)) (ap (fn ((: x Int64)) (if (= x k) 1 0)) 3))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 1 Int64)))

; MANUAL ETA-WRAP of a genuinely-RUNTIME function value. `g` is a runtime two-parameter fn PARAMETER (of
; the recursive `ap`), so it has no compile-time lambda to partially apply. Writing `(fn (b) (g n b))`
; captures `g` (a runtime closure handle) AND `n`, and applies `g` at full arity inside — the eta-wrapper
; is an ordinary capturing closure whose body is a full-arity `call_indirect` on the captured `g`. This is
; the composition of two runtime paths: an outer closure that captures a runtime fn value and CALLS it,
; passed to a second recursive HOF. Both `ap` and `sumapply` recurse, so nothing folds — the program runs
; on TWO nested indirect calls (ap→the eta-wrapper, the eta-wrapper→g). `ap g n` sums over i=n…1 of
; `sumapply((fn (b) (g i b)), 2)` = (g(i,2))+(g(i,1)) = (i+2)+(i+1) = 2i+3; for n=3: 9+7+5 = 21.

(case "a runtime function value is manually eta-wrapped and applied through nested recursive HOFs"
  (doc    "`g` is a runtime two-parameter fn parameter; `(fn (b) (g n b))` captures `g` and `n` and applies
           `g` at full arity inside — a capturing closure whose body is an indirect call on the captured
           runtime `g`. Passed to the recursive `sumapply`, itself driven by the recursive `ap`, so nothing
           folds: two nested call_indirects (ap→wrapper, wrapper→g). `ap g 3` = sum over i=3,2,1 of
           (g(i,2)+g(i,1)) = (2i+3) = 9+7+5 = 21. Pins that a genuinely-runtime fn value can be captured by
           an eta-wrapper and applied — the manual form of runtime currying, on the capture + full-arity
           machinery.")
  (input  (do
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g n b)) 2) (ap g (- n 1)))))
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 21 Int64)))

; A PREDICATE closure — a runtime closure whose RESULT TYPE is Bool. `(fn (x) (= x k))` is a `(-> Int64
; Bool)` value threaded through the recursive `anyp` ("does any i in n…1 satisfy the predicate?"), which
; SHORT-CIRCUITS on the first `true`. The closure's result crosses the `call_indirect` boundary as a
; boolean (an i32 the lifted signature returns), and drives `anyp`'s `if`. This complements the Int-result
; closures above: a lifted closure may return a Bool, and an "exists" HOF consumes it with early exit.

(case "a predicate closure returning Bool drives an early-exit recursive HOF"
  (doc    "`(fn (x) (= x k))` is a `(-> Int64 Bool)` closure over `k`; `anyp` applies it down n…1 and
           returns true on the first match (short-circuit). With k=2 over 3,2,1 the predicate holds at
           x=2, so `anyp` is true and `main` yields 100; with a k absent from 3,2,1 it is false → 0. Pins
           that a runtime closure whose RESULT is Bool applies via call_indirect and its boolean drives the
           caller's branch.")
  (input  (do
            (def (anyp (: g (-> Int64 Bool)) (: n Int64))
              (if (= n 0) false (if (g n) true (anyp g (- n 1)))))
            (def (main (: k Int64)) (if (anyp (fn ((: x Int64)) (= x k)) 3) 100 0))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 100 Int64)))

; A closure that captures a BOOLEAN. The captured value's TYPE decides the runtime op that unboxes it
; from the env cell — an integer capture reads `get-int`, a boolean reads `get-bool`. That op is emitted
; ONLY inside the LIFTED closure body, never in a top-level def, so the module's import set (which is
; walked to fix each op's import index) must include ops used only in lifted bodies — else `get-bool`
; resolves to a bogus index and the component is invalid. This case exercises a boolean capture read
; back inside the closure: `(fn (x) (if flag (* x 2) x))` closes over the boolean `flag`.

(case "a closure captures a boolean and reads it back inside its lifted body through a recursive HOF"
  (doc    "`(fn (x) (if flag (* x 2) x))` captures the boolean `flag` from `main`'s scope; the lifted
           closure body unboxes it with `get-bool` (an op used ONLY in the lifted body, so it must be
           collected into the import set from the lifted bodies, not just the top-level defs). Passed to
           the recursive `apply-sum` and applied at each step. With flag=true the closure doubles, so
           apply-sum over 3,2,1 = 6+4+2 = 12. Pins that a captured boolean round-trips through the env
           cell and that a lifted-body-only runtime op is imported.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main (: flag Bool))
              (apply-sum (fn ((: x Int64)) (if flag (* x 2) x)) 3))
            (export main)))
  (call   main (: true Bool))
  (output (: 12 Int64)))

; A closure that captures a COMPOUND value — a tuple — and projects it inside the body. The captured
; value is a u32 heap HANDLE (not a boxed scalar), stored into the env cell as-is and read back as-is;
; the projections `(. p 0)`/`(. p 1)` then index the captured tuple. This pins that a capture slot holds
; a compound handle (the tuple), distinct from a scalar capture (an int/bool boxed into the slot), and
; that reading it back and projecting it works through the recursive indirect-call boundary.

(case "a closure captures a tuple and projects it inside its lifted body through a recursive HOF"
  (doc    "`(fn (x) (+ (+ x (. p 0)) (. p 1)))` captures the tuple `p = (tuple 10 20)` — a compound heap
           handle stored in the closure's env cell as-is — and projects both elements inside the body.
           Passed to the recursive `apply-sum`: each application adds 10+20=30, so over 3,2,1 the total
           is (3+30)+(2+30)+(1+30) = 96. Pins that a captured compound (a tuple handle) round-trips
           through the env cell and its projections work at run time.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main)
              (let ((p (tuple 10 20)))
                (apply-sum (fn ((: x Int64)) (+ (+ x (. p 0)) (. p 1))) 3)))
            (export main)))
  (output (: 96 Int64)))

; A closure that captures a SUM value and MATCHES it inside the body. The captured `(Some 100)` is a sum
; handle stored in the env cell; the body's `match` reads it back and switches on its discriminant. This
; pins that a captured sum survives the env round-trip AND that a match whose scrutinee is a CAPTURED
; free variable (not a param or a local) lowers correctly inside a lifted closure body.

(case "a closure captures a sum value and matches it inside its lifted body through a recursive HOF"
  (doc    "`(fn (x) (match o ((Some v) (+ x v)) (None x)))` captures the sum `o = (Some 100)` and matches
           it in the body — the scrutinee is a CAPTURED free variable read from the env cell. Passed to
           the recursive `apply-sum`: each application takes the `Some` arm and adds 100, so over 3,2,1
           the total is (3+100)+(2+100)+(1+100) = 306. Pins that a captured sum round-trips through the
           env cell and a match over a captured scrutinee works inside a lifted closure body.")
  (input  (do
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
            (def (main)
              (let ((o (Some 100)))
                (apply-sum (fn ((: x Int64)) (match o ((Some v) (+ x v)) (None x))) 3)))
            (export main)))
  (output (: 306 Int64)))

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
; `(g i i)` for i = n…1, i.e. `2·(n + … + 1) = n·(n+1)`.

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

; A THREE-parameter runtime closure at full arity — the multi-param lift generalizes past two params.
; `(fn (a b c) …)` lifts to `(env, a, b, c) → result` and applies via one `call_indirect` with all three
; arguments. `ap3 g n` sums `(g i i i) = 3·i` for i = n…1, so with n=3 the total is 3·(3+2+1) = 18.

(case "a three-parameter closure is applied at full arity through a recursive HOF"
  (doc    "`ap3` applies its three-argument function `g` to `(g i i i)` at each recursion level and sums
           the results. `g = (fn (a b c) (+ (+ a b) c))` lifts to a three-parameter closure applied at
           full arity via one indirect call; with n=3 the sum is (3+3+3)+(2+2+2)+(1+1+1) = 18. Pins that
           the multi-parameter lift is not special-cased to two params.")
  (input  (do
            (def (ap3 (: g (-> Int64 (-> Int64 (-> Int64 Int64)))) (: n Int64))
              (if (= n 0) 0 (+ (g n n n) (ap3 g (- n 1)))))
            (def (main (: n Int64))
              (ap3 (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 18 Int64)))

; CURRIED-SYNTAX application of a runtime multi-param closure. `core-semantics.md` §Functions Are
; Single-Arity: `(fn (a b) …)` is single-arity curried sugar, so `((g n) 1)` — apply `g` to `n`, then
; apply THAT to `1` — is the SAME full-arity application as `(g n 1)`, only written with nested parens.
; When `g` is a RUNTIME fn value (a recursive HOF's parameter), the two-paren spine must flatten to one
; `call_indirect` on `g` with both arguments — NOT decline as an unbuilt intermediate closure. This is
; "runtime currying reaches full arity": the application SPINE is peeled and its arguments gathered
; left-to-right, so a curried call site behaves identically to the flat one. (A partial that never
; reaches full arity would still need a heap partial-closure cell; here every use completes the arity.)

(case "a curried-syntax application of a runtime closure flattens to one full-arity indirect call"
  (doc    "`((g n) 1)` where `g` is the recursive `ap`'s runtime two-parameter fn parameter — the curried
           spelling of `(g n 1)`. The nested application spine flattens so `g` is applied to both `n` and
           `1` in ONE indirect call; with `g = (fn (a b) (+ a b))` and n=3 the sum is (3+1)+(2+1)+(1+1) =
           9. Pins that a curried call site of a runtime closure reaches full arity via one call_indirect,
           identical to the flat form — it does not decline as an unbuilt intermediate closure.")
  (input  (do
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
              (if (= n 0) 0 (+ ((g n) 1) (ap g (- n 1)))))
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 9 Int64)))

; A PARTIAL APPLICATION that escapes short of full arity, then runs as a runtime closure. Here `g` is
; `main`'s statically-known two-parameter lambda, so `(g n)` — applied to ONE arg — PARTIALLY APPLIES at
; compile time (`core-semantics.md` §Functions Are Single-Arity: applying a curried function to fewer args
; returns a closure awaiting the rest) into a residual `(fn (b) (+ 5 b))`. That residual then escapes as a
; VALUE passed to the recursive `sumapply`, which cannot inline it — so it survives as a genuine runtime
; closure applied via `call_indirect` at each step. The partial-application fold + the runtime-closure lift
; compose: `sumapply (partial) 2 = (5+2)+(5+1) = 13`. (Pins the fix that made a partially-applied residual's
; parameter annotation survive the β-copy that carries it into the recursive callee — before it, the
; residual's awaited parameter lost its declared type and the closure declined.)

(case "a partially-applied function escapes as a value and runs through a recursive HOF"
  (doc    "`(g n)` where `g` is `main`'s two-parameter lambda applied to ONE arg partially applies to the
           residual `(fn (b) (+ 5 b))`, which escapes as a value into the recursive `sumapply` (applied at
           2 and 1) and runs as a runtime closure via call_indirect. `sumapply (g 5) 2 = (5+2)+(5+1) = 13`.
           Pins that a partial application escaping short of full arity survives as a runtime closure when
           it crosses into a recursive HOF.")
  (input  (do
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
              (sumapply (g n) 2))
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 13 Int64)))

; A closure RETURNED from a RECURSIVE function, then applied through a recursive HOF — two runtime
; function paths composed. `core-semantics.md` §A Function Is A First-Class Value lists both "returned
; as a result" and "passed as an argument"; here they meet at run time. Because `pick` is RECURSIVE it
; cannot be inlined away, so the closure it returns is a genuine runtime value (not folded at the call
; site the way a non-recursive factory folds), and it then crosses into the recursive `applyer` and
; dispatches via `call_indirect` at each step. Pins that a lifted closure produced by one runtime
; function survives being handed to another and applied indirectly.

(case "a closure returned from a recursive function is applied through a recursive HOF"
  (doc    "`pick` recurses to its base case and returns the closure `(fn (x) (+ x 1))`; because `pick`
           recurses it cannot fold, so its returned closure is a real runtime value. That value is passed
           to the recursive `applyer` and applied at each step via an indirect call. `pick n` always
           reaches `(+ x 1)`, so `applyer (pick n) 3 = (3+1)+(2+1)+(1+1) = 9` regardless of the runtime
           `n` fed to pick. Pins that a returned-then-passed runtime closure dispatches correctly.")
  (input  (do
            (def (pick (: n Int64))
              (if (= n 0) (fn ((: x Int64)) (+ x 1)) (pick (- n 1))))
            (def (applyer (: g (-> Int64 Int64)) (: n Int64))
              (if (= n 0) 0 (+ (g n) (applyer g (- n 1)))))
            (def (main (: n Int64)) (applyer (pick n) 3))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 9 Int64)))

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
  (input   ((. (record (f (fn (x) (+ x 1)))) f) 5))
  (output  (: 6 Int64)))

(case "a field is projected from a record returned by a function"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value + #Member Access Projects A
           Record Field: a function may return a record, and its caller projects a field from the
           result. `((fn (x) (record (v x))) 7)` builds the record {v: 7}; projecting `v` yields 7.
           Accessing a field inside the lambda body already works, and accessing a directly-written or
           let-bound record works — projecting the record a lambda RETURNS must behave the same, not
           trap. This is the record-builder idiom a compiler uses constantly.")
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

; Over-applying a USER FUNCTION is arity-checked the SAME way — the case the comment above references
; ("an over-applied constructor is arity-checked the same way an over-applied user function is"). A
; lambda / named def of arity N applied to more than N arguments applies the fully-consumed result
; (which is NOT a function) to the surplus — a type error (CDZ0203), never a silent argument drop.
; `((fn (x) (+ x 1)) 5 9)` desugars to `(((fn (x) (+ x 1)) 5) 9)`: `(fn (x)…) 5` = 6 (an Int64, not a
; function), applied to `9` — the apply-a-non-function error. This pins the over-applied-function half
; that the constructor cases above pin for constructors.

(case "over-applying a lambda by an extra argument is a type error"
  (doc    "`((fn (x) (+ x 1)) 5 9)` — a unary lambda applied to two arguments. Desugars to `(((fn (x)
           (+ x 1)) 5) 9)`: the inner application yields the Int64 6, and applying 6 to 9 applies a
           non-function → CDZ0203. The compiler MUST reject it, not drop the 9 and yield 6.")
  (input  (do (def (main) ((fn ((: x Int64)) (+ x 1)) 5 9)) (export main)))
  (error  CDZ0203))

(case "over-applying a named function by an extra argument is a type error"
  (doc    "The named-def companion: `(def (f x) (+ x 1))`, `(f 5 9)` applies the unary `f` to two args.
           By §Functions Are Single-Arity this desugars to `((f 5) 9)` — `(f 5)` = 6, applied to 9 is a
           non-function application → CDZ0203. Arity is checked for a named function exactly as for a
           lambda or a constructor.")
  (input  (do (def (f (: x Int64)) (+ x 1)) (def (main) (f 5 9)) (export main)))
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

(case "a self-applying term is declined at the reduction budget, not hung on"
  (doc    "`((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))` — a self-application whose argument applies itself
           — has NO normal form: each β-reduction produces a larger term. It is NOT statically recursive
           (the lambdas call a PARAMETER, not a named def, so the call-graph recursion check finds no
           cycle) and each reduction stays within the depth limit, so the depth guard alone does not stop
           it — the term roughly DOUBLES each step and the compiler's reduction/type walk would attempt an
           exponential number of reductions and appear to HANG. The evaluator bounds its TOTAL reduction
           work (`enter_reduction` counts attempts against a budget): past it the reduction DECLINES (a
           resource-limit rejection), so a non-normalizing term is a clean decline in a fraction of a
           second, never a compiler hang. The point of the case is 'never hang' — a compiler completes or
           declines on any input.")
  (input  (do (def (main) ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))) (export main)))
  (error  CDZ0999))

(case "an if-wrapped self-application is rejected in bounded time, not an inference hang"
  (doc    "`(fn v (if (v v) 1 (v v)))` applied to a copy of itself has no normal form: the self-app in the
           if CONDITION forces β-reduction, which reduces the branch's self-app, and applied to itself the
           term grows exponentially. The plain self-app declines at the reduction budget (above), but this
           if-wrapped variant HUNG type INFERENCE through a DIFFERENT path — the lambda-parameter context
           recovery (`expected_arrow_for_lambda` → `type_of` → …) re-derives the growing term's types
           without going through the β-reduction budget, so it stayed within the descent-depth limit while
           attempting an exponential number of context lookups. Charging that recovery against the SAME
           cumulative work budget makes it terminate: inference gives up the context hint past the budget,
           and the program is REJECTED in a fraction of a second (the self-app's Int64 result used as an if
           condition is CDZ0203 'if condition must be Bool'). The point is 'never hang' — a compiler
           completes or declines on any input, regardless of the syntactic form the divergence hides in.")
  (input  (do (def (main) ((fn (v0) (if (v0 v0) 1 (v0 v0))) (fn (v2) (if (v2 v2) 1 (v2 v2))))) (export main)))
  (error  CDZ0203))

(case "a tuple-wrapped self-application is rejected in bounded time, not a compiler stack overflow"
  (doc    "`(fn v (tuple (v v) 1))` applied to a copy of itself has no normal form: the self-app `(v v)` in
           a tuple slot grows the term exponentially. Here the reduction BUDGET already terminates inference
           (β-reduction gives up past the work budget) — but that leaves a MEMOIZED core chain thousands of
           nodes deep, `Tuple[Tuple[…poison…, 1], 1]`, bottoming out in the reduction-bound poison. That
           chain is built bottom-up at shallow demand depths, so lowering's own descent guard never fires on
           it; the REACHED-POISON walk (`collect_reached_poisons`, which reports a provable trap that a
           program unconditionally reaches) then descended the whole pre-built chain in ONE native recursion
           and OVERFLOWED THE COMPILER'S STACK — a process abort on a small valid-to-parse program. Giving
           that walk the same recursive-descent depth guard lowering has makes it surface the reduction-bound
           poison (CDZ0999) past the limit instead of crashing. The guard sits at the walk's single recursive
           entry and the walk dispatches structurally, so the whole compound-construction class (a self-app
           in a tuple / record / list / sum / map / set slot) is covered by ONE guard — not one syntactic
           wrapper at a time. The point is 'never crash' — a compiler completes or declines on any input,
           regardless of the syntactic form the divergence hides in.")
  (input  (do (def (main) ((fn (v0) (tuple (v0 v0) 1)) (fn (v2) (tuple (v2 v2) 1)))) (export main)))
  (error  CDZ0999))

(case "a sum-payload-wrapped self-application is rejected in bounded time, not a compiler stack overflow"
  (doc    "The SUM-CONSTRUCTOR-payload sibling of the tuple-wrapped case above: `(fn v (Some (v v)))` applied
           to a copy of itself. `cdz check` (inference) already declines CDZ0999 (the reduction work budget),
           but `cdz compile` HUNG at a later phase — the LAYOUT reachability walks (`collect_call_callees` /
           `collect_closure_codes`) descend a `Core::SumNew` payload by calling `core_of`, which β-reduces one
           more level per call WITHOUT holding the reduction-DEPTH guard (unlike tuple lowering), so the walk
           materializes an unbounded `Core::SumNew` chain and descends it in ONE native recursion until the
           stack OVERFLOWS. The tuple/record/list walks were bounded earlier; this bounds the sum path too, by
           a DEDICATED walk-depth counter (kept separate from `core_of`'s descent counter, which the walk also
           drives — sharing would spuriously decline a valid moderately-deep program). Past the limit the walk
           stops descending and `collect_faults` reports the coded CDZ0999. Also `(Ok (v v))` and a user
           multi-payload `(P (v v) 1)`. The point is 'never crash' — a compiler completes or declines on any
           input from BOTH check and compile, regardless of the compound the divergence hides in.")
  (input  (do (def (main) ((fn (v0) (Some (v0 v0))) (fn (v2) (Some (v2 v2))))) (export main)))
  (error  CDZ0999))

(case "a deeply nested constant expression compiles or declines without crashing"
  (doc    "A 64-deep nest of `(+ 1 …)` folds to 65 — well within any reasonable bound. The point is the
           companion the gate cannot record: the SAME shape thousands deep must DECLINE (a
           recursion/resource-limit rejection) rather than overflow the compiler's stack and abort. This
           anchors the shallow end; the compiler bounds its own recursive descent and declines when the
           bound is reached, so a pathological depth is a decline, never a process crash.")
  (input  (do (def (main) (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 (+ 1 1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))) (export main)))
  (output (: 65 Int64)))

(case "a deeply nested expression is diagnosed by the parser, never crashes it"
  (doc    "The PARSER's recursive descent (both the s-expr reader and the ML Pratt parser) must return a
           clean diagnostic on pathologically deep nesting, not overflow the native stack and abort the
           process (SIGABRT). The COMPILER already guards this — the case above declines a deep nest at
           the descent-depth bound — but the parser, which runs FIRST on any source-ingesting path
           (`convert`/`check`/`fix`, and critically the guide's `cdz-wasm` on untrusted browser input at
           a ~1MB stack), had no equivalent limit: a depth ≳25000 source crashed with 'thread main has
           overflowed its stack' where `cdz compile` on the same shape cleanly rejects. Both readers now
           carry a nesting-depth guard (mirroring the compiler's limit) that returns a parse error past
           the bound. This small depth-8 witness parses and evaluates fine (=> 9), pinning the SHAPE; the
           crash needs a depth-25000 generator, impractical to inline. Fix: a parse-time depth guard, the
           read-side analogue of the compiler's descent-depth limit.")
  (input  (do (def (main) (+ (+ (+ (+ (+ (+ (+ (+ 1 1) 1) 1) 1) 1) 1) 1) 1)) (export main)))
  (output (: 9 Int64)))

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

; The FAULT WALK over a nested call chain must be LINEAR too, not just the reduction. `type_errors`
; checks each call at its site AND collects the reduced body — and it separately descended each raw
; ARGUMENT for its own faults. On a chain `(f (f … (f 0)))` (where each argument IS the next call) that
; per-level argument descent RE-WALKED the whole remaining chain, and — because a resource-limit-clipped
; walk is not cached — restarted from scratch at every enclosing level, so REACHING the answer was O(N³)
; (a depth-30 chain folded in ms, but a deeper one took seconds→minutes just to decline). The redundant
; descent is dropped for a lambda head whose parameter the body USES (its argument is already in the
; reduced body); only a DEAD argument the body ignores is still descended (its faults are not otherwise
; seen). This case folds a chain at the deepest value-producing depth (just under the inliner's reduce
; limit), exercising the now-linear fault walk near the boundary; a deeper chain is a clean resource-limit
; DECLINE, reached in linear time rather than a hang.

(case "a deeper nested call chain still folds in linear time near the inliner limit"
  (doc    "A depth-30 chain of the incrementing `f` — near the inliner's reduce limit, the deepest that
           still folds to a value: 0, then +1 thirty times = 30. The reduction was already memoized and
           linear, but the FAULT WALK re-descended each raw argument, which on a call chain re-walked the
           remaining chain per level — cubic to reach the answer. Dropping that redundant descent for a
           used parameter, whose argument is already in the reduced body, makes the whole compile linear.
           Pins the fold at a depth the cubic fault walk handled only slowly; a deeper chain declines
           cleanly at a resource limit rather than hanging.")
  (input  (do
            (def (f n) (+ n 1))
            (def (main)
              (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f 0)))))))))))))))))))))))))))))))
            (export main)))
  (call   main)
  (output (: 30 Int64)))

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

(case "a linear non-tail recursion over a non-associative operator preserves its exact result"
  (doc    "A compiler may turn a LINEAR non-tail recursion — one self-call whose result feeds a single
           enclosing operation — into an accumulator TAIL LOOP (accumulator introduction), so deep
           recursion runs in constant stack. That rewrite must preserve the EXACT result, including for a
           NON-ASSOCIATIVE operator where the evaluation ORDER matters. `(alt n) = n - (alt (n-1))`, base
           `(alt 0) = 0`, is right-nested subtraction: alt(5) = 5−(4−(3−(2−(1−0)))) = 5−(4−(3−(2−1))) =
           5−(4−(3−1)) = 5−(4−2) = 5−2 = 3. A transform that naively accumulated `acc − n` left-to-right
           would give a DIFFERENT number; the loop must reproduce the right-nested value 3. Pins that
           accumulator introduction is result-preserving for a non-associative step, not only for `+`/`*`.")
  (input  (do
            (def (alt (: n Int64)) (if (= n 0) 0 (- n (alt (- n 1)))))
            (def (main (: n Int64)) (alt n))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 3 Int64)))

(case "a recursive def named a target-language keyword runs"
  (doc    "`loop` is a valid Cadenza identifier but a keyword in some backends (Rust). A RECURSIVE def named
           `loop` SURVIVES as a real function (a non-recursive one inlines away), so a backend that emits the
           source name verbatim as its function identifier would produce `fn loop(…)` — invalid in a language
           where `loop` is reserved. `loop(3)` counts down to 42. Pins that a def whose name collides with a
           target keyword is emitted as an escaped identifier (a raw identifier `r#loop` on the Rust backend;
           the wasm backend is unaffected — function names there are indices, not identifiers), so the same
           program runs on every backend. Also covers while/for/type/mut/impl/… as surviving function names.")
  (input  (do
            (def (loop (: n Int64)) (if (= n 0) 42 (loop (- n 1))))
            (def (main) (loop 3))
            (export main)))
  (call   main)
  (output (: 42 Int64)))

(case "accumulator introduction threads a transformed extra parameter through a multi-parameter recursion"
  (doc    "Accumulator introduction generalizes to a MULTI-parameter linear recursion: an extra parameter
           that is TRANSFORMED at each recursive step (not merely carried) must be threaded correctly by
           the loop. `(f n m) = if n=0 then 0 else m + (f (n-1) (m*2))` sums a geometric sequence — each
           step adds the current `m` and doubles it for the next: f(4,1) = 1 + 2 + 4 + 8 = 15 = m·(2^n − 1).
           The transform must carry `m` through the accumulator loop applying the per-step `m*2` in the
           right order; a rewrite that dropped the transformation (kept `m` constant) would compute n·m = 4,
           and one that mis-ordered the doublings would differ. Pins that a per-step-transformed threaded
           parameter is preserved by the multi-parameter accumulator loop, the multi-param extension of the
           single-parameter accumulator cases.")
  (input  (do
            (def (f (: n Int64) (: m Int64)) (if (= n 0) 0 (+ m (f (- n 1) (* m 2)))))
            (def (main (: n Int64) (: m Int64)) (f n m))
            (export main)))
  (call   main (: 4 Int64) (: 1 Int64))
  (output (: 15 Int64)))

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

; The same i32/i64 scratch-slot-aliasing family at a HIGHER local count, in a decode-loop shape the
; self-hosted compiler's reader is written in: a self-tail loop whose position advance projects BOTH
; fields of a tuple returned by a recursive helper, accumulating compound-payload sum nodes into a list.
; Over enough locals the loop function reused one slot for an i64 arithmetic temp AND an i32 heap handle
; (an invalid module, `expected i32 found i64`). Root-caused to the same slot-reservation weakness as the
; let-bound if-compound miscompile (a persistent slot must be reserved BEFORE the sub-expressions that
; float their scratch off the high-water) — the fix there cleared this too. A now-passing regression guard.

(case "a self-tail loop advancing by a tuple projection while accumulating compound-sum nodes compiles"
  (doc    "A decode loop `read-leaves` advances its position via `leaf-end`, which projects BOTH fields of
           the tuple returned by the recursive `read-varu` (`(+ (. v 1) (. v 0))`), and pushes `Ast` sum
           nodes (a type with a `(List Ast)` variant — a compound payload) into a `(List Ast)` accumulator.
           Over `b\"\\x00\\x01\\x05\"` it reads ONE leaf, an `(Ast.Int …)`, and `nc` of an `Ast.Int` is 1.
           This emitted INVALID WASM (`expected i32, found i64`) — a threshold-dependent slot-aliasing bug
           in the loop transform (one local held both an i64 arithmetic temp and the i32 handle from
           `read-varu`), the same scratch-slot family as the let-bound if-compound miscompile; the
           slot-reservation fix cleared both. Expected: 1.")
  (input  (do
            (type Ast (Int Int64) (List (List Ast)))
            (def (read-varu (: b Bytes) (: p Int64) (: a Int64) (: s Int64))
              (let ((byte (Option.expect (Bytes.at b p) "v")))
                (let ((a2 (+ a (<< (& byte 127) s))))
                  (if (= (& byte 128) 0) (tuple a2 (+ p 1)) (read-varu b (+ p 1) a2 (+ s 7))))))
            (def (read-mag (: b Bytes) (: p Int64) (: len Int64) (: acc Int64))
              (if (= len 0) acc (read-mag b (+ p 1) (- len 1) (+ (* acc 256) (Option.expect (Bytes.at b p) "m")))))
            (def (read-leaf (: b Bytes) (: pos Int64)) ((. Ast Int) (read-mag b (+ pos 1) (. (read-varu b (+ pos 1) 0 0) 0) 0)))
            (def (leaf-end (: b Bytes) (: pos Int64)) (let ((v (read-varu b (+ pos 1) 0 0))) (+ (. v 1) (. v 0))))
            (def (read-leaves (: b Bytes) (: pos Int64) (: count Int64) (: acc (List Ast)))
              (if (= count 0) acc (read-leaves b (leaf-end b pos) (- count 1) (List.push acc (read-leaf b pos)))))
            (def (nc (: n Ast)) (match n (((. Ast Int) _) 1) (((. Ast List) _) 9)))
            (def (main) (nc (Option.expect (List.at (read-leaves b"\x00\x01\x05" 0 1 (list)) 0) "at")))
            (export main)))
  (output (: 1 Int64)))

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

; A function's RETURN TYPE is declared by ascribing its body: `(def (f …) (: body R))` constrains the
; result to `R` exactly as a parameter binder `(: name T)` constrains a parameter and a value annotation
; `(: expr T)` constrains an expression (type-system.md §Annotations Constrain, Never Contradict). The
; ML surface writes this as `def f(x) -> R = body` (and `fn(x) -> R => body`), which desugars to this
; body ascription — no dedicated return-type node; the arrow is surface sugar over the annotation the
; cases below pin. A return type that AGREES with the body is transparent (the case below); one that
; CONTRADICTS the body's inferred type is rejected (CDZ0203), the result-position companion of the
; parameter-annotation-contradiction case above.

(case "a function's return type ascription agreeing with the body is transparent"
  (doc    "`(def (add (: x Int64) (: y Int64)) (: (+ x y) Int64))` declares the result type by ascribing
           the body `(+ x y)` to `Int64` — the desugaring of the ML `def add(x: Int64, y: Int64) -> Int64
           = x + y`. The ascription agrees with the body's inferred Int64, so it is transparent and the
           function computes normally: `(add 20 22)` = 42. Pins that a return-type annotation constrains
           without changing a well-typed result — the result-position analogue of a matching parameter or
           value annotation.")
  (input  (do
            (def (add (: x Int64) (: y Int64)) (: (+ x y) Int64))
            (def (main)                        (add 20 22)) (export main)))
  (output (: 42 Int64)))

(case "a function's return type contradicting the body is rejected"
  (doc    "`(def (f (: x Int64)) (: (+ x 1) Bool))` declares the return type `Bool` by ascribing the body,
           but `(+ x 1)` is Int64 — the declared result and the inferred result disagree, so the program
           is rejected (CDZ0203), exactly as a contradicting parameter or value annotation is. This is the
           desugaring of the ML `def f(x: Int64) -> Bool = x + 1`: a return-type annotation is an ordinary
           body ascription, and a return type that contradicts the body cannot be reconciled. The
           result-position companion of the parameter-annotation-contradiction case above.")
  (input  (do
            (def (f (: x Int64)) (: (+ x 1) Bool))
            (def (main)          (f 5)) (export main)))
  (error  CDZ0203))

(case "a lambda's return type ascription agreeing with the body is transparent"
  (doc    "The lambda companion: `(fn (x) (: (* x 2) Int64))` ascribes the lambda body to `Int64` — the
           desugaring of `fn(x) -> Int64 => x * 2`. The ascription agrees with the body, so applying the
           lambda computes normally: `((fn (x) (: (* x 2) Int64)) 21)` = 42. A lambda's return type is a
           body ascription exactly as a named def's is.")
  (input  ((fn (x) (: (* x 2) Int64)) 21))
  (output (: 42 Int64)))

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
; The seed realizes a parameterized export, because a compiler authored in
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
; boundary lift on the parameter side and the emitted narrow (i32-slot, range-checked) operation. The
; seed realizes the aliased widths' boundary forms.

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

; A CONSTANT ARGUMENT passed to a NARROW-typed parameter must be RANGE-CHECKED against the parameter's
; declared width, exactly as a direct annotation `(: 200 Int8)` is (→ CDZ0302). β-reduction substitutes
; the argument for the parameter, and the parameter's annotation `(: a T)` constrains what its argument
; may be — so an out-of-range constant is rejected, not laundered into a narrow type and run to a value
; the type cannot hold. This is enforced by carrying the annotation onto the substituted argument
; (`(: arg T)`), so the same fit-check fires on `def`, `fn`, curried, and let-binder substitution alike.
; (An IN-range constant round-trips unchanged; a RUNTIME argument keeps its own already-checked type.)

(case "a constant argument out of a narrow parameter's range is rejected, not laundered"
  (doc    "`(def (f (: a Int8)) a)` returns its Int8 parameter; `(f 200)` passes 200, OUT of Int8's
           -128..127 range. The parameter annotation constrains the argument exactly as a direct
           `(: 200 Int8)` does — CDZ0302. Without the check the constant is β-reduced into the body with
           its annotation discarded and the program runs to 200, a value no Int8 can hold (the boundary is
           sharp: `(f 127)` gives 127, `(f 128)` would wrongly give 128).")
  (input  (do (def (f (: a Int8)) a) (def (main) (f 200)) (export main)))
  (error  CDZ0302))

(case "a negative constant argument to an unsigned parameter is rejected, not laundered"
  (doc    "`(def (f (: a UInt8)) a)` with `(f -1)`: a UInt8 has no negative representation (0..255), so a
           direct `(: -1 UInt8)` rejects CDZ0302 and the parameter path must too. The unsigned case is the
           sharpest witness — not a wrap-around near a boundary but a sign the type does not have.")
  (input  (do (def (f (: a UInt8)) a) (def (main) (f -1)) (export main)))
  (error  CDZ0302))

(case "a narrow-body arithmetic on an in-range constant arg overflows the parameter width"
  (doc    "`(def (f (: a Int8)) (+ a a))` with `(f 100)`: 100 IS in Int8 range, but `(+ a a)` = 200
           OVERFLOWS Int8 (max 127). The argument carries the Int8 annotation into the body, so the
           addition is a homogeneous Int8 op whose CONSTANT operands fold and the compiler proves the
           overflow at compile time — a constant OPERATION with no value → CDZ0304 (ConstTrap), exactly
           as `(+ (: 100 Int8) (: 100 Int8))` and the wide `(+ Int64.max 1)` do. Pins that the width
           constraint is not dropped by inlining: the wide 200 is never kept as the result.")
  (input  (do (def (f (: a Int8)) (+ a a)) (def (main) (f 100)) (export main)))
  (error  CDZ0304))

(case "an in-range constant argument to a narrow parameter computes at the parameter width"
  (doc    "The complement — the check does NOT over-reject. `(def (f (: a Int8)) (+ a 10))` with `(f 100)`
           = 110, which fits Int8, so it computes normally at the Int8 width. Pins that carrying the
           annotation onto the argument range-checks WITHOUT breaking a legitimate in-range call.")
  (input  (do (def (f (: a Int8)) (+ a 10)) (def (main) (f 100)) (export main)))
  (output (: 110 Int8)))

(case "an annotated let binder range-checks its narrow-width bound value"
  (doc    "`(let (((: a Int8) 200)) a)` — the annotated let binder `(: a Int8)` constrains the bound
           value's TYPE (a `(let (((: a Bool) 5)) …)` correctly rejects CDZ0203) AND range-checks the
           narrow-width value: 200 is out of Int8's -128..127 range, so — exactly as the value annotation
           `(let ((a (: 200 Int8))) a)` gives CDZ0302 — the binder-annotation form does too. A binder
           annotation applies its type's fit-check to the bound value, like a value annotation.")
  (input  (do (def (main) (let (((: a Int8) 200)) a)) (export main)))
  (error  CDZ0302))

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
; folded cases never reach. The seed realizes `wrap` for the aliased widths.

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

; ── The pipeline operator `|>` threads a value into a function ───────────────────────────────────────
; `|>` is a REAL operator (arena head `|>`), not surface sugar: it round-trips through both syntaxes and
; the resolver rewrites `(|> L R)` into an ordinary application, threading `L` as `R`'s FIRST argument —
; `(|> x f)` = `(f x)`, and `(|> x (f a))` = `(f x a)`. Because the rewrite yields a plain application,
; the value flows through the same typing, folding, and emission as a written-out call; the two forms are
; INDISTINGUISHABLE downstream. Threading first (not last) matches the collection-first argument order of
; the built-in operations (`(List.map xs f)`), so `(|> xs (List.map f))` reads as "xs, mapped by f".
; `|>` binds looser than every operator but ascription and is left-associative, so a chain reads left to
; right: `(|> (|> x f) g)` = `g(f(x))`.

(case "the pipeline operator threads a value into a named function"
  (doc    "`(|> 5 double)` resolves to the application `(double 5)`: the piped value becomes the sole
           argument. `|>` is the pipeline operator — a real form the resolver rewrites into an ordinary
           application, so the value is typed and folded exactly as a written-out `(double 5)` is.")
  (input  (do (def (double n) (* n 2)) (def (main) (|> 5 double)) (export main)))
  (output (: 10 Int64)))

(case "the pipeline operator splices the value as a call's first argument"
  (doc    "`(|> 3 (add 10))` resolves to `(add 3 10)`: when the right operand is already an application,
           the piped value is spliced in as its FIRST argument and the written arguments follow. This is
           the argument order that lets `(|> xs (op …))` read as an operation on `xs`.")
  (input  (do (def (add a b) (+ a b)) (def (main) (|> 3 (add 10))) (export main)))
  (output (: 13 Int64)))

(case "a pipeline chain applies its stages left to right"
  (doc    "`(|> (|> 5 double) (add 1))` = `(add (double 5) 1)` = 11. `|>` is left-associative and looser
           than the other operators, so a chain of pipes reads as a left-to-right sequence of stages —
           the value out of one stage is the value into the next.")
  (input  (do (def (double n) (* n 2)) (def (add a b) (+ a b))
              (def (main) (|> (|> 5 double) (add 1))) (export main)))
  (output (: 11 Int64)))

; RECURSIVE-GENERIC MONOMORPHIZATION — a recursive function used at more than one type is INSTANTIATED
; more than once. A non-recursive generic function already monomorphizes by inlining (β-reduction at each
; call site IS specialization); a RECURSIVE one cannot inline (it would not terminate), so it lowers to a
; real function. When such a function is GENERIC — a parameter the body only threads, never constraining
; to a concrete type — the compiler synthesizes ONE specialized copy per distinct concrete instantiation
; (`glossary.md §Monomorphization`: "concrete specializations by the same compile-time reduction … done
; before emitting a component interface because generics do not cross the boundary"). Each copy emits as
; an ordinary monomorphic function with its own machine valtypes; two calls at the SAME type share one.

(case "a recursive generic function is instantiated at two different types"
  (doc    "`loopn` counts `n` down, threading `x` UNCHANGED — so `x` is generic (the body never fixes its
           type). Called at Int64 (`(loopn 3 40)` → 40, an i64 slot) AND at String (`(loopn 2 \"hi\")` →
           \"hi\", an i32 heap handle), it is MONOMORPHIZED into two functions with distinct machine
           signatures. Before recursive-generic monomorphization the second use was rejected CDZ0203
           (`x` pinned to Int64 by the first call). `byte-len(\"hi\") = 2`, so `40 + 2 = 42`.")
  (input  (do
            (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))
            (def (main) (+ (loopn 3 40) (String.byte-len (loopn 2 "hi"))))
            (export main)))
  (output (: 42 Int64)))

(case "a recursive generic function called at one type twice shares a single instantiation"
  (doc    "The dedup companion: `loopn` called at Int64 in BOTH `(loopn 3 40)` and `(loopn 2 2)` is
           instantiated ONCE — the two calls share a single monomorphic function (keyed by the concrete
           type), not two copies. `40 + 2 = 42`. Pins that monomorphization is per-TYPE, not per-call:
           the same instantiation is reused, so a program that calls a generic recursive helper at one
           type many times emits one function for it.")
  (input  (do
            (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))
            (def (main) (+ (loopn 3 40) (loopn 2 2)))
            (export main)))
  (output (: 42 Int64)))

; TRANSITIVE recursive-generic monomorphization — a generic recursive function that CALLS another
; generic recursive function, threading its own generic parameter, is itself generic (its result type is
; the callee's, which is the threaded param's). Genericity propagates through the call graph: the inner
; `idr` is called at only ONE syntactic site (`(idr 2 y)`), yet is generic because `wrap` feeds it a
; generic value, and `wrap`'s result stays connected to its parameter so `wrap` too is generic. Both are
; then monomorphized per concrete type at the OUTERMOST call sites.

(case "a recursive generic function threading another generic is itself generic at two types"
  (doc    "`wrap` recurses on `m`, threading `y` UNCHANGED, and at its base calls a SECOND generic
           recursive function `idr` (also threading its arg). `wrap`'s result is `idr`'s result is
           `y`'s type — so `wrap` is generic in `y`, even though `idr` has a single call site. Called at
           Bool (`(wrap 1 true)`) and Int64 (`(wrap 2 40)`), BOTH `wrap` and `idr` are monomorphized at
           each type (four specialized functions). Before transitive genericity, `idr`'s param pinned to
           the first type and the second use was rejected CDZ0203. `(wrap 1 true)` is true → `(wrap 2 40)`
           = 40.")
  (input  (do
            (def (idr (: n Int64) x) (if (= n 0) x (idr (- n 1) x)))
            (def (wrap (: m Int64) y) (if (= m 0) (idr 2 y) (wrap (- m 1) y)))
            (def (main) (if (wrap 1 true) (wrap 2 40) 99))
            (export main)))
  (output (: 40 Int64)))

; Recursive-generic monomorphization reaches EVERY recursive-def flavor, not just top-level defs: a
; MUTUALLY-recursive generic group and a DO-LOCAL generic function are each instantiated once per
; concrete type at their call sites, exactly as a top-level generic is. The do-local case needs the
; specialized copy's self-call to stay resolved to the original def (a do-local name resolves by lexical
; scope, which the re-parented copy escapes) — the copy SHARES the pinned self-call occurrence.

(case "a mutually-recursive generic group is instantiated per type"
  (doc    "`ping`/`pong` mutually recurse, each threading a generic second argument unchanged. Called at
           Bool (`(ping 3 true)`) and Int64 (`(pong 2 40)`), BOTH functions are monomorphized at BOTH
           types — the cross-calls re-resolve by name and re-enter specialization at the same
           instantiation. `(ping 3 true)` bounces ping→pong→ping→pong ending at the base with `true`, so
           the `if` takes `(pong 2 40)` = 40.")
  (input  (do
            (def (ping (: n Int64) x) (if (= n 0) x (pong (- n 1) x)))
            (def (pong (: n Int64) x) (if (= n 0) x (ping (- n 1) x)))
            (def (main) (if (ping 3 true) (pong 2 40) 99))
            (export main)))
  (output (: 40 Int64)))

(case "a do-local generic function is instantiated per type"
  (doc    "A do-local `(def (idr n x) …)` threading a generic `x`, called at Bool and Int64 within the
           same `do` block, is monomorphized per type. A do-local name resolves by LEXICAL do-scope, so
           the specialized copy's self-call must stay resolved to the original def (the re-parented copy
           escapes that scope) — the copy shares the pinned self-call. `(idr 1 true)` = true → `(idr 2
           40)` = 40.")
  (input  (do
            (def (main)
              (do
                (def (idr (: n Int64) x) (if (= n 0) x (idr (- n 1) x)))
                (if (idr 1 true) (idr 2 40) 99)))
            (export main)))
  (output (: 40 Int64)))

; The canonical generic-recursion idiom: a recursive function over a USER-DEFINED GENERIC RECURSIVE SUM
; type (a polymorphic linked list `(type Lst Nil (Cons a (Lst a)))`), called at more than one element
; type. `len` threads down the list's tail generically — its element type is never fixed by the body —
; so it is monomorphized once per element type at its call sites (a `Lst Int64` length and a `Lst String`
; length), exactly as a generic scalar-threading function is. This is recursive-generic monomorphization
; over the real recursive-data idiom, not just a scalar pass-through. (An explicit polymorphic annotation
; `(: l (Lst a))` is a SEPARATE not-yet-built feature — binding a type variable in a signature; here `len`
; is unannotated and inference carries the element type, which is the idiomatic form.)

(case "a recursive function over a generic recursive sum is monomorphized per element type"
  (doc    "`(type Lst Nil (Cons a (Lst a)))` is a polymorphic linked list; `len` counts its elements,
           recursing on the tail without ever constraining the element type. Called on a `Lst Int64`
           (length 2) and a `Lst String` (length 3), `len` is monomorphized into one function per element
           type — the recursive-data analogue of the scalar `loopn` case. 2 + 3 = 5.")
  (input  (do
            (type Lst Nil (Cons a (Lst a)))
            (def (len l) (match l ((Lst.Nil) 0) ((Lst.Cons h t) (+ 1 (len t)))))
            (def (main) (+ (len (Lst.Cons 1 (Lst.Cons 2 Lst.Nil)))
                           (len (Lst.Cons "a" (Lst.Cons "b" (Lst.Cons "c" Lst.Nil))))))
            (export main)))
  (output (: 5 Int64)))

; TYPE-VALUED PARAMETERS — the spec's model for a generic definition (`type-system.md §Generics Are
; Type-Valued Parameters`): a generic def takes the TYPE as an ordinary parameter (annotated `(: t Type)`,
; the kind of types), uses it as a type-constructor argument in a later parameter's annotation `(Box t)`,
; and the caller passes the concrete type as a regular argument `(unbox Int64 …)`. `t` resolves by
; ordinary lexical scope (an earlier parameter is visible in a later parameter's annotation); the type
; argument is compile-time-only and consumed by monomorphization (erased before run time), so `unbox` is
; specialized per passed type exactly as an inferred generic is. NOT implicit type variables — the type is
; a first-class value passed explicitly.

(case "a generic definition takes the type as a type-valued parameter"
  (doc    "`unbox` takes `(: t Type)` — a type-valued parameter — and `(: b (Box t))`, then unwraps the
           box. The caller passes the concrete element type as an ordinary argument: `(unbox Int64 (Box.Mk
           40))` and `(unbox String (Box.Mk \"hi\"))`. `unbox` is monomorphized per passed type (the type
           argument is compile-time-only, erased before run time). 40 + byte-len(\"hi\")=2 = 42.")
  (input  (do
            (type Box (Mk a))
            (def (unbox (: t Type) (: b (Box t))) (match b ((Box.Mk v) v)))
            (def (main) (+ (unbox Int64 (Box.Mk 40)) (String.byte-len (unbox String (Box.Mk "hi")))))
            (export main)))
  (output (: 42 Int64)))

; A RECURSIVE generic definition with a TYPE-VALUED PARAMETER — the type-valued-parameter model over the
; recursive-data idiom. `len` takes `(: t Type)` and `(: l (Lst t))` (a polymorphic linked list applied
; to the type parameter), recurses on the tail passing `t` along. Called with the concrete element type
; as an argument at Int64 and String, `len` is monomorphized per type — and because the type argument is
; compile-time-only, it is ERASED from each specialized function's signature (each `len` takes just the
; list handle, not the type) and from the recursive self-call. This is the recursive analogue of the
; `unbox` type-valued-parameter case.

(case "a recursive generic with a type-valued parameter monomorphizes per type, erasing the type argument"
  (doc    "`len` takes a type-valued `(: t Type)` and `(: l (Lst t))`, recursing on the tail with `(len t
           tl)`. Called `(len Int64 …)` over a two-element `Lst Int64` (length 2) and `(len String …)`
           over a three-element `Lst String` (length 3). `len` is monomorphized into one function per
           element type; the type argument is compile-time-only, erased from the specialized signature and
           the self-call (each `len` takes only the list handle). 2 + 3 = 5.")
  (input  (do
            (type Lst Nil (Cons a (Lst a)))
            (def (len (: t Type) (: l (Lst t)))
              (match l ((Lst.Nil) 0) ((Lst.Cons h tl) (+ 1 (len t tl)))))
            (def (main) (+ (len Int64 (Lst.Cons 1 (Lst.Cons 2 Lst.Nil)))
                           (len String (Lst.Cons "a" (Lst.Cons "b" (Lst.Cons "c" Lst.Nil))))))
            (export main)))
  (output (: 5 Int64)))

; AD-HOC POLYMORPHISM via a DICTIONARY RECORD — a record of functions passed as an ordinary argument,
; the body projecting and calling its fields. No trait resolution, no orphan rule, no coherence: it is
; just records + functions + application. A NON-recursive consumer inlines the dict (β-folds away); a
; RECURSIVE consumer is monomorphized per distinct dictionary — each field function INLINED directly (no
; `call_indirect`, no runtime record) and the dictionary argument ERASED from the emitted signature, the
; same "inline a compile-time-known argument, drop the param" rule that erases a type-valued parameter.

(case "a recursive consumer of a dictionary record inlines and erases the dictionary"
  (doc    "`fold-n` takes a dictionary `(Record (op (-> Int64 Int64)))` and applies its `op` `n` times.
           Called with `(record (op (fn (x) (+ x 10))))`, the dictionary is compile-time-known, so
           `fold-n` is monomorphized with the `op` inlined directly (`(. d op)` folds to `(+ acc 10)` —
           no call_indirect, no runtime record) and the dictionary argument erased. Folding `+10` from 0
           three times = 30.")
  (input  (do
            (def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
              (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
            (def (main) (fold-n (record (op (fn (x) (+ x 10)))) 3 0))
            (export main)))
  (output (: 30 Int64)))

(case "a dictionary consumer called at two dictionaries is monomorphized per dictionary"
  (doc    "The same `fold-n` called with TWO distinct dictionaries — `(+ x 10)` and `(* x 2)` — is
           monomorphized into two functions, each with its own `op` inlined (per-dictionary
           specialization, the ad-hoc-polymorphism analogue of per-type monomorphization). `(+10)` folded
           from 0 thrice = 30; `(*2)` folded from 1 thrice = 8; 30 + 8 = 38.")
  (input  (do
            (def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
              (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
            (def (main) (+ (fold-n (record (op (fn (x) (+ x 10)))) 3 0)
                           (fold-n (record (op (fn (x) (* x 2)))) 3 1)))
            (export main)))
  (output (: 38 Int64)))

; A `const` parameter DECLARES its argument must be compile-time-known: the compiler inlines + erases it,
; and REJECTS an argument that depends on runtime data (the author's contract, enforced). Here the dict's
; `op` captures `main`'s runtime parameter `k`, so the dictionary is NOT compile-time-known — a coded
; CDZ0201 rejection, not a silent runtime fallback.

(case "a const parameter rejects an argument that depends on runtime data"
  (doc    "`fold-n`'s dictionary parameter is `const`, so its argument must be compile-time-known. `main`
           passes `(record (op (fn (x) (+ x k))))` whose `op` captures `main`'s RUNTIME parameter `k` —
           the dictionary is not a compile-time value, violating the `const` contract. The compiler
           rejects it (CDZ0201, 'must be compile-time-known'), rather than silently passing it at runtime.
           The rejection is the program's outcome; there is no value.")
  (input  (do
            (def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
              (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
            (def (main (: k Int64)) (fold-n (record (op (fn (x) (+ x k)))) 3 0))
            (export main)))
  (error  CDZ0201))

(case "a const collection recursively folded is rejected, not compiled to an infinite loop"
  (doc    "A `const` COLLECTION parameter (here a `(List Int64)`) consumed by a SELF-RECURSIVE fold in the
           same function is REJECTED (CDZ0201) rather than compiled — because the composition of const
           erasure and the tail-loop transform would MISCOMPILE it into an infinite loop. The recursion
           `(s t …)` passes a shorter derived list `t` at each depth, but its argument node is the same
           rest-binder occurrence every time, so the specialization memo collapses all depths to ONE copy;
           the tail-loop transform then emits a `loop { … br 0 }` whose exit test (the `(list)`-nil / length
           check) was const-erased away — a valid program that HANGS. Declining is decline-don't-miscompile:
           a coded compile error beats a runtime infinite loop. The RUNTIME-list version (drop `const`)
           compiles + runs correctly (the case below), and a const SCALAR recursion or a const DICTIONARY
           consumer (the dict passed UNCHANGED, driven by a runtime counter) is unaffected — only a const
           collection the callee recursively folds OVER. (Fully unrolling the fold over the compile-time
           list is the ideal future fix; until it is wired safely, the reject prevents the hang.)")
  (input  (do
            (def (s (const (: xs (List Int64))) (: acc Int64))
              (match xs ((list) acc) ((list h .. t) (s t (+ acc h)))))
            (def (main) (s (list 1 2 3) 0))
            (export main)))
  (error  CDZ0201))

(case "the runtime-list version of a tail fold compiles and folds correctly"
  (doc    "The correct alternative to the const-collection reject above: the SAME tail fold over a RUNTIME
           `(List Int64)` parameter (no `const`) compiles to a proper `loop` whose `br_if` exit is the real
           length/nil test, and runs — `s [1,2,3] 0` = 6. Pins that dropping `const` (so the list is an
           ordinary runtime value the loop iterates) is the working form, and that the reject above is
           specific to the const-erasure × tail-loop composition, not to tail-folding a list.")
  (input  (do
            (def (s (: xs (List Int64)) (: acc Int64))
              (match xs ((list) acc) ((list h .. t) (s t (+ acc h)))))
            (def (main) (s (list 1 2 3) 0))
            (export main)))
  (output (: 6 Int64)))

; INLINE POLICY — the `@inline-never` / `@inline-always` ANNOTATIONS (`DESIGN-…-monomorphization`
; Addendum 4). `@name form` is the general-purpose annotation sigil (canonical `(@ name form)`); these are
; the two names the compiler consumes today. The compiler lowers by β-reduction, so the DEFAULT is
; always-inline; `@inline-never` forces a def to be emitted as ONE real function and CALLED (never
; inlined), controlling code size. It COMPOSES with `const`/generics — "avoid the inline but still get
; polymorphism": an `@inline-never` def with a `const` dictionary param still inlines the dict into a
; per-instantiation specialized copy (direct op, no runtime dispatch) and emits that copy once.
; `@inline-always` is the (currently inert) opposite; on a recursive def it is a contradiction (recursion
; can't inline) → rejected.

(case "an inline-never definition is emitted once and called"
  (doc    "`big` is annotated `@inline-never`, so instead of β-reducing at each call site it is emitted as
           one real function and called. Observable via the VALUE (the emission strategy does not change
           semantics): `big(x) = x*7 + x*11 + x*13`, `big(2) + big(3)` = 62 + 93 = 155. The point is that
           `big`'s body is emitted ONCE (one function, two calls) rather than duplicated per call site.")
  (input  (do
            (@ inline-never (def (big (: x Int64)) (+ (* x 7) (+ (* x 11) (* x 13)))))
            (def (main) (+ (big 2) (big 3)))
            (export main)))
  (output (: 155 Int64)))

(case "an inline-never definition with a const dictionary still monomorphizes the dictionary"
  (doc    "`@inline-never` COMPOSES with a `const` dictionary parameter (`avoid the inline but keep
           polymorphism`): `apply2` is emitted ONCE per distinct dictionary with the dictionary's `op`
           INLINED (no runtime record, no indirect dispatch) — the dictionary is compile-time-erased — and
           that specialized function is CALLED at each use rather than the whole body being inlined.
           `apply2` applies `d.op` twice; with `op = (+ n 10)`: `5 → 25`, `100 → 120`; 25 + 120 = 145.")
  (input  (do
            (@ inline-never
              (def (apply2 (const (: d (Record (op (-> Int64 Int64))))) (: x Int64))
                ((. d op) ((. d op) x))))
            (def (main) (+ (apply2 (record (op (fn (n) (+ n 10)))) 5)
                           (apply2 (record (op (fn (n) (+ n 10)))) 100)))
            (export main)))
  (output (: 145 Int64)))

(case "inline-always on a recursive definition is rejected"
  (doc    "`@inline-always` asks the compiler to always fold a def at its call sites, but a RECURSIVE def
           cannot inline (it would inline without end; it is always emitted as one function). The
           annotation is therefore a contradiction and is rejected (CDZ0201). The rejection is the program's
           outcome.")
  (input  (do
            (@ inline-always (def (loop-n (: n Int64)) (if (= n 0) 0 (loop-n (- n 1)))))
            (def (main) (loop-n 5))
            (export main)))
  (error  CDZ0201))

; COST HEURISTIC (Addendum 4). The UNANNOTATED default is always-inline, but a LARGE, MULTIPLY-CALLED def
; whose call has a runtime-dependent argument is emitted ONCE and called instead of duplicated at each site.
; This is an EMISSION-STRATEGY choice — it does NOT change semantics — so it is observable only via the
; VALUE being unchanged: `big(x) = x*7 + x*11 + x*13 + x*17` = 48x. `main a b = big(a) + big(b)`; the export
; is called with runtime args by the harness. The heuristic emits `big` once and calls it twice; the
; `@inline-never` case above forces the same emission, and both agree on the value. `big(2)+big(3)` = 96 +
; 144 = 240. (The floor is deliberately conservative; small helpers stay inlined.)
(case "a large multiply-called definition is emitted once by the cost heuristic"
  (doc    "A def large enough (past the inline-cost floor) and called at multiple sites with a runtime
           argument is emitted as ONE function and called, not inlined per site — the cost heuristic's
           duplication win. Semantics are unchanged (emission strategy only): with runtime args a=2, b=3,
           `big(x)=48x`, so `big(2)+big(3)` = 96 + 144 = 240.")
  (input  (do
            (def (big (: x Int64)) (+ (* x 7) (+ (* x 11) (+ (* x 13) (* x 17)))))
            (def (main (: a Int64) (: b Int64)) (+ (big a) (big b)))
            (export main)))
  (call   main (: 2 Int64) (: 3 Int64))
  (output (: 240 Int64)))
