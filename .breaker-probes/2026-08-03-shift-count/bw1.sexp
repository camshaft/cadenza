(case "bw1 odd-width bitwise ops preserve the declared-range invariant"
  (input  (do
            (def (main (: k Int64))
              (+ (* 10000 (Int64.of (& ((. (Int 24) wrap) -1) ((. (Int 24) wrap) k))))
                 (+ (* 100 (Int64.of (^ ((. (Int 24) wrap) -1) ((. (Int 24) wrap) k))))
                    (Int64.of (| ((. (Int 24) wrap) 0) ((. (Int 24) wrap) k))))))
            (export main)))
  (call   main (: -8388608 Int64)) (output (: -83055607908 Int64)))
