; cd02 corrected: Ast.decode returns a Result; navigate through Ok then the Ast shapes.

(case "cd02c decode-of-encode roundtrip navigated through Result+Option+Ast matches folds (CDZ0304 detector)"
  (input  (do
            (def (second-int (const (: a Ast)))
              (match (Ast.decode (Ast.encode a))
                ((Ok d)
                  (match d
                    ((Ast.List xs)
                      (match (List.at xs 1)
                        ((Option.Some c)
                          (match c
                            ((Ast.Int b) b)
                            (_ (BigInt.of -1))))
                        ((Option.None) (BigInt.of -1))))
                    (_ (BigInt.of -2))))
                ((Err _) (BigInt.of -3))))
            (def (main)
              (if (= (second-int (quote (g 7))) 7N)
                  (trap "cd02c roundtrip navigated")
                  (trap "cd02c WRONG")))
            (export main)))
  (error  CDZ0304 (message "cd02c roundtrip navigated")))

(case "cd02v decode-of-encode roundtrip navigation VALUE form — 7"
  (input  (do
            (def (second-int (const (: a Ast)))
              (match (Ast.decode (Ast.encode a))
                ((Ok d)
                  (match d
                    ((Ast.List xs)
                      (match (List.at xs 1)
                        ((Option.Some c)
                          (match c
                            ((Ast.Int b) b)
                            (_ (BigInt.of -1))))
                        ((Option.None) (BigInt.of -1))))
                    (_ (BigInt.of -2))))
                ((Err _) (BigInt.of -3))))
            (def (main) (second-int (quote (g 7))))
            (export main)))
  (output (: 7 BigInt)))
