(case "sr1 a performing self-recursive walk's result SEEDS an inner handle, then a trailing outer draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0) (* 100 d) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle B (walk 0)
                     ((get (u) t (resume t (+ t 1))))
                     (+ (B.get) (* 100 (B.get))))
                   (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 70808 Int64))
  (call   main (: 12 Int64)) (output (: 141515 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64))
  (call   main (: -13 Int64)) (output (: -70606 Int64)))
