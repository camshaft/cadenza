(example
  (id "rpn-calculator")
  (name "RPN calculator (stack machine)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (type Stack (Empty unit) (Cons (Tuple Int64 Stack)))

  (def (push s x) (Cons #tuple(x s)))

  (type Tok (Num Int64) (Plus unit) (Times unit))

  (def
    (apply2 s f)
    (match
      s
      ((Cons top1)
        (match
          (. top1 1)
          ((Cons top2) (push (. top2 1) (f (. top2 0) (. top1 0))))
          ((Empty _) (trap "rpn: operator underflow (need two operands)"))))
      ((Empty _) (trap "rpn: operator underflow (empty stack)"))))

  (def
    (step s tok)
    (match tok ((Num n) (push s n)) ((Plus _) (apply2 s +)) ((Times _) (apply2 s *))))

  (def
    (run toks i n s)
    (if
      (= i n)
      s
      (match ((. List at) toks i) ((Some t) (run toks (+ i 1) n (step s t))) ((None) s))))

  (def (top s) (match s ((Cons t) (. t 0)) ((Empty _) (trap "rpn: empty result stack"))))

  (def
    (main)
    (let
      ((toks #list((Num 3) (Num 4) (Plus unit) (Num 5) (Times unit))))
      (top (run toks 0 ((. List len) toks) (Empty unit)))))

  (export main)))
  (expected 35))
