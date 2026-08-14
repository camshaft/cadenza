(case "lstF a BOTH-SLOTS writer mixed with a dual-use-let writer — growx pushes the first list, flip SWAPS the two lists in one arm, four alternating dispatches"
  (input  (do
            (effect L
              (op growx (-> Int64))
              (op flip (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle L (tuple (: (list n 7) (List Int64)) (: (list) (List Int64)))
                ((growx () st
                  (match st
                    ((tuple xs ys)
                      (let ((r (List.push xs (+ (lastv xs) 1))))
                        (resume (+ (List.len r) (lastv r)) (tuple r ys))))))
                 (flip () st
                  (match st
                    ((tuple xs ys) (resume (List.len ys) (tuple ys xs))))))
                (let ((a (L.growx)))
                  (let ((b (L.flip)))
                    (let ((c (L.growx)))
                      (let ((d (L.flip)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11000203 Int64))
  (call   main (: 0 Int64)) (output (: 11000203 Int64)))
