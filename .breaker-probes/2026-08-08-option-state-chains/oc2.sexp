(case "oc2 an Option FLOWS between two ops of one effect — find produces Some/None, use consumes it as an op ARG"
  (input  (do
            (effect O (op find (-> Int64 (Option Int64))) (op use (-> (Option Int64) Int64)))
            (def (main (: n Int64))
              (handle O n
                ((find (k) s (resume (if (> k s) (Some (- k s)) (None)) (+ s 1)))
                 (use (m) s (match m
                              ((Some v) (resume (* v 10) s))
                              ((None) (resume (- 0 s) s)))))
                (+ (O.use (O.find 10)) (O.use (O.find 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 43 Int64))
  (call   main (: 20 Int64)) (output (: -43 Int64)))
