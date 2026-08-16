(case "wp4c dissect: abort inside a NON-branching recursive walk (linear list-walk, halt at a marker)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) (Cnt.halt) (+ (Cnt.bump) (loop (- n 1)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1)))
                 (halt (u) s (* 1000 s)))
                (loop n)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2000 Int64)))
