(case "ec2 a PARAMETERIZED handler helper chained through itself — the step size is a closure-free param"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (run (: seed Int64) (: mul Int64))
              (handle St seed
                ((next (u) s (resume s (+ s mul))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (run (run n 1) 10))
            (export main)))
  (call   main (: 5 Int64)) (output (: 32 Int64)))
