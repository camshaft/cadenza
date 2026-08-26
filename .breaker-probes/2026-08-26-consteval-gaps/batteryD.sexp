; breaker const-eval sweep 4 — COMPOSITION DEPTH (the P4-class: constructs that fold alone but
; decline composed). Trap detector = CDZ0304 on fold; Ast.encode = fold-forcer.

(case "cd01 recursive AST leaf-count via Option-threaded indexed walk folds (CDZ0304 detector)"
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a
                ((Ast.List xs) (leaves-of xs 0))
                (_ 1)))
            (def (leaves-of (const (: xs (List Ast))) (const (: i Int64)))
              (match (List.at xs i)
                ((Option.Some c) (+ (leaves c) (leaves-of xs (+ i 1))))
                ((Option.None) 0)))
            (def (main)
              (if (= (leaves (quote (f 1 2))) 3)
                  (trap "cd01 three leaves")
                  (trap "cd01 WRONG leaf count")))
            (export main)))
  (error  CDZ0304 (message "cd01 three leaves")))

(case "cd02 decode-of-encode roundtrip navigated by nested patterns folds (CDZ0304 detector)"
  (input  (do
            (def (second-int (const (: a Ast)))
              (match (Ast.decode (Ast.encode a))
                ((Ast.List xs)
                  (match (List.at xs 1)
                    ((Option.Some (Ast.Int b)) b)
                    (_ (BigInt.of -1))))
                (_ (BigInt.of -2))))
            (def (main)
              (if (= (second-int (quote (g 7))) 7N)
                  (trap "cd02 roundtrip navigated")
                  (trap "cd02 WRONG")))
            (export main)))
  (error  CDZ0304 (message "cd02 roundtrip navigated")))

(case "cd03 function-typed const param applied inside const recursion folds (CDZ0304 detector)"
  (input  (do
            (def (ap (const (: g (-> Int64 Int64))) (const (: n Int64)))
              (if (= n 0) (g 5) (ap g (- n 1))))
            (def (main)
              (if (= (ap (fn (x) (* x 2)) 2) 10)
                  (trap "cd03 lambda applied in fold")
                  (trap "cd03 WRONG")))
            (export main)))
  (error  CDZ0304 (message "cd03 lambda applied in fold")))

(case "cd04 const fn behind an in-scope module folds at the call site (CDZ0304 detector)"
  (input  (do
            (module m
              (def (dec (const (: n Int64))) (- n 1)))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cd04 module-dec reached zero") (f (m.dec n))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cd04 module-dec reached zero")))

(case "cd05 recursive String builder result feeds Ast.encode"
  (input  (do
            (def (rep (const (: n Int64)))
              (if (= n 0) "" (String.concat "x" (rep (- n 1)))))
            (def (run) (= (Ast.encode (Ast.Name (rep 3))) (Ast.encode (Ast.Name "xxx"))))
            (export run)))
  (output (: true Bool)))

(case "cd06 imported library const fn folds at the importing call site (CDZ0304 detector)"
  (module "lib"
    (def (dec (const (: n Int64))) (- n 1))
    (export dec))
  (input  (do
            (import "lib" (dec))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cd06 imported dec reached zero") (f (dec n))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cd06 imported dec reached zero")))
