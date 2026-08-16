(case "mia1 a MATCH over one op's Option result sits in ANOTHER op's argument position — unwrap-then-perform composed twice, verdicts flip"
  (input  (do
            (effect S (op cls (-> Int64 (Option Int64))) (op use (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((cls (v) s (resume (if (< v s) (Some (* v 2)) (: (None unit) (Option Int64))) (+ s 1)))
                 (use (v) s (resume (+ v s) (+ s 10))))
                (+ (S.use (match (S.cls 3) ((Some x) x) ((None _u) -50)))
                   (* 1000 (S.use (match (S.cls 3) ((Some x) x) ((None _u) -50)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 23012 Int64))
  (call   main (: 2 Int64)) (output (: 19953 Int64)))
