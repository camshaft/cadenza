(case "a tuple-destructuring PARAMETER binds its parts and keeps single-argument arity"
  (doc    "core-semantics.md:137 — a def parameter accepts an irrefutable pattern: `(def (dist (tuple
           x y)) …)` occupies ONE argument position and names its parts, so `(dist (tuple a 4))` at
           a=3 computes 9+16=25. Pins the def-param face of the binding-position pattern grant (the
           WORKING perimeter of adv-51, whose fn/lambda face rejects CDZ0101 today — this pin keeps
           the working face from regressing while that fix lands).")
  (input  (do
            (def (dist (tuple x y)) (+ (* x x) (* y y)))
            (def (main (: a Int64))
              (dist (tuple a 4)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 25 Int64)))

(case "a nested tuple-destructuring let over a function result binds all leaves"
  (doc    "The nested-let face over a RUNTIME function result (the pinned :3646/:3650 lets destructure
           literals): `(let (((tuple p (tuple q r)) (mk a))) …)` binds all three leaves of the tuple
           `mk` returns at run time — 100·3 + 10·4 + 5 = 345. Recursive-depth irrefutable matching in
           a let binder against a computed value, the second working-perimeter face of adv-51.")
  (input  (do
            (def (mk (: a Int64)) (tuple a (tuple (+ a 1) (+ a 2))))
            (def (main (: a Int64))
              (let (((tuple p (tuple q r)) (mk a)))
                (+ (* 100 p) (+ (* 10 q) r))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64)))
