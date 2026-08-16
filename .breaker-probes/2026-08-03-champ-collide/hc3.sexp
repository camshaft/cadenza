(case "hc3 removing one key from a THREE-entry collision node equals the directly-built two-key set"
  (input  (do
            (def (main (: z Int64))
              (let ((three (Set.of (list (+ z 0) (+ z 162287980) (+ z 530337572)))))
                (let ((two (Set.remove three 162287981)))
                  (+ (* 100 (if (= two (Set.of (list z (+ z 530337572)))) 1 0))
                     (+ (* 10 (Set.len two))
                        (if (Set.contains two 162287981) 1 0))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 120 Int64)))
