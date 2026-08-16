(case "li2 a list BUILT from pushed draws then read at a draw-picked index — construction and consumption share one thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3))))
                (let ((xs (List.push (List.push (List.push (list) (E.next)) (E.next)) (E.next))))
                  (let ((i (% (E.next) 3)))
                    (match (List.at xs i)
                      ((Some v) (+ 100 (+ (* 10 v) i)))
                      ((None) -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 100 Int64))
  (call   main (: 1 Int64)) (output (: 141 Int64))
  (call   main (: 2 Int64)) (output (: 182 Int64)))
