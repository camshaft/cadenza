(case "op1 the arm's Option verdict flips per dispatch as state compares to the key — Some and None both cross, a pure helper unwraps"
  (input  (do
            (effect S (op find (-> Int64 (Option Int64))))
            (def (unwrap-or (: o (Option Int64)) (: d Int64))
              (match o ((Some v) v) ((None _u) d)))
            (def (main (: n Int64))
              (handle S n
                ((find (k) s (resume (if (< k s) (Some (+ k 100)) (: (None unit) (Option Int64))) s)))
                (+ (* 100 (unwrap-or (S.find 1) -3))
                   (unwrap-or (S.find 9) -3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10097 Int64))
  (call   main (: 0 Int64)) (output (: -303 Int64)))
