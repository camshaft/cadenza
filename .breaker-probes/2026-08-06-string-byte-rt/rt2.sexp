(case "rt2 the UNCOMPACTED rope round-trip: to-bytes of a deep string rope decodes without compact"
  (input  (do
            (def (build (: n Int64) (: acc String))
              (if (= n 0) acc (build (- n 1) (String.concat acc "é"))))
            (def (main (: k Int64))
              (do
                (def s (build k "x"))
                (match (String.from-bytes (String.to-bytes s))
                  ((Some s2) (+ (* 100 (if (= s2 s) 1 0)) (String.scalar-len s2)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 141 Int64)))
