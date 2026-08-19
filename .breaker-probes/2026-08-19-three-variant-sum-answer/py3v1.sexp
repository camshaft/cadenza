(case "py3v1 probe: op resumes a 3-VARIANT user Sum (Lo/Mid/Hi) selected by captured state via nested if; the body matches all three arms — wider tag space than 2-variant Ok/Err, both dispatches land in different variants as state threads across the seed boundary"
  (input (do
  (effect E (op tick (-> Sig)))
  (type Sig (Lo Int64) (Mid Int64) (Hi Int64))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (resume (if (< s 1) (Lo (* s 10))
                    (if (< s 2) (Mid (* s 100)) (Hi (* s 1000))))
                (+ s 1))))
      (+ (* 100 (match (E.tick) ((Lo x) x) ((Mid x) (+ x 1)) ((Hi x) (+ x 2))))
         (match (E.tick) ((Lo x) x) ((Mid x) (+ x 1)) ((Hi x) (+ x 2))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 12102 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))
