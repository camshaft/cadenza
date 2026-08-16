(case "ms3 parity-sum dispatch inside a RECURSIVE walk — each level draws a Mode and accumulates by variant as the thread advances"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)))
            (def (walk (: k Int64))
              (if (<= k 0)
                  0
                  (+ (match (E.mode)
                       ((A) 7)
                       ((B x) x)
                       ((C x y) (* x y)))
                     (walk (- k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1))))
                (walk 4)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 19 Int64))
  (call   main (: 1 Int64)) (output (: 16 Int64)))
