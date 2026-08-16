(case "ms1 a WHOLE nested handle expression as a MATCH's SCRUTINEE — the region builds an Option, the chosen arm draws again"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (match (handle B 0
                         ((g (u) t (resume t t)))
                         (let ((v (+ (B.g) (E.next))))
                           (if (= (% v 2) 0) (Some v) (None))))
                  ((Some x) (+ (* 10 x) (E.next)))
                  ((None) (- (E.next) 7)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 45 Int64))
  (call   main (: 3 Int64)) (output (: -3 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -5 Int64)) (output (: -11 Int64)))
