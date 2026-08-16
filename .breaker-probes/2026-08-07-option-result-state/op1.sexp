(case "op1 the STD Option as handler state — None seeds to Some on first feed, the payload accumulates thereafter"
  (input  (do
            (effect O (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (None)
                ((feed (v) s (match s
                               ((None) (resume 0 (Some v)))
                               ((Some k) (resume k (Some (+ k v)))))))
                (+ (O.feed n) (+ (* 10 (O.feed 3)) (* 100 (O.feed 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 850 Int64))
  (call   main (: 0 Int64)) (output (: 300 Int64)))
