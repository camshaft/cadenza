(case "rp1 a draw-built list REVERSED by a recursive prepend walk — the order-inverting transform preserves the drawn values"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (rev (: xs (List Int64)) (: i Int64) (: acc (List Int64)))
              (match (List.at xs i)
                ((Some v) (rev xs (+ i 1) (List.prepend acc v)))
                ((None) acc)))
            (def (get (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) -999)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (let ((xs (list (E.next) (E.next) (E.next))))
                  (let ((ys (rev xs 0 (list))))
                    (+ (* 100 (get ys 0)) (+ (* 10 (get ys 1)) (get ys 2)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 642 Int64))
  (call   main (: 0 Int64)) (output (: 420 Int64))
  (call   main (: -3 Int64)) (output (: 87 Int64)))
