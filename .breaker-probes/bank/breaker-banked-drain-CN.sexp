(case "a recursive descent RETURNS a closure capturing its deepest frame's binding"
  (doc    "The CAPTURING upgrade of the returned-closure pin (:2344's `pick` returns a capture-FREE
           `(fn (x) (+ x 1))`): here the closure captures `acc` — the value ACCUMULATED across the
           whole descent, materialized in the base-case frame (n=3: acc = 3+2+1 = 6; closure = +6;
           applied to 5 → 11; n=0 → +0 → 5). The env must snapshot the deepest frame's binding and
           survive the entire unwind back to the caller — a capture that read a shallower frame's acc
           (or the frame's slot after the unwind recycled it) gives a wrong base or garbage. The
           escape-from-recursion-depth face of closure capture.")
  (input  (do
            (def (dig (: n Int64) (: acc Int64))
              (if (= n 0)
                (fn ((: d Int64)) (+ acc d))
                (dig (- n 1) (+ acc n))))
            (def (main (: n Int64))
              ((dig n 0) 5))
            (export main)))
  (call   main (: 3 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64)))
