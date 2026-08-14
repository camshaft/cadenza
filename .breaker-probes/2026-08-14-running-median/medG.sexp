(case "medG control (TWO dispatches) — List-arg def with internal let"
  (input  (do
            (effect M (op add (-> Int64 Int64)))
            (def (getat (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None u) 0)))
            (def (midv (: xs (List Int64)))
              (let ((m (List.len xs)))
                (getat xs (/ m 2))))
            (def (main (: n Int64))
              (handle M (: (list) (List Int64))
                ((add (v) st
                  (let ((s2 (List.push st v)))
                    (resume (midv s2) s2))))
                (let ((a (M.add (+ n 4))))
                  (let ((b (M.add 2)))
                    (+ (* 100 a) b)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1402 Int64))
  (call   main (: 0 Int64)) (output (: 402 Int64)))
