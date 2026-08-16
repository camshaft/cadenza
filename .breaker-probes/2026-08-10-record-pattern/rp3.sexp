(case "rp3 NESTED record patterns two levels deep destructure a record-in-record"
  (input  (do
            (def (main (: n Int64))
              (match (record (outer (record (inner n) (pad 1))) (top 9))
                ((record (outer (record (inner v))) (top t)) (+ (* 10 v) t))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 49 Int64)))
