(case "mx1 a mutual pair where only ONE partner performs — the non-performing leg still threads through the group fold"
  (input  (do
            (effect S (op tick (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (pa (: n Int64)) (if (= n 0) (S.tick) (pb n)))
            (def (pb (: n Int64))
              (let ((child (pa (- n 1))))
                (+ child (* 2 n))))
            (def (main (: k Int64))
              (handle S 0
                ((tick (u) s (resume s s))
                 (put (v) s (resume unit (+ s v))))
                (pa k)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 12 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
