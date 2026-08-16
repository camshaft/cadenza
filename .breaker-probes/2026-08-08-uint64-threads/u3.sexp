(case "u3 UInt64 arguments ABOVE Int64.max echo through dispatch exactly — 2^63+41 and u64-max both survive, count pins trips"
  (input  (do
            (effect E (op keep (-> UInt64 UInt64)) (op count (-> UInt64)))
            (def (main (: u UInt64))
              (handle E (: 0 UInt64)
                ((keep (x) s (resume x (+ s (: 1 UInt64))))
                 (count () s (resume s s)))
                (+ (* (: 100 UInt64) (if (= (E.keep (: 9223372036854775849 UInt64)) (: 9223372036854775849 UInt64)) (: 1 UInt64) (: 9 UInt64)))
                   (+ (* (: 10 UInt64) (if (= (E.keep (: 18446744073709551615 UInt64)) (: 18446744073709551615 UInt64)) (: 1 UInt64) (: 9 UInt64)))
                      (E.count)))))
            (export main)))
  (call   main (: 0 UInt64)) (output (: 112 UInt64)))
