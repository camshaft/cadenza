(case "medF control (ONE dispatch) — List-arg def with internal let"
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
                (M.add (+ n 4))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 14 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64)))
