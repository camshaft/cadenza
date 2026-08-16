(case "li1 draws choose BOTH the List.update target and the List.at read index — the collection edit follows the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (let ((xs (list 10 20 30 40 50)))
                  (let ((i1 (% (E.next) 5)))
                    (let ((i2 (% (E.next) 5)))
                      (let ((ys (List.update xs i1 7)))
                        (match (List.at ys i2)
                          ((Some v) (+ (* 100 v) (+ (* 10 i1) i2)))
                          ((None) -1))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3002 Int64))
  (call   main (: 3 Int64)) (output (: 1030 Int64))
  (call   main (: 4 Int64)) (output (: 2041 Int64)))
