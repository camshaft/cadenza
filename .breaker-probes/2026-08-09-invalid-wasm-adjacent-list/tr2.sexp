(case "tr2 ADJACENCY-PROBE: TWO string fields in the tuple, only one phi-grown — does the untouched sibling rope field survive"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab" "cd")
                ((put () st (match st
                              ((tuple s a b)
                               (resume s (tuple (+ s 1) a
                                                (String.concat b (if (= (% s 3) 0) "x" "yz")))))))
                 (size () st (match st
                               ((tuple s a b)
                                (resume (+ (* 100 (String.byte-len a)) (String.byte-len b)) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 410 Int64))
  (call   main (: 1 Int64)) (output (: 412 Int64))
  (call   main (: -2 Int64)) (output (: 412 Int64)))
