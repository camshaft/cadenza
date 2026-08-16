(case "fo1 a float-carrying sum as a Set key: distinct payloads distinct, same-payload dedupes"
  (input  (do
            (type Reading (Temp Float64) (Missing))
            (def (main (: x Float64))
              (let ((s (Set.of (list (Temp x) (Temp 1.5) (Temp x) (Missing)))))
                (+ (Set.len s)
                   (* 10 (if (Set.contains s (Temp x)) 1 0)))))
            (export main)))
  (call   main (: 2.5 Float64)) (output (: 13 Int64)))
