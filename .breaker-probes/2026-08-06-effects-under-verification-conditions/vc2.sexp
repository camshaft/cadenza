(case "vc2 a VIOLATED @requires traps before the body's perform fires (no state advance)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f (- 0 n))))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))
