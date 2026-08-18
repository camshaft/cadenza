(case "pyq1 a FOREIGN TOLLED PERFORM WHOSE CONTINUATION ESCAPES THE INNER REGION — the inner body performs on the tolled OUTER handler so the outer levy's continuation spans past the inner handle's close, the inner toll settles INSIDE that escaping continuation while the outer levy's own toll wraps the whole remainder including the tenfold scaling and the first draw's addition, and the composition orders three tolls across two frames and a region boundary"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (+ (resume t (+ t 1)) (* 10000 (+ t 1)))))
                (+ (T.levy)
                   (* 10 (handle E (: 5 Int64)
                           ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                           (+ (E.tick) (T.levy)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 55071 Int64))
  (call   main (: 0 Int64)) (output (: 35060 Int64)))
