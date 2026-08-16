(case "se1 a SET handler state with dedup dynamics — re-adding an existing element leaves the size fixed"
  (input  (do
            (effect Sx (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Sx (Set.of (list 1 2))
                ((add (v) s (resume (Set.len s) (Set.insert s v))))
                (+ (Sx.add n) (+ (* 10 (Sx.add 2)) (* 100 (Sx.add n))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 332 Int64))
  (call   main (: 1 Int64)) (output (: 222 Int64)))
