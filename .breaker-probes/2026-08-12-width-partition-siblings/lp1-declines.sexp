(case "lp1 sibling: List.at as the Option producer — two at-matches in the arm + computed perform arg used as index"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 7 9)
                ((put (k v) xs
                  (let ((xs2 (match (List.at xs k)
                               ((Some x) (List.push xs x))
                               ((None u) (List.push xs k)))))
                    (resume (match (List.at xs2 2) ((Some y) y) ((None u) -1)) xs2))))
                (S.put (+ n 1) n)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 9 Int64))
  (call   main (: 3 Int64)) (output (: 4 Int64)))
