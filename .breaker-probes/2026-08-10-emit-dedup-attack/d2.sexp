(case "d2 three None-producing built-in reads in one expression — List.at in and out of range plus Map.lookup hit and miss"
  (input  (do
            (def (opt-score (: o (Option Int64)))
              (match o
                ((Some v) v)
                (None -7)))
            (def (main (: n Int64))
              (let ((xs (list 10 20 30))
                    (m (Map.insert (Map.insert (Map.empty) 1 100) 2 200)))
                (+ (* 1000000 (opt-score (List.at xs n)))
                   (+ (* 1000 (opt-score (Map.lookup m n)))
                      (opt-score (List.at xs (+ n 10)))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 20099993 Int64))
  (call   main (: 5 Int64)) (output (: -7007007 Int64))
  (call   main (: 2 Int64)) (output (: 30199993 Int64)))
