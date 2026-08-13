(case "ivl1 INTERVAL OVERLAP counting — each add answers how many existing intervals the newcomer overlaps (closed-interval test lo<=b and a<=hi) before inserting itself; the seeded interval slides in or out of range"
  (input  (do
            (effect S (op add (-> Int64 Int64 Int64)))
            (def (count-ovl (: ivs (List (Tuple Int64 Int64))) (: i Int64) (: lo Int64) (: hi Int64) (: acc Int64))
              (match (List.at ivs i)
                ((Some p) (match p
                            ((tuple a b)
                              (count-ovl ivs (+ i 1) lo hi
                                         (if (and (<= lo b) (<= a hi)) (+ acc 1) acc)))))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (: (list) (List (Tuple Int64 Int64)))
                ((add (lo hi) ivs
                  (let ((c (count-ovl ivs 0 lo hi 0)))
                    (resume c (List.push ivs (tuple lo hi))))))
                (let ((a (S.add 0 5)))
                  (let ((b (S.add n (+ n 3))))
                    (let ((c (S.add 4 10)))
                      (let ((d (S.add 20 25)))
                        (+ (* 1000 (+ a 1)) (+ (* 100 (+ b 1)) (+ (* 10 (+ c 1)) (+ d 1))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1231 Int64))
  (call   main (: 8 Int64)) (output (: 1131 Int64)))
