(case "rb1 a RATE-limiter window: a list of timestamps prunes against a moving cutoff"
  (input  (do
            (def (prune (: xs (List Int64)) (: cutoff Int64) (: acc (List Int64)))
              (match xs
                ((list) acc)
                ((list h .. t) (prune t cutoff (if (>= h cutoff) (List.push acc h) acc)))))
            (def (feed (: i Int64) (: n Int64) (: win (List Int64)) (: rejected Int64))
              (if (> i n) (tuple win rejected)
                (let ((pruned (prune win (- (* i 10) 25) (list))))
                  (if (< (List.len pruned) 3)
                      (feed (+ i 1) n (List.push pruned (* i 10)) rejected)
                      (feed (+ i 1) n pruned (+ rejected 1))))))
            (def (main (: n Int64))
              (match (feed 1 n (list) 0)
                ((tuple win rejected) (+ (* 10 (List.len win)) rejected))))
            (export main)))
  (call   main (: 12 Int64)) (output (: 30 Int64)))
