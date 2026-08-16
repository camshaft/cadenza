(case "a wildcard element inside a destructuring def parameter drops that part silently"
  (doc    "The wildcard face of the destructuring parameter (:137's trivial-irrefutable element,
           composed into a def param): `(def (first (tuple x _)) x)` binds x and DROPS the second
           element — here a HEAP list, so the wildcard slot must release the unused part cleanly
           (a Perceus drop of the never-bound element, not a leak or a phantom binding) while the
           bound scalar flows through (9). The def-param twin of the wildcard-field pins in match
           position; the binding-position wildcard-over-heap face was unpinned.")
  (input  (do
            (def (first (tuple x _)) x)
            (def (main (: a Int64))
              (first (tuple a (list 1 2 3))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 9 Int64)))

(case "a wildcard element inside a destructuring let binder drops that part"
  (doc    "The let-binder twin over a RUNTIME function result: `(let (((tuple v _) (mk a))) v)` binds
           the first element (a+1 = 10) and wildcards the second (a nested tuple, dropped unbound).
           Pins that a let-position wildcard neither binds a phantom name nor disturbs the sibling
           binding when the discarded part is itself compound.")
  (input  (do
            (def (mk (: a Int64)) (tuple (+ a 1) (tuple a a)))
            (def (main (: a Int64))
              (let (((tuple v _) (mk a)))
                v))
            (export main)))
  (call   main (: 9 Int64)) (output (: 10 Int64)))
