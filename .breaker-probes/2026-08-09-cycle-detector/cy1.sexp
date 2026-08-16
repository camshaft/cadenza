(case "cy1 a CYCLE detector — a bitmask accumulator of seen residues stops the walk on the first repeat"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: seen Int64) (: steps Int64))
              (if (>= steps 6)
                  (+ (* 100 steps) seen)
                  (let ((d (% (E.next) 4)))
                    (if (= (& seen (<< 1 d)) 0)
                        (walk (| seen (<< 1 d)) (+ steps 1))
                        (+ (* 100 (+ steps 1)) seen)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (walk 0 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 305 Int64))
  (call   main (: 1 Int64)) (output (: 310 Int64))
  (call   main (: 2 Int64)) (output (: 305 Int64)))
