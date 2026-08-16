(case "uv3 the handler ARM calls a @requires-guarded helper — the contract checks the live state per dispatch"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (requires (> x 0)) (def (safe-dbl (: x Int64)) (* x 2)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume (safe-dbl s) (+ s 1))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64))
  (call   main (: 1 Int64)) (output (: 6 Int64)))
