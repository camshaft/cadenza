(case "lstW TWO list-WRITING ops interleaved — grow pushes through a dual-use let, shrink pops through dropl, four straight-line dispatches alternate them"
  (input  (do
            (effect L
              (op grow (-> Int64))
              (op shrink (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (dropl (: xs (List Int64)) (: i Int64) (: keep Int64) (: acc (List Int64)))
              (if (< i keep)
                  (dropl xs (+ i 1) keep (List.push acc (match (List.at xs i) ((Some v) v) ((None u) 0))))
                  acc))
            (def (main (: n Int64))
              (handle L (: (list n 7) (List Int64))
                ((grow () st
                  (let ((r (List.push st (+ (lastv st) 1))))
                    (resume (+ (List.len r) (lastv r)) r)))
                 (shrink () st
                  (if (= (List.len st) 0)
                      (resume -99 st)
                      (resume (lastv st) (dropl st 0 (- (List.len st) 1) (: (list) (List Int64)))))))
                (let ((a (L.grow)))
                  (let ((b (L.shrink)))
                    (let ((c (L.grow)))
                      (let ((d (L.shrink)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11081108 Int64))
  (call   main (: 0 Int64)) (output (: 11081108 Int64)))
