(case "tl2 a THREE-frame value pipeline — innermost pick feeds middle dbl feeds outermost send, then an outer draw stamps the hundreds"
  (input  (do
            (effect O (op next (-> Int64)) (op send (-> Int64 Int64)))
            (effect M (op dbl (-> Int64 Int64)))
            (effect I (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1)))
                 (send (v) s (resume (+ v s) s)))
                (handle M 0
                  ((dbl (x) m (resume (* 2 x) m)))
                  (handle I 7
                    ((pick () t (resume t t)))
                    (+ (O.send (M.dbl (I.pick))) (* 100 (O.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 519 Int64))
  (call   main (: 0 Int64)) (output (: 14 Int64))
  (call   main (: -20 Int64)) (output (: -2006 Int64)))
