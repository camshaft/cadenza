(case "ss2 a STRING state GROWS by a parity-picked suffix per dispatch — byte-len pins the concatenation history"
  (input  (do
            (effect E (op grow (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple "" n)
                ((grow () s (match s
                              ((tuple acc k)
                                (resume (String.byte-len acc)
                                        (tuple (String.concat acc (if (= (% k 2) 0) "ab" "xyz"))
                                               (+ k 1)))))))
                (do (E.grow) (E.grow) (E.grow)
                    (+ (* 10 (E.grow)) 3))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 73 Int64))
  (call   main (: 3 Int64)) (output (: 83 Int64))
  (call   main (: 1 Int64)) (output (: 83 Int64)))
