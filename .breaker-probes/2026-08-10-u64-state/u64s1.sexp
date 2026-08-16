(case "u64s1 a UInt64 handler state ABOVE the i64 boundary — unsigned comparison in the arm stays correct as the thread advances past 2^63"
  (input  (do
            (effect E (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E (if (> n 0) (: 9223372036854775813 UInt64)
                            (if (= n 0) (: 9223372036854775808 UInt64)
                                (: 9223372036854775802 UInt64)))
                ((probe () s
                  (resume (if (> s (: 9223372036854775808 UInt64)) 1 0)
                          (+ s (: 5 UInt64)))))
                (+ (* 10 (E.probe)) (E.probe))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -6 Int64)) (output (: 0 Int64)))
