(case "lf5 TWO lists walked in LOCKSTEP with a per-pair 2-arg dispatch — length-guarded via projection helpers"
  (input  (do
            (effect St (op mix (-> Int64 Int64 Int64)))
            (def (pair-or (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) -1)))
            (def (zipwalk (: xs (List Int64)) (: ys (List Int64)) (: i Int64))
              (if (< i (List.len xs))
                  (+ (St.mix (pair-or xs i) (pair-or ys i)) (zipwalk xs ys (+ i 1)))
                  0))
            (def (main (: n Int64))
              (handle St n
                ((mix (a b) s (resume (+ (* a b) s) (+ s 1))))
                (zipwalk (list 1 2 3) (list 10 20 30) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 158 Int64))
  (call   main (: 0 Int64)) (output (: 143 Int64)))
