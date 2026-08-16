(case "cx1 a COUNTDOWN arm exhausts mid-sequence — positive states pass through, the floor returns a sentinel-free constant"
  (input  (do
            (effect E (op take (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((take () s (resume (if (> s 0) s 999) (- s 2))))
                (+ (E.take) (+ (E.take) (+ (E.take) (E.take))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1008 Int64))
  (call   main (: 3 Int64)) (output (: 2002 Int64))
  (call   main (: 1 Int64)) (output (: 2998 Int64)))
