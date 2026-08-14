(case "lstX a List.push dual-use let in a MIXED-op region — grow binds the pushed list once for both slots, llen reads the length between grows"
  (input  (do
            (effect L
              (op grow (-> Int64))
              (op llen (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle L (: (list n 7) (List Int64))
                ((grow () st
                  (let ((r (List.push st (+ (lastv st) 1))))
                    (resume (+ (List.len r) (lastv r)) r)))
                 (llen () st (resume (List.len st) st)))
                (let ((a (L.grow)))
                  (let ((b (L.llen)))
                    (let ((c (L.grow)))
                      (let ((d (L.llen)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11031304 Int64))
  (call   main (: 0 Int64)) (output (: 11031304 Int64)))
