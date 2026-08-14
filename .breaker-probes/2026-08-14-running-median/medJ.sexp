(case "medJ control — the def's let binds List.len SPECIFICALLY"
  (input  (do
            (effect M (op add (-> Int64 Int64)))
            (def (lenplus (: xs (List Int64)))
              (let ((m (List.len xs)))
                (+ m 1)))
            (def (main (: n Int64))
              (handle M (: (list n 7) (List Int64))
                ((add (v) st
                  (resume (+ (lenplus st) v) (List.push st v))))
                (let ((a (M.add (+ n 4))))
                  (let ((b (M.add 2)))
                    (+ (* 100 a) b)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1706 Int64))
  (call   main (: 0 Int64)) (output (: 706 Int64)))