(case "mk2 map keys DERIVED from prior draws — n=1 collides the derived key with the first insert, shrinking the map"
  (input  (do
            (effect Reg (op touch (-> Int64 Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle Reg (map)
                ((touch (k) s (resume (Map.len s) (Map.insert s k k)))
                 (size () s (resume (Map.len s) s)))
                (let ((a (Reg.touch n)))
                  (let ((b (Reg.touch (+ a 1))))
                    (+ (* 100 (Reg.size)) (+ (* 10 b) (Reg.touch n)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 212 Int64))
  (call   main (: 1 Int64)) (output (: 111 Int64)))
