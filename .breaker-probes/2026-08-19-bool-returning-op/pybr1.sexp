(case "pybr1 probe: an op RETURNS a Bool computed from a comparison of the captured state — ge(t) answers (>= s t) and threads +1; the body uses the two boolean draws as if-guards selecting different constants, so the threaded state flips the first guard across seeds while the second stays true"
  (input (do
  (effect E (op ge (-> Int64 Bool)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((ge (t) s (resume (>= s t) (+ s 1))))
      (+ (if (E.ge (: 1 Int64)) (: 1000 Int64) (: 2000 Int64))
         (if (E.ge (: 0 Int64)) (: 30 Int64) (: 40 Int64)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 1030 Int64))
  (call   main (: 0 Int64)) (output (: 2030 Int64)))
