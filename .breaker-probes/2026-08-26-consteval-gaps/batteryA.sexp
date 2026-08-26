; breaker probe battery A — const-eval must-fold contexts across the value-shape space.
; Detector 1: a TAKEN trap on the const-executed path must surface as CDZ0304 (fold-proof).
;   If the evaluator declines/never activates, the trap happens at runtime instead -> case mismatch.
; Detector 2: Ast.encode DEMANDS a compile-time constant; a decline is the "runtime AST value" error.
; Expectations below are written for the OPERATOR-MANDATED fully-general fold; every mismatch = a gap.

(case "ca01 CONTROL: Int64 const-param countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "ca01 reached zero") (f (- n 1))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "ca01 reached zero")))

(case "ca02 String const-param recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: s String)))
              (if (= s "stop") (trap "ca02 string reached stop") (f "stop")))
            (def (main) (f "go"))
            (export main)))
  (error  CDZ0304 (message "ca02 string reached stop")))

(case "ca03 Float64 const-param countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: x Float64)))
              (if (= x 0.0) (trap "ca03 float reached zero") (f (- x 1.0))))
            (def (main) (f 3.0))
            (export main)))
  (error  CDZ0304 (message "ca03 float reached zero")))

(case "ca04 (List Int64) const-param growth trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: xs (List Int64))))
              (if (= (List.len xs) 3) (trap "ca04 list grew to three") (f (List.prepend xs 1))))
            (def (main) (f (list 7)))
            (export main)))
  (error  CDZ0304 (message "ca04 list grew to three")))

(case "ca05 (Option Int64) const-param countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (if (= k 0) (trap "ca05 option reached zero") (f (Option.Some (- k 1)))))
                ((Option.None) 0)))
            (def (main) (f (Option.Some 2)))
            (export main)))
  (error  CDZ0304 (message "ca05 option reached zero")))

(case "ca06 tuple const-param countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: t (Tuple Int64 Int64))))
              (match t
                ((tuple a b) (if (= a b) (trap "ca06 tuple fields met") (f (tuple (- a 1) b))))))
            (def (main) (f (tuple 3 1)))
            (export main)))
  (error  CDZ0304 (message "ca06 tuple fields met")))

(case "ca07 record const-param countdown trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: r (Record (: n Int64)))))
              (if (= (. r n) 0) (trap "ca07 record field reached zero") (f (record (= n (- (. r n) 1))))))
            (def (main) (f (record (= n 2))))
            (export main)))
  (error  CDZ0304 (message "ca07 record field reached zero")))

(case "ca08 division in const-param recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 1) (trap "ca08 halving reached one") (f (/ n 2))))
            (def (main) (f 8))
            (export main)))
  (error  CDZ0304 (message "ca08 halving reached one")))

(case "ca09 modulo+bitwise in const-param recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= (% n 7) 0) (trap "ca09 multiple of seven") (f (- (^ n (& n 1)) 1))))
            (def (main) (f 10))
            (export main)))
  (error  CDZ0304 (message "ca09 multiple of seven")))

(case "ca10 String.concat folds under Ast.encode"
  (input  (do
            (def (f (const (: s String))) (String.concat s "x"))
            (def (run) (= (Ast.encode (Ast.Name (f "a"))) (Ast.encode (Ast.Name "ax"))))
            (export run)))
  (output (: true Bool)))

(case "ca11 record build+project folds under Ast.encode"
  (input  (do
            (def (f (const (: n Int64))) (. (record (= a "hi") (= b n)) a))
            (def (run) (= (Ast.encode (Ast.Name (f 1))) (Ast.encode (Ast.Name "hi"))))
            (export run)))
  (output (: true Bool)))

(case "ca12 Map insert+lookup folds under Ast.encode"
  (input  (do
            (def (f (const (: n Int64)))
              (match (Map.lookup (Map.insert (map) n "found") n)
                ((Option.Some s) s)
                ((Option.None) "absent")))
            (def (run) (= (Ast.encode (Ast.Name (f 4))) (Ast.encode (Ast.Name "found"))))
            (export run)))
  (output (: true Bool)))
