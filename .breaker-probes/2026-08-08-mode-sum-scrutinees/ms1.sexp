(case "ms1 an op returns a THREE-variant sum built from state parity — two sequential performing scrutinees, the second sees the advanced state"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)))
            (def (score (: m Mode))
              (match m
                ((A) 7)
                ((B x) (* 10 x))
                ((C x y) (+ (* 100 x) y))))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1))))
                (+ (* 1000 (score (E.mode))) (score (E.mode)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7010 Int64))
  (call   main (: 1 Int64)) (output (: 10202 Int64))
  (call   main (: 2 Int64)) (output (: 202007 Int64))
  (call   main (: 4 Int64)) (output (: 40505 Int64)))
