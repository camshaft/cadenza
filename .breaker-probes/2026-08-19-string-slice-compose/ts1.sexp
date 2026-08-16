(case "ts1 a String slice OF a slice over a multibyte concat rope composes scalar offsets"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (String.concat "aé∀" "bçd"))
                (match (String.slice rope 1 5)
                  ((Some outer)
                    (match (String.slice outer 1 3)
                      ((Some inner)
                        (+ (* 100 (String.scalar-len inner))
                           (if (= inner "∀b") 1 0)))
                      ((None _u) -2)))
                  ((None _u) -3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 201 Int64)))
