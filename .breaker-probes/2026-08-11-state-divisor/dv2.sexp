(case "dv2 INT_MIN divided by the state — the minus-one seed overflows, other signs give exact halves"
  (input  (do
            (effect S (op div (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((div (v) s (resume (/ v s) (+ s 1))))
                (S.div -9223372036854775808)))
            (export main)))
  (call   main (: 2 Int64)) (output (: -4611686018427387904 Int64))
  (call   main (: -2 Int64)) (output (: 4611686018427387904 Int64))
  (call   main (: -1 Int64)) (trap "integer overflow"))
