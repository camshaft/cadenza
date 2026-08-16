(case "sab1 a CONDITIONALLY-aborting handle inside the SEED — the abort value or the fall-through both become the outer init"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle B
                     (handle Bail 0
                       ((out (v) t (+ 100 v)))
                       (let ((d (E.next)))
                         (if (> d 0) (do (Bail.out d) 999) (* 5 d))))
                     ((get (u) t (resume t (+ t 1))))
                     (+ (B.get) (* 10 (B.get))))
                   (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1171 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -2 Int64)) (output (: -101 Int64)))
