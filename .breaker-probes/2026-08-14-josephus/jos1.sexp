(case "jos1 JOSEPHUS elimination — every dispatch advances the cursor k-1 around the SHRINKING ring by modulo, removes the survivor there by filtered rebuild, and answers the eliminated id"
  (input  (do
            (effect S (op elim (-> Int64)))
            (def (drop-at (: xs (List Int64)) (: i Int64) (: k Int64) (: acc (List Int64)))
              (match (List.at xs i)
                ((Some v) (drop-at xs (+ i 1) k (if (= i k) acc (List.push acc v))))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (tuple (list 1 2 3 4 5) 0)
                ((elim () st
                  (match st
                    ((tuple ring pos)
                      (let ((p2 (% (+ pos (- n 1)) (List.len ring))))
                        (let ((v (match (List.at ring p2) ((Some x) x) ((None u) -1))))
                          (resume v (tuple (drop-at ring 0 p2 (: (list) (List Int64))) p2))))))))
                (let ((a (S.elim)))
                  (let ((b (S.elim)))
                    (let ((c (S.elim)))
                      (+ (* 100 (+ (* 100 a) b)) c))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 20401 Int64))
  (call   main (: 3 Int64)) (output (: 30105 Int64)))
