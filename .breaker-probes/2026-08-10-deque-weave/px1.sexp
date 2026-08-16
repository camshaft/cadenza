(case "px1 alternating PUSH+PREPEND builds a deque-shaped list, both ends and middle read back"
  (input  (do
            (def (weave (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc
                  (weave (- i 1)
                    (if (= (% i 2) 0) (List.push acc i) (List.prepend acc (- 0 i))))))
            (def (main (: n Int64))
              (do
                (def xs (weave n (list 0)))
                (def (at (: q (List Int64)) (: i Int64)) (Option.expect (List.at q i) "in"))
                (+ (* 10000 (List.len xs))
                   (+ (* 100 (if (= (at xs 0) -1) 1 0))
                      (if (= (at xs (- (List.len xs) 1)) 2) 1 0)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 410101 Int64)))
