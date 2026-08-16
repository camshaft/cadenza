(case "lc1 a LIST of strategies is walked recursively, each applied to a fresh perform result"
  (input  (do
            (effect Cnt (op next (-> Unit Int64)))
            (def (apply-all fs)
              (match fs
                ((list) 0)
                ((list f .. r) (+ (f (Cnt.next)) (apply-all r)))))
            (def (main (: n Int64))
              (handle Cnt n
                ((next (u) s (resume s (+ s 1))))
                (apply-all (list (fn ((: x Int64)) (* x 10)) (fn ((: x Int64)) (+ x 100)) (fn ((: x Int64)) x)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 163 Int64)))
