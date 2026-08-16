(case "ra10 the multi-draw walk on E, trailing read on UNTOUCHED effect B — is B's thread insulated from E's fork"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Int64)))
            (def (walk (: k Int64))
              (let ((a (E.next)))
                (let ((b (E.next)))
                  (if (< a 20) (walk (+ k 1)) k))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (handle B (+ n 3)
                  ((g () t (resume t (+ t 2))))
                  (let ((steps (walk 0)))
                    (+ (* 100 steps) (B.g))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 208 Int64))
  (call   main (: 1 Int64)) (output (: 204 Int64)))
