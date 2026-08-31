(example
  (id "expression-interpreter")
  (name "Expression interpreter")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (type Expr (Lit Int64) (Add (Tuple Expr Expr)) (Mul (Tuple Expr Expr)) (Neg Expr))

  (def
    (eval e)
    (match
      e
      ((Lit n) n)
      ((Add p) (+ (eval (. p 0)) (eval (. p 1))))
      ((Mul p) (* (eval (. p 0)) (eval (. p 1))))
      ((Neg x) (Num.neg (eval x)))))

  (def (main) (eval (Mul #tuple((Add #tuple((Lit 2) (Lit 3))) (Neg (Lit 4))))))

  (export main)))
  (expected -20))
