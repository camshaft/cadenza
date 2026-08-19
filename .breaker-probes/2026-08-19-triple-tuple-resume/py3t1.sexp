(case "py3t1 probe: op resumes a THREE-element tuple built from the state (s, 2s, s+100); the body destructures all three fields in one match and packs them into distinct digit ranges, so a field-order swap or a dropped element scrambles the result"
  (input (do
  (effect E (op tick (-> (Tuple Int64 Int64 Int64))))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (tuple s (* s 2) (+ s 100)) (+ s 1))))
      (match (E.tick) ((tuple a b c) (+ (* 1000 c) (+ (* 10 b) a))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 101021 Int64))
  (call   main (: 0 Int64)) (output (: 100000 Int64)))
