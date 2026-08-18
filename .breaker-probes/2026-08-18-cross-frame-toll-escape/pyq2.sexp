(case "pyq2 the ESCAPING LEVY AS THE INNER BODY'S LAST FORM — the tolled inner draw comes first and the foreign tolled levy second so the levy's continuation carries only the addition the inner toll and the region close, the outer toll wraps that short escape while the inner toll still settles inside it, and swapping the two summands crosses into the declined fold face"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (+ (resume t (+ t 1)) (* 10000 (+ t 1)))))
                (handle E (: 5 Int64)
                  ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                  (+ (* 10 (E.tick)) (T.levy)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 20551 Int64))
  (call   main (: 0 Int64)) (output (: 10550 Int64)))
