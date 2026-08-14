(case "lstP TWO writing ops over a TUPLE-OF-TWO-LISTS — growx pushes the first list through a dual-use let, growy pushes the second, four alternating dispatches"
  (input  (do
            (effect L
              (op growx (-> Int64))
              (op growy (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle L (tuple (: (list n 7) (List Int64)) (: (list) (List Int64)))
                ((growx () st
                  (match st
                    ((tuple xs ys)
                      (let ((r (List.push xs (+ (lastv xs) 1))))
                        (resume (+ (List.len r) (lastv r)) (tuple r ys))))))
                 (growy () st
                  (match st
                    ((tuple xs ys)
                      (let ((r (List.push ys (List.len ys))))
                        (resume (List.len r) (tuple xs r)))))))
                (let ((a (L.growx)))
                  (let ((b (L.growy)))
                    (let ((c (L.growx)))
                      (let ((d (L.growy)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11011302 Int64))
  (call   main (: 0 Int64)) (output (: 11011302 Int64)))
