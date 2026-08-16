(case "iv-nat2 the natural invariant construction over a VIOLATING perform result traps through the handler"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (unwrap (Percent.Pct (St.next)))))
            (export main)))
  (call   main (: 42 Int64))
  (output (: 42 Int64))
  (call   main (: 200 Int64))
  (trap   "unreachable"))
