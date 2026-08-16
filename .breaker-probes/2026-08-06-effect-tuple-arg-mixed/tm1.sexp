(case "tm1 a heterogeneous TUPLE as op ARGUMENT — the arm destructures both components"
  (input  (do
            (effect St (op score (-> (Tuple String Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((score (p) s (match p ((tuple name pts) (resume (+ (String.byte-len name) (* pts 10)) s)))))
                (St.score (tuple "abc" n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 53 Int64)))
