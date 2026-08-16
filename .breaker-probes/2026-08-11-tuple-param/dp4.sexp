(case "dp4 pushing through ONE alias of a doubly-held list leaves the sibling field intact"
  (input  (do
            (type Pair (Mk (List Int64) (List Int64)))
            (def (consume-left (: p Pair))
              (match p ((Mk a _b) (List.push a 99))))
            (def (main (: k Int64))
              (let ((xs (list k)))
                (let ((p (Mk xs xs)))
                  (let ((grown (consume-left p)))
                    (+ (List.len grown)
                       (+ (* 10 (match p ((Mk _a b) (List.len b))))
                          (* 100 (match (List.at grown 1) ((Some v) v) ((None _u) -1)))))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 9912 Int64)))
