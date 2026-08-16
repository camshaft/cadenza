(case "ts3 a slice window ending exactly at the rope seam then re-sliced to its multibyte tail"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (String.concat "aé∀" "bçd"))
                (match (String.slice rope 0 3)
                  ((Some head)
                    (match (String.slice head 1 3)
                      ((Some tail2)
                        (+ (* 10 (if (= tail2 "é∀") 1 0)) (String.scalar-len tail2)))
                      ((None _u) -2)))
                  ((None _u) -3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
