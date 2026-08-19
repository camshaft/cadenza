(case "pyl2 a GROWING LIST AS THE STATE THREAD — the seed becomes the list's head and every dispatch answers head-tenfold-plus-length while pushing the old length, both the seeded head and the growing length land in every answer, and a thread that rebuilds the list or drops the push loses one digit family each"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (list (% n 3))
                ((tick () xs
                  (resume (match (List.at xs 0)
                            ((Some h) (+ (* h 10) (List.len xs)))
                            ((None) (: -1 Int64)))
                          (List.push xs (List.len xs)))))
                (+ (E.tick) (* 100 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1211 Int64))
  (call   main (: 0 Int64)) (output (: 201 Int64)))
