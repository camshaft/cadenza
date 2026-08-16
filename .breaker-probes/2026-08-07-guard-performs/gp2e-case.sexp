(case "gp2e TWO pure guards cascade over a draw, the fallback re-performs — guard misses leave dispatch serviceable"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (let ((k (St.next)))
                  (match k
                    ((guard _a (> _a 50)) 111)
                    ((guard _b (> _b 10)) 222)
                    (_o (- 0 (St.next)))))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 111 Int64))
  (call   main (: 20 Int64)) (output (: 222 Int64))
  (call   main (: 3 Int64)) (output (: -6 Int64)))
