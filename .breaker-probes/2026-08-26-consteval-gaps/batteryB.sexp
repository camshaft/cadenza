; breaker const-eval sweep 2 — Bytes/BigInt/narrow-int/Char/Symbol/Set/user-sums/mutual-recursion/
; budget/compound-lists/composed-projection/string-building. Recursive taken-trap = CDZ0304 detector;
; Ast.encode = fold-forcer for non-recursive compositions. Expectations = mandated general fold.

(case "cb01 Bytes const-param growth trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: b Bytes)))
              (if (= (Bytes.len b) 3) (trap "cb01 bytes grew to three") (f (Bytes.concat b b"\x01"))))
            (def (main) (f b"\x00"))
            (export main)))
  (error  CDZ0304 (message "cb01 bytes grew to three")))

(case "cb02 BigInt const-param countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: n BigInt)))
              (if (= n 0N) (trap "cb02 bigint reached zero") (f (- n 1N))))
            (def (main) (f 3N))
            (export main)))
  (error  CDZ0304 (message "cb02 bigint reached zero")))

(case "cb03 UInt8 const-param wrapping countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: b UInt8)))
              (if (= b (UInt8.wrap 0)) (trap "cb03 u8 reached zero") (f (UInt8.wrapping-add b (UInt8.wrap 255)))))
            (def (main) (f (UInt8.wrap 2)))
            (export main)))
  (error  CDZ0304 (message "cb03 u8 reached zero")))

(case "cb04 Char const-param equality in const recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: c Char)) (const (: n Int64)))
              (if (= n 0)
                  (if (= c #\a) (trap "cb04 char was a") (trap "cb04 char other"))
                  (f c (- n 1))))
            (def (main) (f #\a 2))
            (export main)))
  (error  CDZ0304 (message "cb04 char was a")))

(case "cb05 Symbol const-param equality in const recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: s Symbol)) (const (: n Int64)))
              (if (= n 0)
                  (if (= s (Symbol.of "hot")) (trap "cb05 symbol hot") (trap "cb05 symbol other"))
                  (f s (- n 1))))
            (def (main) (f (Symbol.of "hot") 2))
            (export main)))
  (error  CDZ0304 (message "cb05 symbol hot")))

(case "cb06 Set.of + Set.contains folds under Ast.encode"
  (input  (do
            (def (f (const (: n Int64)))
              (if (Set.contains (Set.of (list 1 2 3)) n) "in" "out"))
            (def (run) (= (Ast.encode (Ast.Name (f 2))) (Ast.encode (Ast.Name "in"))))
            (export run)))
  (output (: true Bool)))

(case "cb07 user 3-variant sum with payloads folds under Ast.encode"
  (input  (do
            (type Sig (Lo) (Mid Int64) (Hi String))
            (def (f (const (: n Int64)))
              (match (if (= n 0) (Sig.Mid 7) (Sig.Hi "big"))
                ((Sig.Lo) "lo")
                ((Sig.Mid k) (if (= k 7) "mid7" "mid"))
                ((Sig.Hi s) s)))
            (def (run) (= (Ast.encode (Ast.Name (f 0))) (Ast.encode (Ast.Name "mid7"))))
            (export run)))
  (output (: true Bool)))

(case "cb08 MUTUAL recursion between const-param fns trap surfaces CDZ0304"
  (input  (do
            (def (ev (const (: n Int64)))
              (if (= n 0) (trap "cb08 even reached zero") (od (- n 1))))
            (def (od (const (: n Int64)))
              (if (= n 0) 1 (ev (- n 1))))
            (def (main) (ev 4))
            (export main)))
  (error  CDZ0304 (message "cb08 even reached zero")))

(case "cb09 budget edge: 5M-step const countdown trap still surfaces CDZ0304"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb09 five million steps") (f (- n 1))))
            (def (main) (f 5000000))
            (export main)))
  (error  CDZ0304 (message "cb09 five million steps")))

(case "cb10 (List (Tuple Int64 Int64)) const-param growth trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: xs (List (Tuple Int64 Int64)))))
              (if (= (List.len xs) 2) (trap "cb10 tuple list grew") (f (List.prepend xs (tuple 1 2)))))
            (def (main) (f (list (tuple 0 0))))
            (export main)))
  (error  CDZ0304 (message "cb10 tuple list grew")))

(case "cb11 member projection off a const fn call inside const recursion trap surfaces CDZ0304"
  (input  (do
            (def (mk (const (: n Int64))) (record (= lo n) (= hi (* n 2))))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb11 projected to zero") (f (. (mk (- n 1)) lo))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cb11 projected to zero")))

(case "cb12 String growth via String.concat in const recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: s String)))
              (if (= (String.len s) 3) (trap "cb12 string grew to three") (f (String.concat s "x"))))
            (def (main) (f ""))
            (export main)))
  (error  CDZ0304 (message "cb12 string grew to three")))
