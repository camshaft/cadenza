(case "pyft1 probe: BARE foreign perform (not a nested handle) in the post-resume TOLL position — should FOLD unlike the nested-handle toll"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op aux (-> Int64)))
  (def (main (: n Int64))
    (handle F (: 0 Int64)
      ((aux () fs (resume (: 100 Int64) fs)))
      (handle E (% n 3)
        ((tick () s (+ (resume (+ s 1) (* 10 s)) (F.aux))))
        (+ (E.tick) (* 10 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 312 Int64))
  (call   main (: 0 Int64)) (output (: 211 Int64)))
