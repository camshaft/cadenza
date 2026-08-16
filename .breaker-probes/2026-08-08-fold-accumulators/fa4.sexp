(case "fa4 the accumulator IS a sum — each level's draw moves it around an A->B->C->A cycle capturing payloads on the way"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op next (-> Int64)))
            (def (spin (: k Int64) (: acc Mode))
              (if (<= k 0)
                  acc
                  (spin (- k 1)
                        (match acc
                          ((A) (B (E.next)))
                          ((B x) (C x (E.next)))
                          ((C x y) (A))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 10 (match (spin 4 (A))
                           ((A) 7)
                           ((B x) (* 10 x))
                           ((C x y) (+ (* 100 x) y))))
                   (E.next))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 304 Int64))
  (call   main (: 0 Int64)) (output (: 203 Int64))
  (call   main (: -2 Int64)) (output (: 1 Int64)))
