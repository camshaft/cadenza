(case "ej5 the IF-join face of the Var-arm class — a Map.empty lookup payload in the then, empty (list) in the else"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (if (> n 0)
                              (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (list)))
                              (list))))
                  (List.len (List.push xs n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
