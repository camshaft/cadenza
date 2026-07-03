; Functions and closures — witnesses core-semantics.md §Functions. Functions are
; first-class values (fn), applied by (fn-expr arg…), capturing their enclosing
; scope. These are CORE cases (no (needs …)): the seed realizes them, because a
; compiler authored in Cadenza is built from functions and closures. Results are
; (: <value> <Type>); unbounded recursion halts as (exhausted).

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

(case "a function is returned as a result"
  (doc    "Witnesses core-semantics.md §A Function Is A First-Class Value: adder returns a closure over
           its parameter n; the returned function is then applied.")
  (input  (let ((adder (fn (n) (fn (x) (+ x n)))))
            ((adder 10) 5)))
  (output (: 15 Int64)))

(case "applying a function to the wrong number of arguments traps"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments
           (arity sentence): the dynamic interpreter has no result for a mismatched-arity application,
           so it traps of a defined kind rather than producing an unspecified value. A typed generation
           rejects this at compile time before running.")
  (input  ((fn (x y) (+ x y)) 1))
  (trap   "arity mismatch"))

(case "a recursive def computes over its argument"
  (doc    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments and
           §Recursion Is Accountable Against The Resource Measure: sum-to counts down to 0, bounded by
           the resource measure. sum-to(3) = 3 + 2 + 1 + 0 = 6.")
  (input  (module m
            (def (sum-to n)
              (if (= n 0) 0 (+ n (sum-to (+ n -1)))))
            (def (main) (sum-to 3))))
  (output (: 6 Int64)))

(case "unbounded recursion halts by exhausting the resource measure"
  (doc    "Witnesses core-semantics.md §Recursion Is Accountable Against The Resource Measure: a
           function that applies itself with no base case consumes the deterministic resource measure
           and halts at a defined point rather than running forever.")
  (input  (module m
            (def (spin n) (spin (+ n 1)))
            (def (main) (spin 0))))
  (exhausted))
