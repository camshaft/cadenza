; sweep-2 follow-ups: cb12 API fix + cb04/cb11 isolation controls.

(case "cb12b String growth via String.concat in const recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: s String)))
              (if (= (String.byte-len s) 3) (trap "cb12b string grew to three") (f (String.concat s "x"))))
            (def (main) (f ""))
            (export main)))
  (error  CDZ0304 (message "cb12b string grew to three")))

(case "cb13 control: TWO Int64 const params recursion trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: m Int64)) (const (: n Int64)))
              (if (= n 0)
                  (if (= m 7) (trap "cb13 m was seven") (trap "cb13 m other"))
                  (f m (- n 1))))
            (def (main) (f 7 2))
            (export main)))
  (error  CDZ0304 (message "cb13 m was seven")))

(case "cb14 control: NON-recursive Char equality under Ast.encode"
  (input  (do
            (def (f (const (: c Char))) (if (= c #\a) "was-a" "other"))
            (def (run) (= (Ast.encode (Ast.Name (f #\a))) (Ast.encode (Ast.Name "was-a"))))
            (export run)))
  (output (: true Bool)))

(case "cb15 control: helper CALL (no projection) in the recursive argument trap surfaces CDZ0304"
  (input  (do
            (def (dec (const (: n Int64))) (- n 1))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb15 helper-dec reached zero") (f (dec n))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cb15 helper-dec reached zero")))

(case "cb16 control: projection off a LET-BOUND const helper result in recursion trap surfaces CDZ0304"
  (input  (do
            (def (mk (const (: n Int64))) (record (= lo n) (= hi (* n 2))))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb16 let-projected to zero") (let ((r (mk (- n 1)))) (f (. r lo)))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cb16 let-projected to zero")))

(case "cb17 tuple-returning helper destructured in recursion trap surfaces CDZ0304"
  (input  (do
            (def (mk (const (: n Int64))) (tuple (- n 1) (* n 2)))
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "cb17 tuple-fed zero") (match (mk n) ((tuple a b) (f a)))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "cb17 tuple-fed zero")))
