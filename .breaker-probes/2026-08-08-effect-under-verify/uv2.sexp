(case "uv2 a @requires-guarded fn fed by DRAWS — the contract checks effectful argument values at each call"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (requires (> x 3)) (def (f (: x Int64)) (* x 10)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (f (St.next)) (f (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 4 Int64)) (output (: 90 Int64)))
