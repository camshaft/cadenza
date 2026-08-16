(case "dl3 a 40-element SET op result — a multi-node CHAMP payload crosses resume"
  (input  (do
            (effect St (op universe (-> Int64 (Set Int64))))
            (def (fill (: i Int64) (: k Int64) (: acc (Set Int64)))
              (if (> i k) acc (fill (+ i 1) k (Set.insert acc (* i 3)))))
            (def (main (: n Int64))
              (handle St 0
                ((universe (k) s (resume (fill 1 k (Set.of (list))) s)))
                (let ((xs (St.universe (* n 8))))
                  (+ (* 100 (Set.len xs))
                     (+ (if (Set.contains xs 60) 10 0)
                        (if (Set.contains xs 61) 1 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4010 Int64)))
