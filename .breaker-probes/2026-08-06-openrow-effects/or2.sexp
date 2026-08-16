(case "or2 an OPEN-ROW helper projects from the handler state inside the arm (row-poly under the fold)"
  (input  (do
            (effect St (op hit (-> Unit Int64)))
            (def (get-count r) (. r count))
            (def (main (: n Int64))
              (handle St (record (count n) (tag 7))
                ((hit (u) s (resume (get-count s) (record (count (+ (get-count s) 1)) (tag 9)))))
                (+ (* 10 (St.hit)) (St.hit))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
