(case "al1 chained LET locals inside the arm — intermediate names feed both the resume value and the next-state"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (let ((doubled (* v 2)))
                            (let ((shifted (+ doubled s)))
                              (resume shifted (+ s doubled))))))
                (+ (E.f 3) (E.f 5))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 32 Int64))
  (call   main (: 0 Int64)) (output (: 22 Int64)))
