(case "sc2 the stacked contract's PRE fires through the @ensures layer before the perform (abortive observer)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0))
            (@ (ensures (>= ret 100))
               (def (f (: x Int64)) (+ x (St.bump)))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s 999))
                (f (- 0 n))))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))
