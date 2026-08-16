(case "es2 a VIOLATED @ensures on a performing def traps at body-exit"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (ensures (>= ret 1000)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))
