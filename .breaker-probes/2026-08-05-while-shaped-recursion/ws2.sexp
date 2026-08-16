(case "ws2 perform-terminated loop, condition-position spelling (no def-in-do)"
  (input  (do
            (effect St (op draw (-> Unit Int64)) (op last (-> Unit Int64)))
            (def (drain (: acc Int64))
              (if (= (St.draw) 0) acc (drain (+ acc (St.last)))))
            (def (main (: n Int64))
              (handle St (tuple n 0)
                ((draw (u) s (resume (. s 0) (tuple (- (. s 0) 1) (. s 0))))
                 (last (u) s (resume (. s 1) s)))
                (drain 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 6 Int64)))
