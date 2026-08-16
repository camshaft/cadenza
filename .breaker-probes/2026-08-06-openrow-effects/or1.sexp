(case "or1 an open-row RECORD as handler state — the arm projects one field and rebuilds"
  (input  (do
            (effect St (op hit (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (record (count n) (tag 7))
                ((hit (u) s (resume (. s count) (record (count (+ (. s count) 1)) (tag (. s tag))))))
                (+ (* 10 (St.hit)) (St.hit))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
