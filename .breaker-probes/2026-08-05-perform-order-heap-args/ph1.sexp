(case "ph1 performs building HEAP args of one call: (Bytes.concat (mk (St.a)) (mk (St.a))) — order via content"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (mk (: v Int64)) (Bytes.of (list (UInt8.wrap v))))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (do
                  (def b (Bytes.concat (mk (St.a)) (mk (St.a))))
                  (+ (* 100 (match (Bytes.at b 0) ((Some v) (Int64.of v)) ((None _u) -1)))
                     (match (Bytes.at b 1) ((Some v) (Int64.of v)) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))
