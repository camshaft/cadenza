(case "s4 capturing closure stored in map, called ONLY via lookup"
  (input  (do
            (def (main (: d Int64))
              (let ((xs (list 7 8 9)))
                (let ((f1 (fn ((: v Int64)) (+ (* (List.len xs) 100) v))))
                  (let ((m2 (Map.insert Map.empty 1 f1)))
                    (match (Map.lookup m2 1) ((Some g) (g d)) ((None u) -999))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 305 Int64)))
