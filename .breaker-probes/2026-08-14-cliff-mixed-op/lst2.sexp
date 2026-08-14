(case "lst2 the lstB twin over a TUPLE-OF-TWO-LISTS state — grow pushes the first list, the branch consults its length, the second list rides along untouched"
  (input  (do
            (effect L (op grow (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle L (tuple (: (list n 7) (List Int64)) (: (list) (List Int64)))
                ((grow () st
                  (match st
                    ((tuple xs ys)
                      (if (< 4 (List.len xs))
                          (resume (- 0 (List.len xs)) st)
                          (let ((r (List.push xs (+ (lastv xs) 1))))
                            (resume (+ (List.len r) (lastv r)) (tuple r ys))))))))
                (let ((a (L.grow)))
                  (let ((b (L.grow)))
                    (let ((c (L.grow)))
                      (let ((d (L.grow)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11131495 Int64))
  (call   main (: 0 Int64)) (output (: 11131495 Int64)))
