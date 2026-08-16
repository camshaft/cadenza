(case "sq1 two SEQUENTIAL handles of the SAME inner effect, each seeded by a fresh outer draw — independent inner threads, one advancing outer thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle B (E.next)
                     ((get (u) t (resume t (+ t 2))))
                     (+ (B.get) (B.get)))
                   (handle B (* 10 (E.next))
                     ((get (u) t (resume t (+ t 3))))
                     (+ (B.get) (B.get))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 135 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: -3 Int64)) (output (: -41 Int64)))
