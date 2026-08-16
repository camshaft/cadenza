(case "or3 a two-site arm over a RECORD state (threshold gates on a projected field)"
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (record (hits 0) (cap n))
                ((feed (v) s
                  (if (< (. s hits) (. s cap))
                    (resume v (record (hits (+ (. s hits) 1)) (cap (. s cap))))
                    (resume -1 s))))
                (+ (* 100 (St.feed 7)) (+ (* 10 (St.feed 8)) (St.feed 9)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 779 Int64)))
