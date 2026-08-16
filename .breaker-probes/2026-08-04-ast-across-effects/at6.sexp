(case "at6 inner arm performs OUTER and the resume value READS inner state (the #2102 decline shape)"
  (input  (do
            (effect Outer (op log (-> Int64 Int64)))
            (effect Inner (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Outer 0
                ((log (v) s (resume v s)))
                (handle Inner 100
                  ((step (v) t (resume (+ t (Outer.log v)) t)))
                  (Inner.step n))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 125 Int64)))
