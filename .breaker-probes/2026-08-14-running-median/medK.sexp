(case "medK control — the def uses the LIST AGAIN after its let (list live across the let)"
  (input  (do
            (effect M (op add (-> Int64 Int64)))
            (def (getat (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None u) 0)))
            (def (lastof (: xs (List Int64)))
              (let ((m (List.len xs)))
                (getat xs (- m 1))))
            (def (main (: n Int64))
              (handle M (: (list n 7) (List Int64))
                ((add (v) st
                  (resume (+ (lastof st) v) (List.push st v))))
                (let ((a (M.add (+ n 4))))
                  (let ((b (M.add 2)))
                    (+ (* 100 a) b)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2116 Int64))
  (call   main (: 0 Int64)) (output (: 1106 Int64)))