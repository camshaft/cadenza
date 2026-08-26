; cb11/cb16/cb17 discriminator: helper-returned compound vs INLINE compound inside const recursion.

(case "cb18 INLINE record projection in recursive argument trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb18 inline record zero") (f (. (record (= lo (- n 1)) (= hi 9)) lo))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cb18 inline record zero")))

(case "cb19 INLINE tuple destructure in recursive body trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb19 inline tuple zero") (match (tuple (- n 1) 9) ((tuple a b) (f a)))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cb19 inline tuple zero")))

(case "cb20 helper returning record consumed OUTSIDE recursion folds under Ast.encode"
  (input  (do
            (def (mk (const (: n Int64))) (record (= lo n) (= hi (* n 2))))
            (def (g (const (: k Int64)))
              (if (= k 0) 0 (g (- k 1))))
            (def (run) (= (Ast.encode (Ast.Int (BigInt.of (+ (g 3) (. (mk 5) lo)))))
                          (Ast.encode (Ast.Int (BigInt.of 5)))))
            (export run)))
  (output (: true Bool)))
