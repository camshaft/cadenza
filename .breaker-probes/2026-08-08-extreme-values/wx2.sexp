(case "wx2 Int64.min and Int64.max cross the dispatch as OP ARGUMENTS and come back exact — a count arm tallies the trips"
  (input  (do
            (effect E (op keep (-> Int64 Int64)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((keep (x) s (resume x (+ s 1)))
                 (count () s (resume s s)))
                (+ (if (= (E.keep -9223372036854775808) -9223372036854775808) 100 900)
                   (+ (if (= (E.keep 9223372036854775807) 9223372036854775807) 10 90)
                      (E.count)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 112 Int64)))
