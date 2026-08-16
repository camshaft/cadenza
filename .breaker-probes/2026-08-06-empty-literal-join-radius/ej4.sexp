(case "ej4 ms13 shape but the Some-arm REBINDS through a pure helper first (evidence at the arm)"
  (input  (do
            (def (grab (: xs (List Int64))) xs)
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (match (Map.lookup m "k") ((Some ys) (grab ys)) ((None _u) (list)))))
                  (List.len (List.push xs n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
