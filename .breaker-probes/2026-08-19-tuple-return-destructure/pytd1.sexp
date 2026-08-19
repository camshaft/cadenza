(case "pytd1 probe: op RETURNS a tuple resume value; the body DESTRUCTURES each dispatch's tuple result via match, two dispatches thread state so the tuple fields differ per call"
  (input (do
  (effect E (op split (-> (Tuple Int64 Int64))))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((split () s (resume (tuple (+ s 5) (* s 10)) (+ s 1))))
      (let ((p (match (E.split) ((tuple a b) (+ a b)))))
        (match (E.split) ((tuple c d) (+ (* 1000 p) (+ c d)))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 16027 Int64))
  (call   main (: 0 Int64)) (output (: 5016 Int64)))
