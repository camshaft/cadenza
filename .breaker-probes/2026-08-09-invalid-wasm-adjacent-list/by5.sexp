(case "by5 ADJACENCY-PROBE: phi-merged BYTES growth in a tuple state, 2 puts + 2 reads (the slmin11 shape on a bytes slot)"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (Bytes.of (list (UInt8.wrap 9))))
                ((put () st (match st
                              ((tuple s b)
                               (resume s (tuple (+ s 1)
                                                (Bytes.concat b (if (= (% s 3) 0)
                                                                    (Bytes.of (list (UInt8.wrap 1)))
                                                                    (Bytes.of (list (UInt8.wrap 2) (UInt8.wrap 3))))))))))
                 (size () st (match st ((tuple s b) (resume (Bytes.len b) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 8 Int64))
  (call   main (: 1 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: 10 Int64)))
