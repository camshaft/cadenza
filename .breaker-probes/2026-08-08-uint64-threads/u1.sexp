(case "u1 a UInt64 state thread ABOVE Int64.max — high-half values survive dispatch, mod-10 digits pin the sequence"
  (input  (do
            (effect E (op next (-> UInt64)))
            (def (main (: n UInt64))
              (handle E (+ (: 9223372036854775808 UInt64) n)
                ((next () s (resume s (+ s (: 1 UInt64)))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (+ (* (: 10 UInt64) (% d1 (: 10 UInt64))) (% d2 (: 10 UInt64)))))))
            (export main)))
  (call   main (: 5 UInt64)) (output (: 34 UInt64))
  (call   main (: 0 UInt64)) (output (: 89 UInt64))
  (call   main (: 7 UInt64)) (output (: 56 UInt64)))
