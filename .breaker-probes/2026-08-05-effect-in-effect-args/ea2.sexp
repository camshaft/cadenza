(case "ea2 a perform whose arg is an IF over another perform, branch-selecting between two MORE performs"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op lo (-> Unit Int64)) (op hi (-> Unit Int64)) (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1)))
                 (lo (u) s (resume 10 s))
                 (hi (u) s (resume 99 s))
                 (put (v) s (resume (+ v s) s)))
                (St.put (if (> (St.get) 3) (St.hi) (St.lo)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
