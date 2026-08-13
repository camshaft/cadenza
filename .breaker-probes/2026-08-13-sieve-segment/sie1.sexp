(case "sie1 a SIEVE SEGMENT — each sieve dispatch clears every multiple of its prime by STRIDED List.update writes in one recursive walk, count answers survivors, probes read prime and composite slots"
  (input  (do
            (effect S
              (op sieve (-> Int64 Int64))
              (op probe (-> Int64 Int64)))
            (def (clear-multiples (: f (List Int64)) (: m Int64) (: p Int64))
              (if (> m 13)
                  f
                  (clear-multiples (List.update f (- m 2) 0) (+ m p) p)))
            (def (count-flags (: f (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at f i)
                ((Some v) (count-flags f (+ i 1) (+ acc v)))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (list 1 1 1 1 1 1 1 1 1 1 1 1)
                ((sieve (p) f
                  (let ((f2 (clear-multiples f (* 2 p) p)))
                    (resume (count-flags f2 0 0) f2)))
                 (probe (x) f
                  (resume (match (List.at f (- x 2)) ((Some v) v) ((None u) -1)) f)))
                (let ((a (S.sieve 2)))
                  (let ((b (S.sieve 3)))
                    (let ((c (S.probe 9)))
                      (let ((d (S.probe n)))
                        (+ (* 10 (+ (* 10 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 70601 Int64))
  (call   main (: 4 Int64)) (output (: 70600 Int64)))
