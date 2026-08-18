(case "pyi3 the SHADOWING ARM'S SELF-PERFORM HITS A TOLLED OUTER ARM — the inner arm draws the effect it handles and the draw routes to the outer handler whose thousandfold toll then wraps everything downstream of that dispatch including the inner region's completion, two outer frames stack their tolls around the whole computation, and mispricing either toll or misrouting the self-perform shifts separate digit ranges"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (+ (E.tick)
                   (* 10 (handle E (: 50 Int64)
                           ((tick () s (resume (+ s (E.tick)) (+ s 1))))
                           (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5521 Int64))
  (call   main (: 0 Int64)) (output (: 3510 Int64)))
