(case "hc1 THREE full-hash-colliding keys share one collision node as distinct Set elements"
  (input  (do
            (def (main (: z Int64))
              (let ((s (Set.of (list (+ z 0) (+ z 162287980) (+ z 530337572)))))
                (+ (* 1000 (Set.len s))
                   (+ (* 100 (if (Set.contains s 1) 1 0))
                      (+ (* 10 (if (Set.contains s 162287981) 1 0))
                         (if (Set.contains s 530337573) 1 0))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3111 Int64)))
