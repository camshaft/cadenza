(case "rw1 a nested handle at a performing RECURSION's exit leaf, then a trailing outer draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0)
                    (handle B d
                      ((g (u) t (resume t (+ t 1))))
                      (+ (B.g) (* 10 (B.g))))
                    (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (walk 0) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 95 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -13 Int64)) (output (: -73 Int64)))
