; D follow-ups: value-form discriminators (decline vs mis-fold) + syntax fixes.

(case "cd01v recursive AST leaf-count VALUE form — 3 leaves"
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a
                ((Ast.List xs) (leaves-of xs 0))
                (_ 1)))
            (def (leaves-of (const (: xs (List Ast))) (const (: i Int64)))
              (match (List.at xs i)
                ((Option.Some c) (+ (leaves c) (leaves-of xs (+ i 1))))
                ((Option.None) 0)))
            (def (main) (leaves (quote (f 1 2))))
            (export main)))
  (output (: 3 Int64)))

(case "cd03v function-typed const param applied in recursion VALUE form — 10"
  (input  (do
            (def (ap (const (: g (-> Int64 Int64))) (const (: n Int64)))
              (if (= n 0) (g 5) (ap g (- n 1))))
            (def (main) (ap (fn (x) (* x 2)) 2))
            (export main)))
  (output (: 10 Int64)))

(case "cd02b decode-of-encode roundtrip navigated by SPLIT matches folds (CDZ0304 detector)"
  (input  (do
            (def (second-int (const (: a Ast)))
              (match (Ast.decode (Ast.encode a))
                ((Ast.List xs)
                  (match (List.at xs 1)
                    ((Option.Some c)
                      (match c
                        ((Ast.Int b) b)
                        (_ (BigInt.of -1))))
                    ((Option.None) (BigInt.of -1))))
                (_ (BigInt.of -2))))
            (def (main)
              (if (= (second-int (quote (g 7))) 7N)
                  (trap "cd02b roundtrip navigated")
                  (trap "cd02b WRONG")))
            (export main)))
  (error  CDZ0304 (message "cd02b roundtrip navigated")))

(case "cd06b imported library const fn folds at the importing call site (CDZ0304 detector)"
  (module "lib"
    (do
      (def (dec (const (: n Int64))) (- n 1))
      (export dec)))
  (input  (do
            (import "lib" (dec))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cd06b imported dec reached zero") (f (dec n))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cd06b imported dec reached zero")))
