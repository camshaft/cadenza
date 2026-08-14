(case "medI control — List-arg def whose internal let binds the ELEMENT (scalar from list)"
  (input  (do
            (effect M (op add (-> Int64 Int64)))
            (def (getat (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None u) 0)))
            (def (firstplus (: xs (List Int64)))
              (let ((h (getat xs 0)))
                (+ h 1)))
            (def (main (: n Int64))
              (handle M (: (list n 7) (List Int64))
                ((add (v) st
                  (resume (+ (firstplus st) v) (List.push st v))))
                (let ((a (M.add (+ n 4))))
                  (let ((b (M.add 2)))
                    (+ (* 100 a) b)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2513 Int64))
  (call   main (: 0 Int64)) (output (: 503 Int64)))