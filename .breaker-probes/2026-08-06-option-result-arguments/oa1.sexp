(case "oa1 an OPTION as op ARGUMENT — the arm matches Some/None it was handed, per dispatch"
  (input  (do
            (effect St (op weigh (-> (Option Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((weigh (o) s (resume (match o ((Some v) (* v 10)) ((None _u) -1)) s)))
                (+ (* 100 (St.weigh (Some n)))
                   (St.weigh (None unit)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4999 Int64)))
