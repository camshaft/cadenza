(case "fr1 a TRIPLING Float64 thread crosses a fixed threshold — three compares catch the crossing depth, exact integer-valued floats"
  (input  (do
            (effect E (op over (-> Float64)))
            (def (main (: seed Float64))
              (handle E seed
                ((over () s (resume (if (> s 4.0) 1.0 0.0) (* s 3.0))))
                (+ (* 100.0 (E.over)) (+ (* 10.0 (E.over)) (E.over)))))
            (export main)))
  (call   main (: 1.0 Float64)) (output (: 1.0 Float64))
  (call   main (: 2.0 Float64)) (output (: 11.0 Float64))
  (call   main (: 8.0 Float64)) (output (: 111.0 Float64)))
