(case "rcy1 a RECYCLING sorter with contamination eviction — an item files into paper or glass by its residue, a CONTAMINANT counts itself and evicts one item from the FULLER bin (paper wins ties, an empty pair evicts nothing), audit packs both bins and the contamination count, and the seed shifts one item's code so a glass item becomes a second contaminant whose eviction hits a different bin than the first"
  (input  (do
            (effect S
              (op item (-> Int64 Int64))
              (op audit (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((item (code) st
                  (match st
                    ((tuple p g c)
                      (if (= (% code 3) 0)
                          (resume (+ (* (+ p 1) 10) 1) (tuple (+ p 1) g c))
                          (if (= (% code 3) 1)
                              (resume (+ (* (+ g 1) 10) 2) (tuple p (+ g 1) c))
                              (if (if (>= p g) (> p 0) false)
                                  (resume (+ (: 900 Int64) (+ c 1)) (tuple (- p 1) g (+ c 1)))
                                  (if (> g 0)
                                      (resume (+ (: 900 Int64) (+ c 1)) (tuple p (- g 1) (+ c 1)))
                                      (resume (+ (: 900 Int64) (+ c 1)) (tuple p g (+ c 1))))))))))
                 (audit () st
                  (match st
                    ((tuple p g c)
                      (resume (+ (* p 100) (+ (* g 10) c)) st)))))
                (let ((a (S.item (: 4 Int64))))
                  (let ((b (S.item (+ (: 5 Int64) (% n 3)))))
                    (let ((c (S.item (: 6 Int64))))
                      (let ((d (S.item (: 8 Int64))))
                        (let ((f (S.audit)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 12011021901111 Int64))
  (call   main (: 0 Int64)) (output (: 12901011902002 Int64)))
