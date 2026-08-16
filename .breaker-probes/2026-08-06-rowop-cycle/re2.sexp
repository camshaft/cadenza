(case "re2 a FIELD-COUNT-changing chain (2->4->1->3) reads every generation's own shape"
  (input  (do
            (def (main (: n Int64))
              (do
                (def g0 (record (a n) (b 2)))
                (def g1 (Record.extend (Record.extend g0 #"c" 3) #"d" 4))
                (def g2 (Record.without (Record.without (Record.without g1 (a)) (b)) (c)))
                (def g3 (Record.extend (Record.extend g2 #"x" 7) #"y" 8))
                (+ (* 1000 (. g0 a))
                   (+ (* 100 (. g1 d))
                      (+ (* 10 (. g2 d))
                         (. g3 x))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5447 Int64)))
