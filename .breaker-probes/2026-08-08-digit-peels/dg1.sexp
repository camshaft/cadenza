(case "dg1 a DIGIT-extractor arm — each dispatch peels the low digit and floors the state by ten, three peels reverse the tail"
  (input  (do
            (effect E (op peel (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((peel () s (resume (% s 10) (/ s 10))))
                (+ (* 100 (E.peel)) (+ (* 10 (E.peel)) (E.peel)))))
            (export main)))
  (call   main (: 4728 Int64)) (output (: 827 Int64))
  (call   main (: 56 Int64)) (output (: 650 Int64))
  (call   main (: 900 Int64)) (output (: 9 Int64)))
