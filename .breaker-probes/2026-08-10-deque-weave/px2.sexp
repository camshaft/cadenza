(case "px2 update at BOTH ends of a deque-woven list, original persists"
  (input  (do
            (def (weave (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc
                  (weave (- i 1)
                    (if (= (% i 2) 0) (List.push acc i) (List.prepend acc (- 0 i))))))
            (def (main (: n Int64))
              (do
                (def xs (weave n (list 0)))
                (def last (- (List.len xs) 1))
                (def u (List.update (List.update xs 0 777) last 888))
                (def (at (: q (List Int64)) (: i Int64)) (Option.expect (List.at q i) "in"))
                (+ (* 1000 (if (and (= (at u 0) 777) (= (at u last) 888)) 1 0))
                   (+ (* 10 (if (= (at xs 0) -1) 1 0))
                      (if (= (at xs last) 2) 1 0)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 1011 Int64)))
