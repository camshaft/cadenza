(case "se2 the handler RESUMES with a lookup: rope probe key hits the flat-keyed entry inside the arm"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "y") (- n 1))))
            (effect Q (op ask (-> Int64 Int64)))
            (def (main (: n Int64))
              (do
                (def m (Map.insert (Map.insert Map.empty "kyy" 20) "kyyy" 30))
                (handle Q 0
                  ((ask (v) s
                    (match (Map.lookup m (String.concat "k" (rep "" v)))
                      ((Some x) (resume x s))
                      ((None _u) (resume -1 s)))))
                  (+ (Q.ask n) (Q.ask (+ n 1))))))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 50 Int64)))
